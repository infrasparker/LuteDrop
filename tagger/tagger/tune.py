"""Search core for mood auto-tune: pure numpy, no model, no HTTP.

Given cosine matrices (prompt units x songs) and a list of flags (songs the user
says should read high / mid / low), find combinations of prompt units whose
mood bars best fit those flags. Bars use the same mapping as the app: z-score
against a reference distribution, then a logistic curve.
"""
import itertools
import re
from dataclasses import dataclass

import numpy as np
from scipy.stats import rankdata

AIM_MARGIN = 0.03        # aim this far inside a band edge so near-misses still pass
ORDER_PENALTY = 0.2      # per (high flag, low flag) pair where high <= low
FOCUS_WEIGHT = 3.0       # a focused flag (the one the user just added) counts this many times over
# Stability, for the search only: unflagged songs cost nothing to move otherwise, so the
# search would widen a mood freely. See `stability`.
STABILITY_WEIGHT = 1.0   # times the mean bar move of unflagged songs
FIRES_WEIGHT = 0.6       # times the change in songs at 0.5 or more, over max(fires before, 10)
FIRES_FLOOR = 10
LINE_COST = 0.01         # per line, so a tie on everything else goes to the shorter combination
# Words too small to tell two lines apart (see `words_of`).
SMALL_WORDS = frozenset({"a", "an", "the", "and", "or", "of", "in", "on", "to", "with", "for", "at", "by"})
MAX_UNIT_CHARS = 300
LOO_MAX_FLAGS = 20       # leave-one-out checks at most this many flags
SPLIT_MIN_HIGH = 4       # a split needs this many high-side flags
SPLIT_MAX_RHO = 0.9      # halves ranking the library this alike are one mood
TOP_N = 15               # songs listed at the top of a report
MOVERS_N = 10            # risers and fallers listed in a report


@dataclass(frozen=True)
class FlagTarget:
    index: int           # row of the song in the library matrix
    lo: float | None     # bar should be at least this
    hi: float | None     # bar should be at most this
    weight: float        # stale flags count less
    fresh: bool
    side: str            # "high" | "low" | "mid"; mid flags skip the order penalty
    focus: bool = False  # the flag the user is tuning around: its weight counts FOCUS_WEIGHT times
    target: float | None = None  # continuous strength rating; no interval dead zone
    polarity: int | None = None  # +1 belongs, -1 does not belong
    confidence: float = 1.0
    pivot: float = .5  # Higher/Lower keep their original comparison anchor
    feedback_version: int = 0


@dataclass(frozen=True)
class Mapping:
    midpoint_z: float
    slope: float
    source: str          # "library" | "reference"


@dataclass
class Combo:
    lines: tuple[str, ...]
    loss: float


def split_units(texts: list[str]) -> list[str]:
    """Comma-split, strip, lowercase; drop empties, repeats and items over 300 chars."""
    seen: dict[str, None] = {}
    for text in texts:
        for part in text.split(","):
            unit = part.strip().lower()
            if unit and len(unit) <= MAX_UNIT_CHARS:
                seen.setdefault(unit)
    return list(seen)


def excluded(text: str, words) -> bool:
    """Whether `text` holds any of `words` (each a word or phrase) as a whole word,
    ignoring case. Blank entries match nothing."""
    for w in words:
        w = " ".join(w.lower().split())
        if w and re.search(r"(?<![\w'])" + re.escape(w) + r"(?![\w'])", text.lower()):
            return True
    return False


