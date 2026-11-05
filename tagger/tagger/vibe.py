"""Similarity, text ranking and 2D layout over track embeddings."""

import numpy as np
from sklearn.decomposition import PCA
from sklearn.manifold import TSNE


def _unit_rows(m: np.ndarray) -> np.ndarray:
    norms = np.linalg.norm(m, axis=1, keepdims=True)
    return m / np.where(norms == 0, 1, norms)


def _top(scores: np.ndarray, limit: int) -> list[tuple[int, float]]:
    order = np.argsort(-scores, kind="stable")[: max(limit, 0)]
    return [(int(i), float(scores[i])) for i in order]


def rank_by_text(text_vec: np.ndarray, cands: np.ndarray, limit: int) -> list[tuple[int, float]]:
    """Candidate indices by cosine similarity to one vector, best first."""
    if len(cands) == 0:
        return []
    q = np.asarray(text_vec, dtype=np.float64).ravel()
    n = np.linalg.norm(q)
    q = q / n if n else q
    return _top(_unit_rows(np.asarray(cands, dtype=np.float64)) @ q, limit)


def centroid(vectors: np.ndarray) -> np.ndarray:
    """Unit-length mean of the unit-normalized rows."""
    mean = _unit_rows(np.asarray(vectors, dtype=np.float64)).mean(axis=0)
    n = np.linalg.norm(mean)
    return mean / n if n else mean


def similar(query: np.ndarray, cands: np.ndarray, limit: int) -> list[tuple[int, float]]:
    """Candidate indices by cosine similarity to the query vector (the
    normalized centroid of the seeds), best first."""
    return rank_by_text(query, cands, limit)


def _min_max(points: np.ndarray) -> np.ndarray:
    lo, hi = points.min(axis=0), points.max(axis=0)
    span = hi - lo
    out = np.full_like(points, 0.5)
    ok = span > 1e-12
    out[:, ok] = (points[:, ok] - lo[ok]) / span[ok]
    return out


def layout_2d(vectors: np.ndarray) -> np.ndarray:
    """Place vectors on a plane, shaped (n, 2) with values in [0, 1].

    t-SNE (cosine metric, fixed seed) from 5 vectors up; PCA below that. A
    single vector sits at (0.5, 0.5).
    """
    v = np.asarray(vectors, dtype=np.float64)
    n = len(v)
    if n == 0:
        return np.empty((0, 2))
    if n == 1:
        return np.array([[0.5, 0.5]])
    if np.ptp(v, axis=0).max() < 1e-12:
        # No spread to lay out (identical or all-zero vectors).
        return np.full((n, 2), 0.5)
    if n < 5:
        pts = PCA(n_components=min(2, n, v.shape[1])).fit_transform(v)
        if pts.shape[1] < 2:
            pts = np.hstack([pts, np.zeros((n, 2 - pts.shape[1]))])
    else:
        perplexity = min(30, max(5, (n - 1) // 3))
        pts = TSNE(
            n_components=2, metric="cosine", perplexity=min(perplexity, n - 1), random_state=0
        ).fit_transform(v)
    return _min_max(pts)
