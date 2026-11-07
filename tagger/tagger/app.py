import hmac
import logging
import multiprocessing
import os
import threading
from collections.abc import Callable
from typing import Annotated, Literal
from concurrent.futures import Executor, ProcessPoolExecutor
from concurrent.futures.process import BrokenProcessPool
from pathlib import Path

import numpy as np
from fastapi import Depends, FastAPI, Header, HTTPException
from pydantic import BaseModel, Field, StringConstraints, model_validator

from . import moods, scry, tune, tune_vocab
from .embed import Embedder, EmbeddingCache, MuqEmbedder, decode_track
from .reference import ReferenceSet, load_reference, reference_stats
from .tune_phrases import PHRASES

MAX_EMBED_ITEMS = 32

DEFAULT_RESULTS = 50
MAX_RESULTS = 500
MAP_CACHE_SIZE = 8

# Limits on the mood vocabulary a /score request may carry.
MAX_MOODS = 32
MAX_PROMPTS_PER_GROUP = 16
MAX_PROMPT_CHARS = 300
# Embedded prompt texts kept between requests. Past this the cache starts
# over; it only grows when the user keeps editing prompts. /tune alone brings ~1000 texts
# (its units, with the built-in vocabulary), so the room is twice that and more.
MAX_CACHED_PROMPTS = 4096

# Limits on what a /tune request may carry.
MAX_TUNE_FLAGS = 500
MAX_TUNE_BANK = 1000
MAX_TUNE_VERSIONS = 8     # versions a compare-mode request may carry
# The built-in vocabulary (tune_vocab) is large; only its this many best words alone, by
# the search's own loss, join the search, which keeps /tune within a few seconds.
VOCAB_KEEP = 120

# How long /embed waits for the model to finish loading before it answers
# 503. Loading starts when the app is created.
LOAD_WAIT_SECONDS = 10

log = logging.getLogger(__name__)


class EmbedItem(BaseModel):
    hash: str
    path: str


class EmbedRequest(BaseModel):
    items: list[EmbedItem] = Field(max_length=MAX_EMBED_ITEMS)


PromptText = Annotated[str, StringConstraints(strip_whitespace=True, min_length=1, max_length=MAX_PROMPT_CHARS)]
PromptList = Annotated[list[PromptText], Field(min_length=1, max_length=MAX_PROMPTS_PER_GROUP)]
MoodKey = Annotated[str, StringConstraints(pattern=r"^[a-z]+$", max_length=32)]


class EnergyPrompts(BaseModel):
    positive: PromptList
    negative: PromptList


class ScoreRequest(BaseModel):
    # Fields an older app still sends (scene, gate, tags, pace: the auto-tagging
    # heads, since removed) are ignored: pydantic drops unknown fields by default.
    targets: list[str] = []
    # The app's mood vocabulary: prompts per mood id, and Energy's two sides.
    # Either one, when given, replaces the matching defaults in `moods.py`.
    mood_prompts: Annotated[dict[MoodKey, PromptList], Field(max_length=MAX_MOODS)] | None = None
    energy_prompts: EnergyPrompts | None = None


class PreviewRequest(BaseModel):
    prompts: PromptList
    targets: list[str] = []


class SimilarRequest(BaseModel):
    query: list[str] = []
    candidates: list[str] = []
    limit: int = DEFAULT_RESULTS


class SearchRequest(BaseModel):
    text: str
    candidates: list[str] = []
    limit: int = DEFAULT_RESULTS


class MapRequest(BaseModel):
    hashes: list[str] = []


class TuneFlag(BaseModel):
    hash: str
    min: float | None = None
    max: float | None = None
    weight: float = 1.0
    fresh: bool = True
    side: Literal["high", "low", "mid"]
    # The flag the user is tuning around (at most one): it counts tune.FOCUS_WEIGHT times.
    focus: bool = False


class TuneCalibration(BaseModel):
    source: Literal["library", "reference"]
    midpoint_z: float
    slope: float


class OtherMood(BaseModel):
    name: str
    lines: PromptList