class UnitPool:
    """The search's units in order, each with the sources that contributed it.

    A source is credited only for units it brought: `add_parts` and `add_whole`
    credit every unit the source holds (two sources holding "sad" both did), but a
    source added with `only_new` (the phrase list, the built-in vocabulary) is only
    credited for texts nobody brought before it."""

    def __init__(self) -> None:
        self.sources: dict[str, list[str]] = {}

    def _add(self, unit: str, label: str) -> None:
        labels = self.sources.setdefault(unit, [])
        if label not in labels:
            labels.append(label)

    def add_parts(self, label: str, texts) -> None:
        """Each text comma-split, stripped and lowercased, as `split_units`."""
        for unit in split_units(texts):
            self._add(unit, label)

    def add_whole(self, label: str, texts, *, only_new: bool = False, lower: bool = False) -> None:
        """Each text whole (stripped; lowercased when `lower`), skipping blanks and
        texts over the line length limit; with `only_new`, skipping texts already in."""
        for text in texts:
            unit = text.strip().lower() if lower else text.strip()
            if not unit or len(unit) > MAX_UNIT_CHARS or (only_new and unit in self.sources):
                continue
            self._add(unit, label)

    def drop(self, rule) -> None:
        """Removes every unit `rule(unit)` is true for."""
        self.sources = {u: labels for u, labels in self.sources.items() if not rule(u)}

    @property
    def units(self) -> list[str]:
        return list(self.sources)


def words_of(line: str) -> frozenset[str]:
    """The words of a line, lowercased, without punctuation or SMALL_WORDS. The search
    never puts two lines in one combination when one's words are all in the other's."""
    return frozenset(w for w in re.findall(r"[a-z0-9']+", line.lower()) if w not in SMALL_WORDS)


def _overlaps(a: frozenset[str], b: frozenset[str]) -> bool:
    return a <= b or b <= a


def bars(raw: np.ndarray, ref_raw: np.ndarray, m: Mapping) -> np.ndarray:
    """Logistic bar in [0, 1] for each raw score, z-scored against ref_raw.

    For library source the caller passes the library raw itself as ref_raw.
    Population sd (ddof=0). A flat reference (sd == 0) has no spread to measure
    against, so every bar is 0.5.
    """
    sd = float(np.std(ref_raw))
    if sd == 0.0:
        return np.full(np.shape(raw), 0.5)
    z = (np.asarray(raw, dtype=float) - float(np.mean(ref_raw))) / sd
    exponent = np.clip(-m.slope * (z - m.midpoint_z), -500.0, 500.0)
    return np.clip(1.0 / (1.0 + np.exp(exponent)), 0.0, 1.0)


def loss(b: np.ndarray, flags: list[FlagTarget]) -> float:
    return _loss_fn(flags)(b)


def _loss_fn(flags: list[FlagTarget]):
    """loss(b, flags) as a function of b alone, with the flag arrays built once.

    Per flag: weighted distance short of its floor and past its ceiling, each aimed
    AIM_MARGIN inside the band; a focused flag's weight is FOCUS_WEIGHT times its own. Plus ORDER_PENALTY for every (high, low) pair where
    the high song reads at or below the low one; a pair is only as sure as its
    shakier flag, so it counts at the smaller of the two weights.
    """
    if any(f.polarity is not None or f.feedback_version >= 4 for f in flags):
        from .personal import directional_losses
        weight = np.array([f.weight*f.confidence for f in flags],dtype=float); weight /= max(1.,weight.sum())
        return lambda b: float(weight@directional_losses(np.tan(np.pi*(np.clip(np.asarray(b)[[f.index for f in flags]],1e-12,1-1e-12)-.5)),flags)[0])
    if flags and all(f.target is not None for f in flags):
        return lambda b: float(np.mean([(b[f.index]-f.target)**2 for f in flags]))
    idx = np.array([f.index for f in flags], dtype=int)
    lo = np.array([np.nan if f.lo is None else f.lo + AIM_MARGIN for f in flags])
    hi = np.array([np.nan if f.hi is None else f.hi - AIM_MARGIN for f in flags])
    w = np.array([f.weight * (FOCUS_WEIGHT if f.focus else 1.0) for f in flags])
    high = [p for p, f in enumerate(flags) if f.side == "high"]
    low = [p for p, f in enumerate(flags) if f.side == "low"]
    pair_w = ORDER_PENALTY * np.minimum.outer(w[high], w[low]) if high and low else None
    hi_idx, lo_idx = idx[high], idx[low]

    def fn(b: np.ndarray) -> float:
        if not flags:
            return 0.0
        v = b[idx]
        miss = np.nan_to_num(np.maximum(0.0, lo - v)) + np.nan_to_num(np.maximum(0.0, v - hi))
        total = float((w * miss).sum())
        if pair_w is not None:
            total += float(pair_w[b[hi_idx][:, None] <= b[lo_idx][None, :]].sum())
        return total

    return fn


