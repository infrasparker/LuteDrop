"""Search core for mood auto-tune: pure numpy, no model, no HTTP.

Given cosine matrices (prompt units x songs) and a list of flags (songs the user
says should read high / mid / low), find combinations of prompt units whose
mood bars best fit those flags. Bars use the same mapping as the app: z-score
against a reference distribution, then a logistic curve.
"""
from dataclasses import dataclass

import numpy as np

AIM_MARGIN = 0.03        # aim this far inside a band edge so near-misses still pass
ORDER_PENALTY = 0.2      # per (high flag, low flag) pair where high <= low
MAX_UNIT_CHARS = 300


@dataclass(frozen=True)
class FlagTarget:
    index: int           # row of the song in the library matrix
    lo: float | None     # bar should be at least this
    hi: float | None     # bar should be at most this
    weight: float        # stale flags count less
    fresh: bool
    side: str            # "high" | "low" | "mid"; mid flags skip the order penalty


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
    total = 0.0
    for f in flags:
        v = b[f.index]
        miss = 0.0
        if f.lo is not None:
            miss += max(0.0, (f.lo + AIM_MARGIN) - v)
        if f.hi is not None:
            miss += max(0.0, v - (f.hi - AIM_MARGIN))
        total += f.weight * miss
    for hi in (f for f in flags if f.side == "high"):
        for lo in (f for f in flags if f.side == "low"):
            if b[hi.index] <= b[lo.index]:
                # A pair is only as sure as its shakier flag.
                total += ORDER_PENALTY * min(hi.weight, lo.weight)
    return float(total)


def broken(b: np.ndarray, flags: list[FlagTarget]) -> list[int]:
    """Positions (in `flags`) whose bar falls outside [lo, hi]."""
    out = []
    for pos, f in enumerate(flags):
        v = b[f.index]
        if (f.lo is not None and v < f.lo) or (f.hi is not None and v > f.hi):
            out.append(pos)
    return out


def combo_raw(U: np.ndarray, rows: list[int]) -> np.ndarray:
    return U[rows].mean(axis=0)


def _score(rows: list[int], U_lib, U_ref, flags, m: Mapping) -> float:
    raw = combo_raw(U_lib, rows)
    ref = raw if U_ref is None else combo_raw(U_ref, rows)
    return loss(bars(raw, ref, m), flags)


def beam_search(U_lib: np.ndarray, U_ref: np.ndarray | None, units: list[str],
                flags: list[FlagTarget], m: Mapping, *, top_units: int = 24,
                beam: int = 8, max_lines: int = 6, keep: int = 3) -> list[Combo]:
    """Best prompt combinations by loss.

    Scores every unit alone, keeps the best `top_units` as the pool, then grows
    combinations one unit at a time, keeping `beam` per depth. Combinations are
    deduplicated by their set of rows; lines come back in unit order.
    """
    solo = sorted(((_score([i], U_lib, U_ref, flags, m), (i,)) for i in range(len(units))))
    pool = [rows[0] for _, rows in solo[:top_units]]
    frontier = solo[:beam]
    best = list(solo)
    seen = {rows for _, rows in solo}
    for _ in range(2, max_lines + 1):
        grown = []
        for _, rows in frontier:
            for u in pool:
                if u in rows:
                    continue
                cand = tuple(sorted(rows + (u,)))
                if cand in seen:
                    continue
                seen.add(cand)
                grown.append((_score(list(cand), U_lib, U_ref, flags, m), cand))
        if not grown:
            break
        grown.sort()
        frontier = grown[:beam]
        best += grown
    best.sort()
    return [Combo(tuple(units[i] for i in rows), l) for l, rows in best[:keep]]