class TuneRequest(BaseModel):
    mood_id: str
    name: str
    current_lines: PromptList
    flags: Annotated[list[TuneFlag], Field(max_length=MAX_TUNE_FLAGS)]
    targets: list[str] = []
    bank: Annotated[list[str], Field(max_length=MAX_TUNE_BANK)] = []
    # Which of `bank` the user typed ("Extra words to try"), so their lines say so.
    extra_words: Annotated[list[str], Field(max_length=MAX_TUNE_BANK)] = []
    # Each other mood's name and lines; app versions before line sources sent the lines alone.
    other_moods: Annotated[dict[MoodKey, OtherMood | PromptList], Field(max_length=MAX_MOODS)] = {}
    # Words to leave out: no unit (or blend) holding one as a whole word is used.
    exclude: Annotated[list[str], Field(max_length=MAX_TUNE_BANK)] = []
    calibration: TuneCalibration
    # compare: review each of `candidates` (the user's own versions of the lines) as
    #   written; no search.
    # arrange: search only the user's typed words and lines (`extra_words`) and the
    #   current lines, split into words and kept whole.
    # suggest: search the bank, other moods' words, PHRASES and the built-in vocabulary too.
    mode: Literal["compare", "arrange", "suggest"] = "suggest"
    candidates: Annotated[list[PromptList], Field(max_length=MAX_TUNE_VERSIONS)] = []

    @model_validator(mode="after")
    def _compare_needs_versions(self):
        if self.mode == "compare" and not self.candidates:
            raise ValueError("compare mode needs at least one version in candidates")
        return self

    def other(self) -> dict[str, OtherMood]:
        """`other_moods` with a name for each (its id when the app sent lines alone)."""
        return {k: v if isinstance(v, OtherMood) else OtherMood(name=k, lines=v)
                for k, v in self.other_moods.items()}


def _limit(n: int) -> int:
    return min(max(n, 0), MAX_RESULTS)


def _decode(path: str) -> np.ndarray:
    # Module-level so a process pool can pickle it by reference.
    return decode_track(path)


def _failure(hash: str, e: Exception) -> dict:
    return {"hash": hash, "ok": False, "error": str(e) or type(e).__name__}