def stability(b: np.ndarray, before: np.ndarray, flags: list[FlagTarget]) -> float:
    """How far bars `b` reshape the mood away from `before` (the current lines' bars):
    STABILITY_WEIGHT times the mean absolute bar move over unflagged songs, plus
    FIRES_WEIGHT times the change in how many songs fire (bar 0.5 or more), relative
    to max(fires before, FIRES_FLOOR). 0.0 when nothing moves."""
    return _stability_fn(before, flags)(b)


def _stability_fn(before: np.ndarray, flags: list[FlagTarget]):
    """`stability` as a function of b alone, with the unflagged mask built once."""
    loose = np.ones(len(before), dtype=bool)
    loose[[f.index for f in flags]] = False
    b0 = before[loose]
    fires0 = int((before >= 0.5).sum())
    denom = max(fires0, FIRES_FLOOR)

    def fn(b: np.ndarray) -> float:
        move = float(np.abs(b[loose] - b0).mean()) if b0.size else 0.0
        fires = abs(int((b >= 0.5).sum()) - fires0) / denom
        return STABILITY_WEIGHT * move + FIRES_WEIGHT * fires

    return fn


def _search_loss_fn(flags: list[FlagTarget], before: np.ndarray | None):
    """The loss the search ranks by: the flags' loss, plus `stability` against `before`
    when it is given (the current lines' bars)."""
    flag_fn = _loss_fn(flags)
    if before is None:
        return flag_fn
    stab = _stability_fn(before, flags)
    return lambda b: flag_fn(b) + stab(b)


def broken(b: np.ndarray, flags: list[FlagTarget]) -> list[int]:
    """Positions (in `flags`) whose bar falls outside [lo, hi]."""
    out = []
    for pos, f in enumerate(flags):
        if f.target is not None or f.polarity is not None or f.feedback_version >= 4: continue  # preferences cannot pass or fail
        v = b[f.index]
        if (f.lo is not None and v < f.lo) or (f.hi is not None and v > f.hi):
            out.append(pos)
    return out


def combo_raw(U: np.ndarray, rows: list[int]) -> np.ndarray:
    return U[rows].mean(axis=0)


def _score(rows, U_lib, U_ref, loss_fn, m: Mapping) -> float:
    raw = combo_raw(U_lib, list(rows))
    ref = raw if U_ref is None else combo_raw(U_ref, list(rows))
    return loss_fn(bars(raw, ref, m)) + LINE_COST * len(rows)


def best_rows(U_lib: np.ndarray, U_ref: np.ndarray | None, rows: list[int], flags: list[FlagTarget],
              m: Mapping, k: int, *, before: np.ndarray | None = None) -> list[int]:
    """The `k` of `rows` whose unit alone scores best by the search's loss, best first."""
    loss_fn = _search_loss_fn(flags, before)
    return [r for _, r in sorted((_score((r,), U_lib, U_ref, loss_fn, m), r) for r in rows)[:k]]


def diverse_rows(U_lib, U_ref, rows, flags, m, k, *, before=None):
    """Keep mostly overall winners, reserving at most a third for flag specialists.

    A weak standalone unit can complement another unit. Pruning exclusively by
    aggregate loss discards it before combinations are tested. Specialist slots
    use the same band loss, focus and stale weights; final proposals still pay
    the full stability cost. Small searches retain their original ordering.
    """
    ranked = best_rows(U_lib, U_ref, rows, flags, m, k, before=before)
    if len(flags) < 2 or k < 3 or len(rows) <= k:
        return ranked
    reserve = k // 3
    chosen = ranked[:k - reserve]
    unit_bars = {r: _combo_bars(U_lib, U_ref, [r], m) for r in rows}
    order = {r: i for i, r in enumerate(best_rows(U_lib, U_ref, rows, flags, m, len(rows), before=before))}
    targets = sorted(flags, key=lambda f: (not f.focus, not f.fresh, -f.weight, f.index))
    for f in itertools.islice(itertools.cycle(targets), len(targets) * reserve):
        if len(chosen) >= k:
            break
        candidates = [r for r in rows if r not in chosen]
        if not candidates:
            break
        # Overall rank breaks specialist ties so stability still guides selection.
        chosen.append(min(candidates, key=lambda r: (loss(unit_bars[r], [f]), order[r])))
    return chosen


