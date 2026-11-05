import hmac
import logging
import multiprocessing
import os
import threading
from collections.abc import Callable
from concurrent.futures import Executor, ProcessPoolExecutor
from concurrent.futures.process import BrokenProcessPool
from pathlib import Path

import numpy as np
from fastapi import Depends, FastAPI, Header, HTTPException
from pydantic import BaseModel, Field

from . import heads, moods, rhythm, vibe
from .embed import Embedder, EmbeddingCache, MuqEmbedder, decode_track

MAX_EMBED_ITEMS = 32

DEFAULT_RESULTS = 50
MAX_RESULTS = 500
MAP_CACHE_SIZE = 8

# How long /embed waits for the model to finish loading before it answers
# 503. Loading starts when the app is created.
LOAD_WAIT_SECONDS = 10

log = logging.getLogger(__name__)


class EmbedItem(BaseModel):
    hash: str
    path: str


class EmbedRequest(BaseModel):
    items: list[EmbedItem] = Field(max_length=MAX_EMBED_ITEMS)


class Examples(BaseModel):
    positives: list[str] = []
    negatives: list[str] = []


class ScoreRequest(BaseModel):
    scene: str = ""
    gate: Examples
    tags: dict[str, Examples] = {}
    pace: Examples | None = None
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


def _limit(n: int) -> int:
    return min(max(n, 0), MAX_RESULTS)


def _decode(path: str) -> tuple[np.ndarray, dict[str, float] | None]:
    # Module-level so a process pool can pickle it by reference.
    return decode_track(path)


def _failure(hash: str, e: Exception) -> dict:
    return {"hash": hash, "ok": False, "error": str(e) or type(e).__name__}


def create_app(
    embedder: Embedder,
    cache: EmbeddingCache,
    token: str,
    executor_factory: Callable[[], Executor] | None = None,
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
    # and numba code (librosa's onset/PLP kernels) segfaults in forked children.
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
            if cache.has(item.hash) and cache.has_rhythm(item.hash):
                results[item.hash] = {"hash": item.hash, "ok": True}
            elif item.hash not in pending:
                try:
                    pending[item.hash] = submit(item.path)
                except BrokenProcessPool as e:
                    replace_pool(current[0])
                    results[item.hash] = _failure(item.hash, e)
        for hash, (pool, future) in pending.items():
            try:
                windows, feats = future.result()
                if not cache.has(hash):
                    cache.put(hash, embedder.embed_windows(windows))
                if feats is not None:
                    cache.put_rhythm(hash, feats)
                else:
                    log.warning("No rhythm features for %s; pace is unavailable for it", hash)
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

    def rhythm_vectors(hashes: list[str]) -> tuple[list[str], np.ndarray]:
        """Rhythm feature rows (FEATURES order) for the hashes that have them."""
        found, rows = [], []
        for h in dict.fromkeys(hashes):
            feats = cache.get_rhythm(h)
            # A cached file written with a different feature set is skipped.
            if feats is not None and all(k in feats for k in rhythm.FEATURES):
                found.append(h)
                rows.append([feats[k] for k in rhythm.FEATURES])
        return found, (np.array(rows) if rows else np.empty((0, len(rhythm.FEATURES))))

    def score_head(examples: Examples, target_hashes, X_targets, get=vectors, scale=False):
        _, X_pos = get(examples.positives)
        _, X_neg = get(examples.negatives)
        # fit_and_score cannot predict on an empty array; score one example
        # row instead and drop the result when there are no targets.
        probe = X_targets if len(target_hashes) else (X_pos[:1] if len(X_pos) else X_neg[:1])
        r = heads.fit_and_score(X_pos, X_neg, probe, scale)
        return {
            "quality": {"auc": r.auc, "pos": r.pos, "neg": r.neg},
            "trusted": r.trusted,
            "threshold": r.threshold,
            "neg_threshold": r.neg_threshold,
            "scores": {h: float(s) for h, s in zip(target_hashes, r.scores)}
            if len(target_hashes)
            else {},
        }

    # Unit-length mood prompt vectors per pole, embedded the first time /score
    # finds the model loaded and reused after that. Published only once every
    # pole succeeded, so a failure part-way leaves nothing half-filled.
    mood_vecs: dict[str, np.ndarray] = {}
    mood_lock = threading.Lock()

    def mood_poles() -> dict[str, np.ndarray] | None:
        """The pole vectors, or None while the model is not ready (or the
        prompts could not be embedded); /score then omits mood. Never waits."""
        if not loaded.is_set() or load_error:
            return None
        with mood_lock:
            if not mood_vecs:
                built = {}
                try:
                    for name, texts in moods.POLES.items():
                        vecs = np.asarray(embedder.embed_text(texts), dtype=np.float64)
                        norms = np.linalg.norm(vecs, axis=1, keepdims=True)
                        built[name] = vecs / np.where(norms == 0, 1, norms)
                except Exception:
                    log.exception("Mood prompts failed to embed")
                    return None
                mood_vecs.update(built)
            return dict(mood_vecs)

    def mood_scores(target_hashes: list[str], X_targets: np.ndarray) -> dict[str, dict] | None:
        if not target_hashes:
            return {}
        poles = mood_poles()
        if poles is None:
            return None
        sims = {name: (X_targets @ v.T).mean(axis=1) for name, v in poles.items()}
        bright = sims["bright"] - sims["dark"]
        light = sims["light"] - sims["heavy"]
        return {
            h: {"bright": float(b), "light": float(li)}
            for h, b, li in zip(target_hashes, bright, light)
        }

    @app.post("/score")
    def score(req: ScoreRequest):
        target_hashes, X_targets = vectors(req.targets)
        out = {
            "model": embedder.name,
            "gate": score_head(req.gate, target_hashes, X_targets),
            "tags": {
                tag: score_head(ex, target_hashes, X_targets)
                for tag, ex in req.tags.items()
            },
        }
        if req.pace is not None:
            r_hashes, R_targets = rhythm_vectors(req.targets)
            out["pace"] = score_head(req.pace, r_hashes, R_targets, rhythm_vectors, scale=True)
        mood = mood_scores(target_hashes, X_targets)
        if mood is not None:
            out["mood"] = mood
        return out

    @app.post("/similar")
    def similar(req: SimilarRequest):
        _, Q = vectors(req.query)
        excluded = set(req.query)
        c_hashes, C = vectors([h for h in req.candidates if h not in excluded])
        if len(Q) == 0 or len(C) == 0:
            return {"results": []}
        ranked = vibe.similar(vibe.centroid(Q), C, _limit(req.limit))
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
        ranked = vibe.rank_by_text(text_vec, C, _limit(req.limit))
        return {"results": [{"hash": c_hashes[i], "score": s} for i, s in ranked]}

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
            coords = vibe.layout_2d(X[[row[h] for h in key]] if key else X)
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
    )


def main() -> None:
    import uvicorn

    uvicorn.run(factory(), host="0.0.0.0", port=8765)


if __name__ == "__main__":
    main()