def create_app(
    embedder: Embedder,
    cache: EmbeddingCache,
    token: str,
    executor_factory: Callable[[], Executor] | None = None,
    reference: ReferenceSet | None = None,
) -> FastAPI:
    # Load the model in the background so /health answers while the weights
    # download on first start. `loaded` is set once loading has finished,
    # whether it worked or not; `load_error` holds the failure, if any.
    loaded = threading.Event()
    load_error: list[str] = []

    def load_model() -> None:
        try:
            embedder.load()
        except Exception as e:
            log.exception("Model failed to load")
            load_error.append(str(e) or type(e).__name__)
        finally:
            loaded.set()

    threading.Thread(target=load_model, daemon=True).start()

    # Workers are spawned, not forked: the parent has CUDA and threads loaded,
    # which a forked child inherits in a broken state.
    make_pool = executor_factory or (
        lambda: ProcessPoolExecutor(os.cpu_count(), mp_context=multiprocessing.get_context("spawn"))
    )
    pool_lock = threading.Lock()
    current = [make_pool()]

    def submit(path: str):
        with pool_lock:
            pool = current[0]
            return pool, pool.submit(_decode, path)

    def replace_pool(broken: Executor) -> None:
        """Swap in a fresh pool once a worker has died; a no-op if another
        request already did."""
        with pool_lock:
            if current[0] is broken:
                broken.shutdown(wait=False, cancel_futures=True)
                current[0] = make_pool()

    def require_token(x_lutedrop_token: str = Header(default="")) -> None:
        ok = hmac.compare_digest(x_lutedrop_token.encode(), token.encode())
        if not token or not ok:
            raise HTTPException(status_code=401, detail="unauthorized")

    app = FastAPI(dependencies=[Depends(require_token)])

    @app.get("/health")
    def health():
        return {
            "model": embedder.name,
            "cache_count": cache.count(),
            "ready": loaded.is_set() and not load_error,
            "error": load_error[0] if load_error else None,
            "reference": {"set": reference.set_id, "tracks": reference.tracks} if reference else None,
        }

    @app.post("/embed")
    def embed(req: EmbedRequest):
        if not loaded.wait(LOAD_WAIT_SECONDS):
            raise HTTPException(status_code=503, detail="model is still loading")
        if load_error:
            raise HTTPException(status_code=503, detail=f"model failed to load: {load_error[0]}")
        results: dict[str, dict] = {}
        pending = {}
        for item in req.items:
            if cache.has(item.hash):
                results[item.hash] = {"hash": item.hash, "ok": True}
            elif item.hash not in pending:
                try:
                    pending[item.hash] = submit(item.path)
                except BrokenProcessPool as e:
                    replace_pool(current[0])
                    results[item.hash] = _failure(item.hash, e)
        for hash, (pool, future) in pending.items():
            try:
                windows = future.result()
                if not cache.has(hash):
                    cache.put(hash, embedder.embed_windows(windows))
                results[hash] = {"hash": hash, "ok": True}
            except BrokenProcessPool as e:
                replace_pool(pool)
                results[hash] = _failure(hash, e)
            except Exception as e:
                results[hash] = _failure(hash, e)
        return {"results": [results[i.hash] for i in req.items]}

    def vectors(hashes: list[str]) -> tuple[list[str], np.ndarray]:
        """Mean embeddings for the hashes that are cached, in input order."""
        found, rows = [], []
        for h in dict.fromkeys(hashes):
            v = cache.get(h)
            if v is not None:
                found.append(h)
                rows.append(v)
        return found, (np.vstack(rows) if rows else np.empty((0, 0)))

    # Unit-length vectors per prompt text, embedded the first time /score
    # needs a text with the model loaded and reused after that, so changing
    # one mood's prompts embeds only the new texts. A batch is added only once
    # it embedded in full, so a failure part-way leaves nothing half-filled.
    text_vecs: dict[str, np.ndarray] = {}
    text_lock = threading.Lock()

    def prompt_vectors(groups: dict[str, list[str]]) -> dict[str, np.ndarray] | None:
        """One matrix of unit prompt vectors per group, or None while the
        model is not ready (or the prompts could not be embedded); /score
        then omits mood. Never waits for the model."""
        if not loaded.is_set() or load_error:
            return None
        with text_lock:
            needed = list(dict.fromkeys(t for texts in groups.values() for t in texts))
            missing = [t for t in needed if t not in text_vecs]
            if len(text_vecs) + len(missing) > MAX_CACHED_PROMPTS:
                # Start over, embedding every text this call needs, so the
                # lookup below never misses a text cleared here.
                text_vecs.clear()
                missing = needed
            if missing:
                try:
                    vecs = np.asarray(embedder.embed_text(missing), dtype=np.float64)
                except Exception:
                    log.exception("Mood prompts failed to embed")
                    return None
                norms = np.linalg.norm(vecs, axis=1, keepdims=True)
                vecs = vecs / np.where(norms == 0, 1, norms)
                text_vecs.update(zip(missing, vecs))
            return {name: np.vstack([text_vecs[t] for t in texts]) for name, texts in groups.items()}

    def mood_scores(
        req: ScoreRequest, target_hashes: list[str], X_targets: np.ndarray
    ) -> tuple[dict[str, dict], dict | None] | None:
        """Raw mood scores per target, plus the reference set's stats for the
        same prompts (None without a reference set). None overall while the
        prompts cannot be embedded."""
        if not target_hashes:
            return {}, None
        mood_groups = req.mood_prompts if req.mood_prompts is not None else moods.MOODS
        if req.energy_prompts is not None:
            energy_pos, energy_neg = req.energy_prompts.positive, req.energy_prompts.negative
        else:
            energy_pos, energy_neg = moods.ENERGY_POS, moods.ENERGY_NEG
        # "energy+" / "energy-" cannot clash with a mood id (lowercase letters only).
        vecs = prompt_vectors({**mood_groups, "energy+": energy_pos, "energy-": energy_neg})
        if vecs is None:
            return None
        # Raw scores; the app calibrates them. Each mood is the mean cosine
        # with its prompts; energy is positive minus negative.
        sims = {name: (X_targets @ v.T).mean(axis=1) for name, v in vecs.items()}
        energy = sims["energy+"] - sims["energy-"]
        scores = {
            h: {
                "moods": {name: float(sims[name][i]) for name in mood_groups},
                "energy": float(energy[i]),
            }
            for i, h in enumerate(target_hashes)
        }
        stats = reference_stats(reference, vecs, list(mood_groups)) if reference else None
        return scores, stats

    @app.post("/score")
    def score(req: ScoreRequest):
        target_hashes, X_targets = vectors(req.targets)
        out = {"model": embedder.name}
        mood = mood_scores(req, target_hashes, X_targets)
        if mood is not None:
            out["mood"], mood_reference = mood
            if mood_reference is not None:
                out["mood_reference"] = mood_reference
        return out

    @app.post("/preview")
    def preview(req: PreviewRequest):
        """Raw similarity of each cached target with one set of prompts (the
        mean cosine, as a mood is scored in /score), keyed by hash. No
        calibration: for eyeballing which tracks a prompt set ranks first
        before saving it. Waits for the model like /search."""
        target_hashes, X_targets = vectors(req.targets)
        if not target_hashes:
            return {}
        if not loaded.wait(LOAD_WAIT_SECONDS):
            raise HTTPException(status_code=503, detail="model is still loading")
        if load_error:
            raise HTTPException(status_code=503, detail=f"model failed to load: {load_error[0]}")
        vecs = prompt_vectors({"preview": req.prompts})
        if vecs is None:
            raise HTTPException(status_code=503, detail="the prompts could not be embedded")
        sims = (X_targets @ vecs["preview"].T).mean(axis=1)
        return {h: float(sims[i]) for i, h in enumerate(target_hashes)}

    @app.post("/similar")
    def similar(req: SimilarRequest):
        _, Q = vectors(req.query)
        excluded = set(req.query)
        c_hashes, C = vectors([h for h in req.candidates if h not in excluded])
        if len(Q) == 0 or len(C) == 0:
            return {"results": []}
        ranked = scry.similar(scry.centroid(Q), C, _limit(req.limit))
        return {"results": [{"hash": c_hashes[i], "score": s} for i, s in ranked]}

    @app.post("/search")
    def search(req: SearchRequest):
        c_hashes, C = vectors(req.candidates)
        if not req.text.strip() or not c_hashes:
            return {"results": []}
        if not loaded.wait(LOAD_WAIT_SECONDS):
            raise HTTPException(status_code=503, detail="model is still loading")
        if load_error:
            raise HTTPException(status_code=503, detail=f"model failed to load: {load_error[0]}")
        text_vec = embedder.embed_text([req.text])[0]
        ranked = scry.rank_by_text(text_vec, C, _limit(req.limit))
        return {"results": [{"hash": c_hashes[i], "score": s} for i, s in ranked]}

    @app.post("/tune")
    def tune_mood(req: TuneRequest):
        """Search for prompt lines that fit a mood's flagged songs, and report on the
        best ones against the current lines. Bars are read with the calibration the
        app sent. Waits for the model like /preview."""
        if req.calibration.source == "reference" and reference is None:
            raise HTTPException(status_code=409, detail="The analyzer has no reference set")
        hashes, X = vectors([*req.targets, *(f.hash for f in req.flags)])
        if not hashes:
            raise HTTPException(status_code=409, detail="None of the targets has a cached embedding")
        if not loaded.wait(LOAD_WAIT_SECONDS):
            raise HTTPException(status_code=503, detail="model is still loading")
        if load_error:
            raise HTTPException(status_code=503, detail=f"model failed to load: {load_error[0]}")
        X = X.astype(np.float64)
        row = {h: i for i, h in enumerate(hashes)}
        skipped = [f.hash for f in req.flags if f.hash not in row]
        kept = [f for f in req.flags if f.hash in row]
        flags = [tune.FlagTarget(row[f.hash], f.min, f.max, f.weight, f.fresh, f.side, f.focus)
                 for f in kept]
        flag_hashes = [f.hash for f in kept]
        m = tune.Mapping(req.calibration.midpoint_z, req.calibration.slope, req.calibration.source)
        use_ref = req.calibration.source == "reference"

        # Units: the words and lines in play, each with where it came from, less any
        # holding a word the user left out. Arrange mode takes only the user's typed
        # words and lines and the current lines; suggest mode adds the bank, other
        # moods' words, the mood's name, the shipped phrases and the built-in vocabulary.
        # (Compare mode searches nothing; its units only serve the reports.)
        name = req.name.strip().lower()
        others = req.other()
        suggest = req.mode == "suggest"
        pool = tune.UnitPool()
        if suggest:
            typed = {w.strip().lower() for w in req.extra_words}
            pool.add_parts("word bank", [t for t in req.bank if t.strip().lower() not in typed])
            pool.add_parts("your extra words", req.extra_words)
        else:
            # Arrange: the typed words, and each typed line whole, as written.
            pool.add_parts("your words", req.extra_words)
        pool.add_parts("current lines", req.current_lines)
        if suggest:
            for o in others.values():
                pool.add_parts(f"{o.name} words", o.lines)
            pool.add_parts("mood name", [name])
        # Each current line as written, so the search can start from (and keep) them whole.
        pool.add_whole("current lines", req.current_lines)
        if req.mode == "arrange":
            pool.add_whole("your words", req.extra_words)
        if suggest:
            pool.add_whole("phrase list", PHRASES, only_new=True, lower=True)
            pool.add_whole("built-in vocabulary", tune_vocab.VOCAB, only_new=True, lower=True)
        pool.drop(lambda u: tune.excluded(u, req.exclude))
        sources = pool.sources
        units = pool.units
        vocab = {u for u, labels in sources.items() if labels == ["built-in vocabulary"]}

        groups = {"units": units, "current": req.current_lines}
        groups.update({f"other:{k}": o.lines for k, o in others.items()})
        groups.update({f"version:{i}": lines for i, lines in enumerate(req.candidates)})
        vecs = prompt_vectors(groups)
        if vecs is None:
            raise HTTPException(status_code=503, detail="the prompts could not be embedded")
        U_lib = vecs["units"] @ X.T
        U_ref = vecs["units"] @ reference.vectors.T if use_ref else None

        def raws(V: np.ndarray) -> tuple[np.ndarray, np.ndarray | None]:
            # The mean cosine with the lines, as /score computes a mood.
            return (V @ X.T).mean(axis=0), (V @ reference.vectors.T).mean(axis=0) if use_ref else None

        before = raws(vecs["current"])
        # The current lines' bars: the search pays for moving unflagged songs away from them.
        b_before = tune.bars(before[0], before[0] if before[1] is None else before[1], m)
        other_raw = {k: raws(vecs[f"other:{k}"])[0] for k in others}

        # Only the built-in vocabulary's best words alone stay in the search.
        vocab_rows = [i for i, u in enumerate(units) if u in vocab]
        if len(vocab_rows) > VOCAB_KEEP:
            best = set(tune.best_rows(U_lib, U_ref, vocab_rows, flags, m, VOCAB_KEEP, before=b_before))
            dropped = set(vocab_rows) - best
            keep = [i for i in range(len(units)) if i not in dropped]
            units = [units[i] for i in keep]
            U_lib = U_lib[keep]
            U_ref = None if U_ref is None else U_ref[keep]
        row_of = {u: i for i, u in enumerate(units)}
        current_rows = tuple(row_of[line] for line in dict.fromkeys(req.current_lines) if line in row_of)
        candidates = [u for u in tune.split_units(req.bank if suggest else req.extra_words)
                      if " " not in u and u in row_of]
        candidates += [name] if name in row_of else []

        loo_memo: dict = {}

        def report(lines, after, sub_flags, sub_hashes, proposed=True, names_from=candidates):
            # Leave-one-out and name suggestions only make sense for a proposal.
            loo, names = (0, 0), []
            if proposed:
                loo = tune.leave_one_out(U_lib, U_ref, units, sub_flags, m, tune.Combo(tuple(lines), 0.0),
                                         seeds=[current_rows], memo=loo_memo, before=b_before)
                names = tune.name_suggestions(U_lib, units, names_from, after[0])
            return tune.build_report(
                lines=lines, after=after, before=before, flags=sub_flags, flag_hashes=sub_hashes,
                hashes=hashes, m=m, other_raw=other_raw, loo=loo, names=names, sources=sources,
            )

        baseline = report(req.current_lines, before, flags, flag_hashes, proposed=False)
        proposals, split = [], None
        if req.mode == "compare":
            # Each version as written, in order, whether or not it beats the current lines.
            for i, lines in enumerate(req.candidates):
                for line in lines:
                    sources[line] = ["your version"]
                proposals.append(report(list(lines), raws(vecs[f"version:{i}"]), flags, flag_hashes,
                                        proposed=False))
        elif flags and units:
            # Blends of the best single units join the pool as extra units. A blend's
            # sources are "blend" and then its members'.
            groups_ = tune.blend_groups(U_lib, U_ref, units, flags, m, before=b_before)
            blends = [text for text, _ in groups_]
            for text, members in groups_:
                sources[text] = list(dict.fromkeys(["blend", *(s for i in members for s in sources.get(units[i], []))]))
            if blends:
                blend_vecs = prompt_vectors({"blends": blends})
                if blend_vecs is None:
                    raise HTTPException(status_code=503, detail="the prompts could not be embedded")
                U_lib = np.vstack([U_lib, blend_vecs["blends"] @ X.T])
                if use_ref:
                    U_ref = np.vstack([U_ref, blend_vecs["blends"] @ reference.vectors.T])
                units = units + blends
            found = tune.beam_search(U_lib, U_ref, units, flags, m, seeds=[current_rows], before=b_before)
            current_set = sorted(dict.fromkeys(req.current_lines))
            for c in found:
                # Strictly better than the current lines (a hair of float noise is not
                # better), and never the current lines again in another order.
                if c.loss < baseline["loss"] - 1e-9 and sorted(c.lines) != current_set:
                    after = tune.combo_raws(U_lib, U_ref, units, c.lines)
                    proposals.append(report(list(c.lines), after, flags, flag_hashes))
            # A split only answers flags the dialog shows broken: the best proposal's,
            # or the current lines' when nothing beats them.
            shown = proposals[0] if proposals else baseline
            if any(f["fresh"] and not f["holds"] for f in shown["flags"]):
                offer = tune.try_split(U_lib, U_ref, units, flags, m, X, found[0], before=b_before)
                if offer is not None:
                    split = _split_report(offer, flags, flag_hashes, report, U_lib, U_ref, units,
                                          candidates, name)
        return {"baseline": baseline, "proposals": proposals, "split": split, "skipped": skipped}

    def _split_report(offer, flags, flag_hashes, report, U_lib, U_ref, units, candidates, name):
        rest = [p for p, f in enumerate(flags) if f.side != "high"]
        halves = []
        for combo, group in ((offer["a"], offer["a_flags"]), (offer["b"], offer["b_flags"])):
            after = tune.combo_raws(U_lib, U_ref, units, combo.lines)
            ranking = [w for w, _ in tune.name_suggestions(U_lib, units, candidates, after[0], k=len(candidates))]
            # The half whose best-matching words reach the mood's name first keeps it.
            rank = ranking.index(name) if name in ranking else len(ranking)
            halves.append(((rank, -len(group)), combo, group, after))
        keep, new = sorted(halves, key=lambda h: h[0])

        def half_report(half, names_from):
            _, combo, group, after = half
            sub = group + rest
            return report(list(combo.lines), after, [flags[p] for p in sub], [flag_hashes[p] for p in sub],
                          names_from=names_from)

        return {
            # The new mood needs a name of its own, so the current one is not offered.
            "keep": half_report(keep, candidates),
            "new": half_report(new, [w for w in candidates if w != name]),
            "keep_hashes": [flag_hashes[p] for p in keep[2]],
            "new_hashes": [flag_hashes[p] for p in new[2]],
            "rho": offer["rho"],
        }

    # Layouts keyed by the sorted hash tuple; the oldest entry is dropped first.
    map_cache: dict[tuple[str, ...], list[dict]] = {}
    map_lock = threading.Lock()

    @app.post("/map")
    def map_points(req: MapRequest):
        found, X = vectors(req.hashes)
        key = tuple(sorted(found))
        with map_lock:
            points = map_cache.get(key)
        if points is None:
            row = {h: i for i, h in enumerate(found)}
            coords = scry.layout_2d(X[[row[h] for h in key]] if key else X)
            points = [
                {"hash": h, "x": float(x), "y": float(y)} for h, (x, y) in zip(key, coords)
            ]
            with map_lock:
                map_cache[key] = points
                while len(map_cache) > MAP_CACHE_SIZE:
                    map_cache.pop(next(iter(map_cache)))
        return {"points": points}

    return app


def factory() -> FastAPI:
    return create_app(
        MuqEmbedder(),
        EmbeddingCache(Path(os.environ.get("TAGGER_CACHE_DIR", "/cache"))),
        os.environ.get("LUTEDROP_TAGGER_TOKEN", ""),
        reference=load_reference(Path(__file__).parent / "reference", MuqEmbedder.name),
    )


def main() -> None:
    import uvicorn

    uvicorn.run(factory(), host="0.0.0.0", port=8765)


if __name__ == "__main__":
    main()