def beam_search(U_lib: np.ndarray, U_ref: np.ndarray | None, units: list[str],
                flags: list[FlagTarget], m: Mapping, *, top_units: int = 60,
                beam: int = 24, max_lines: int = 6, keep: int = 3,
                seeds=(), before: np.ndarray | None = None) -> list[Combo]:
    """Best prompt combinations by loss (plus `stability` against `before`, the current
    lines' bars, when given, and LINE_COST per line).

    Scores every unit alone, keeps mostly the best `top_units` plus flag specialists
    within that budget, then grows combinations one unit at a time, keeping `beam`
    per depth with a minority reserved for distinct flag failures. Each seed (a tuple
    of rows, such as the mood's current lines) and each seed less one line also
    join the frontier at their depth, so "the current lines plus, minus or with one
    line swapped" is always explored even when those lines are outside the pool.
    Seeds longer than `max_lines` are skipped. Combinations are deduplicated by
    their set of rows; lines come back in unit order. Growth never adds a line whose
    words (`words_of`) are all in a line already there, or the other way round.
    """
    words = [words_of(u) for u in units]
    loss_fn = _search_loss_fn(flags, before)
    solo = sorted((_score((i,), U_lib, U_ref, loss_fn, m), (i,)) for i in range(len(units)))
    pool = diverse_rows(U_lib, U_ref, list(range(len(units))), flags, m, top_units, before=before)
    seen = {rows for _, rows in solo}
    seeded: dict[tuple[int, ...], float] = {}
    for seed in seeds:
        seed = tuple(sorted(set(seed)))
        if not seed or len(seed) > max_lines:
            continue
        for c in [seed] + [seed[:i] + seed[i + 1:] for i in range(len(seed))]:
            if c and c not in seeded:
                seeded[c] = _score(c, U_lib, U_ref, loss_fn, m)
    best = list(solo) + [(l, c) for c, l in seeded.items() if c not in seen]
    seen |= set(seeded)

    def with_seeds(top: list, depth: int) -> list:
        extra = [(l, c) for c, l in seeded.items() if len(c) == depth]
        return top + [e for e in extra if e not in top]

    def frontier_rows(candidates):
        # Preserve distinct ways of missing flags, rather than spending the whole
        # beam on near-identical candidates. The majority remains loss-ranked.
        if len(flags) < 2 or beam < 3:
            return candidates[:beam]
        selected = list(candidates[:beam - beam // 3])
        signatures = {tuple(broken(_combo_bars(U_lib, U_ref, list(c), m), flags)) for _, c in selected}
        for item in candidates[len(selected):]:
            signature = tuple(broken(_combo_bars(U_lib, U_ref, list(item[1]), m), flags))
            if signature not in signatures:
                selected.append(item)
                signatures.add(signature)
                if len(selected) == beam:
                    break
        for item in candidates:
            if len(selected) >= beam:
                break
            if item not in selected:
                selected.append(item)
        return selected

    frontier = with_seeds(frontier_rows(solo), 1)
    for depth in range(2, max_lines + 1):
        grown = []
        for _, rows in frontier:
            for u in pool:
                if u in rows or any(_overlaps(words[u], words[r]) for r in rows):
                    continue
                cand = tuple(sorted(rows + (u,)))
                if cand in seen:
                    continue
                seen.add(cand)
                grown.append((_score(cand, U_lib, U_ref, loss_fn, m), cand))
        grown.sort()
        frontier = with_seeds(frontier_rows(grown), depth)
        if not frontier:
            break
        best += grown
    best.sort()
    selected = best[:keep]
    # Keep the loss winner. If the other slots all repeat its flag failures,
    # expose a repair that holds a strict superset of its fresh flags. The API
    # still rejects proposals worse than Current and reports stability separately.
    if keep > 1 and selected and flags:
        def fresh_misses(item):
            return frozenset(p for p in broken(_combo_bars(U_lib, U_ref, list(item[1]), m), flags) if flags[p].fresh)
        primary = fresh_misses(selected[0])
        if primary and all(fresh_misses(c) == primary for c in selected):
            repair = next((c for c in best[len(selected):] if fresh_misses(c) < primary), None)
            if repair is not None:
                selected[-1] = repair
                selected.sort()
    return [Combo(tuple(units[i] for i in rows), l) for l, rows in selected]


# --- report pieces -----------------------------------------------------------


def spearman(a: np.ndarray, b: np.ndarray) -> float:
    """Rank correlation (ties share their average rank); 0.0 when either side is flat."""
    ra = rankdata(a)
    rb = rankdata(b)
    ra = ra - ra.mean()
    rb = rb - rb.mean()
    denom = float(np.sqrt((ra ** 2).sum() * (rb ** 2).sum()))
    return 0.0 if denom == 0.0 else float((ra * rb).sum() / denom)


def _rows(units: list[str], lines) -> list[int]:
    return [units.index(line) for line in lines]


def combo_raws(U_lib, U_ref, units: list[str], lines) -> tuple[np.ndarray, np.ndarray | None]:
    """(library raw, reference raw or None) of the combination made of `lines`, each a unit."""
    rows = _rows(units, lines)
    return combo_raw(U_lib, rows), None if U_ref is None else combo_raw(U_ref, rows)


def _combo_bars(U_lib, U_ref, rows: list[int], m: Mapping) -> np.ndarray:
    raw = combo_raw(U_lib, rows)
    return bars(raw, raw if U_ref is None else combo_raw(U_ref, rows), m)


def blend_units(U_lib, U_ref, units: list[str], flags: list[FlagTarget], m: Mapping,
                top: int = 8, *, before: np.ndarray | None = None) -> list[str]:
    """The texts of `blend_groups`."""
    return [text for text, _ in blend_groups(U_lib, U_ref, units, flags, m, top, before=before)]


def blend_groups(U_lib, U_ref, units: list[str], flags: list[FlagTarget], m: Mapping,
                 top: int = 8, *, before: np.ndarray | None = None) -> list[tuple[str, tuple[int, ...]]]:
    """Comma blends ("a, b" and "a, b, c") of every pair and triple of the `top`
    best single units (by the search's loss, so with `stability` against `before`
    when given), plus pairs from a pool of at most twice `top` with specialist slots,
    each with the rows of the units it is made of. A unit that is itself
    a comma list adds its parts, and a part that repeats, or whose words (`words_of`)
    are all in the parts before it, is left out. Blends with fewer than two distinct
    parts, over the line length limit, or already in `units` are dropped."""
    loss_fn = _search_loss_fn(flags, before)
    solo = sorted((_score((i,), U_lib, U_ref, loss_fn, m), i) for i in range(len(units)))
    best = [i for _, i in solo[:top]]
    # Keep the existing pairs/triples, then try pairs involving specialists.
    # These are real comma-text embeddings at the caller, not averaged vectors.
    diverse = diverse_rows(U_lib, U_ref, list(range(len(units))), flags, m, top * 2, before=before) if top >= 3 else best
    taken = set(units)
    out: list[tuple[str, tuple[int, ...]]] = []
    for members, n in ((best, 2), (best, 3), (diverse, 2)):
        for group in itertools.combinations(members, n):
            parts: list[str] = []
            covered: set[str] = set()
            for p in (p.strip() for i in group for p in units[i].split(",")):
                # A part whose words are all in the blend already adds nothing but length.
                if p and p not in parts and not (words_of(p) and words_of(p) <= covered):
                    parts.append(p)
                    covered |= words_of(p)
            text = ", ".join(parts)
            if len(parts) >= 2 and len(text) <= MAX_UNIT_CHARS and text not in taken:
                taken.add(text)
                out.append((text, tuple(group)))
    return out


def name_suggestions(U_lib, units: list[str], candidates: list[str], combo_raw: np.ndarray,
                     k: int = 3) -> list[tuple[str, float]]:
    """The `k` candidate words whose own library ranking best matches the combination's
    (rank correlation, best first). A candidate must also be one of `units`."""
    index = {u: i for i, u in enumerate(units)}
    scored = [(w, spearman(U_lib[index[w]], combo_raw)) for w in dict.fromkeys(candidates) if w in index]
    scored.sort(key=lambda t: -t[1])
    return scored[:k]


def leave_one_out(U_lib, U_ref, units: list[str], flags: list[FlagTarget], m: Mapping,
                  winner: Combo, *, seeds=(), memo: dict | None = None,
                  before: np.ndarray | None = None) -> tuple[int, int]:
    """(held, total): for each fresh flag (at most 20, lowest song index first), search
    again without it and see whether the best combination of the winner's size still puts
    that song inside its band. Shows whether the proposal generalises or only memorises.

    The answer depends only on the flags and the winner's size, so pass the same `memo`
    dict for proposals of one request to search each (flag, size) once.
    """
    memo = {} if memo is None else memo
    fresh = sorted((f for f in flags if f.fresh), key=lambda f: f.index)[:LOO_MAX_FLAGS]
    held = 0
    for f in fresh:
        key = (f, tuple(flags), len(winner.lines))
        if key not in memo:
            rest = [g for g in flags if g is not f]
            best = beam_search(U_lib, U_ref, units, rest, m, beam=8, max_lines=len(winner.lines),
                               keep=1, seeds=seeds, before=before)[0]
            b = _combo_bars(U_lib, U_ref, _rows(units, best.lines), m)
            memo[key] = not broken(b, [f])
        held += memo[key]
    return held, len(fresh)


def _two_means(X: np.ndarray, iterations: int = 10) -> np.ndarray:
    """Group label (0 or 1) per unit-length row: seeded with the least similar pair,
    then rows go to the nearer group by cosine."""
    sim = X @ X.T
    a, b = np.unravel_index(np.argmin(sim), sim.shape)
    centroids = np.vstack([X[a], X[b]])
    labels = np.zeros(len(X), dtype=int)
    for _ in range(iterations):
        unit = centroids / np.maximum(np.linalg.norm(centroids, axis=1, keepdims=True), 1e-12)
        labels = np.argmax(X @ unit.T, axis=1)
        for g in (0, 1):
            if (labels == g).any():
                centroids[g] = X[labels == g].mean(axis=0)
    return labels


def try_split(U_lib, U_ref, units: list[str], flags: list[FlagTarget], m: Mapping,
              X_lib: np.ndarray, best: Combo, *, before: np.ndarray | None = None) -> dict | None:
    """Offer to split one mood in two when its high-side flags pull apart. Each half's
    search pays `stability` against `before` (the current lines' bars) when given.

    Only when `best` leaves a fresh flag broken, there are at least 4 high-side flags,
    each half (its high flags plus every low/mid flag) is fully satisfied by its own best
    combination, and the halves rank the library differently (rho < 0.9). Returns
    {"a": Combo, "b": Combo, "a_flags": [...], "b_flags": [...], "rho": float}, the flag
    lists holding positions in `flags` of each half's high flags; None otherwise. Which
    half keeps the name of the mood is left to the caller.
    """
    best_bars = _combo_bars(U_lib, U_ref, _rows(units, best.lines), m)
    if not any(flags[p].fresh for p in broken(best_bars, flags)):
        return None
    high = [p for p, f in enumerate(flags) if f.side == "high"]
    if len(high) < SPLIT_MIN_HIGH:
        return None
    labels = _two_means(X_lib[[flags[p].index for p in high]])
    groups = [[p for p, g in zip(high, labels) if g == side] for side in (0, 1)]
    if not all(groups):
        return None
    rest = [p for p, f in enumerate(flags) if f.side != "high"]
    combos, raws = [], []
    for group in groups:
        sub = [flags[p] for p in group + rest]
        c = beam_search(U_lib, U_ref, units, sub, m, keep=1, before=before)[0]
        rows = _rows(units, c.lines)
        if any(sub[p].fresh for p in broken(_combo_bars(U_lib, U_ref, rows, m), sub)):
            return None
        combos.append(c)
        raws.append(combo_raw(U_lib, rows))
    rho = spearman(raws[0], raws[1])
    if rho >= SPLIT_MAX_RHO:
        return None
    return {"a": combos[0], "b": combos[1], "a_flags": groups[0], "b_flags": groups[1], "rho": rho}


def _top(b: np.ndarray, hashes: list[str]) -> list[dict]:
    order = np.argsort(-b, kind="stable")[:TOP_N]
    return [{"hash": hashes[i], "bar": float(b[i])} for i in order]


def build_report(*, lines, after, before, flags: list[FlagTarget], flag_hashes: list[str],
                 hashes: list[str], m: Mapping, other_raw: dict[str, np.ndarray],
                 loo: tuple[int, int], names: list[tuple[str, float]],
                 sources: dict[str, list[str]] | None = None,
                 after_bars: np.ndarray | None = None, before_bars: np.ndarray | None = None) -> dict:
    """One review report (the JSON shape /tune returns).

    `after` and `before` are (library raw, reference raw or None) for this combination
    and for the current lines of the mood; `hashes` names each library row; `other_raw`
    is each other mood's library raw, for the overlap notes; `sources` names where
    each line came from (`line_sources`, aligned with `lines`; [] when unknown).
    """
    sources = sources or {}
    b_after = bars(after[0], after[0] if after[1] is None else after[1], m) if after_bars is None else after_bars
    b_before = bars(before[0], before[0] if before[1] is None else before[1], m) if before_bars is None else before_bars
    broken_after = set(broken(b_after, flags))
    stab = stability(b_after, b_before, flags)
    flagged = {f.index for f in flags}
    delta = b_after - b_before
    loose = [int(i) for i in np.argsort(-delta, kind="stable") if int(i) not in flagged]
    risers = [i for i in loose if delta[i] > 0][:MOVERS_N]
    fallers = [i for i in reversed(loose) if delta[i] < 0][:MOVERS_N]

    def mover(i: int) -> dict:
        return {"hash": hashes[i], "before": float(b_before[i]), "after": float(b_after[i])}

    # Each song that leaves the top list, with its rank (1-based) in the whole new ranking and its new bar.
    rank_after = np.empty(len(b_after), dtype=int)
    rank_after[np.argsort(-b_after, kind="stable")] = np.arange(1, len(b_after) + 1)
    top_now = set(np.argsort(-b_after, kind="stable")[:TOP_N].tolist())
    out_ranks = [{"hash": hashes[i], "rank": int(rank_after[i]), "bar": float(b_after[i])}
                 for i in np.argsort(-b_before, kind="stable")[:TOP_N].tolist() if i not in top_now]

    return {
        "lines": list(lines),
        "line_sources": [list(sources.get(line) or sources.get(line.lower(), [])) for line in lines],
        # What the search ranks by: the flags' loss, stability (0.0 for the current lines)
        # and the line cost.
        "loss": loss(b_after, flags) + stab + LINE_COST * len(lines),
        "stability": stab,
        "flags": [
            {"hash": h, "before": float(b_before[f.index]), "after": float(b_after[f.index]),
             "holds": p not in broken_after, "fresh": f.fresh, "focus": f.focus}
            for p, (f, h) in enumerate(zip(flags, flag_hashes))
        ],
        "top": _top(b_after, hashes),
        "top_before": _top(b_before, hashes),
        "risers": [mover(i) for i in risers],
        "fallers": [mover(i) for i in fallers],
        "fires_before": int((b_before >= 0.5).sum()),
        "fires_after": int((b_after >= 0.5).sum()),
        "overlap": [
            {"mood_id": mood_id, "before": spearman(b_before, raw), "after": spearman(b_after, raw)}
            for mood_id, raw in other_raw.items()
        ],
        "loo": {"held": loo[0], "total": loo[1]},
        "names": [{"word": w, "rho": r} for w, r in names],
        "unsatisfied": [flag_hashes[p] for p in sorted(broken_after) if flags[p].fresh],
        "out_ranks": out_ranks,
    }
