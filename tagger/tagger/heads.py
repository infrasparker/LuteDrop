from dataclasses import dataclass

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import roc_auc_score
from sklearn.model_selection import LeaveOneOut

from . import config


@dataclass
class HeadResult:
    auc: float | None
    pos: int
    neg: int
    trusted: bool
    threshold: float | None
    # The same rule on 1 - p with the classes swapped: the smallest 1 - p at
    # which "not this class" predictions reach the target precision.
    neg_threshold: float | None
    scores: np.ndarray


def _new_model() -> LogisticRegression:
    return LogisticRegression(C=config.LR_C, class_weight="balanced", max_iter=5000)


def loo_scores(X: np.ndarray, y: np.ndarray) -> np.ndarray:
    """Leave-one-out positive-class probability for every example."""
    out = np.zeros(len(y))
    for train, test in LeaveOneOut().split(X):
        model = _new_model().fit(X[train], y[train])
        out[test] = model.predict_proba(X[test])[:, 1]
    return out


def _precision_threshold(scores: np.ndarray, y: np.ndarray) -> float | None:
    """Smallest score t where precision of (score >= t) meets the target and
    at least one positive is selected."""
    for t in np.unique(scores):
        selected = scores >= t
        hits = int(y[selected].sum())
        if hits > 0 and hits / int(selected.sum()) >= config.TARGET_PRECISION:
            return float(t)
    return None


def fit_and_score(
    X_pos: np.ndarray, X_neg: np.ndarray, X_targets: np.ndarray
) -> HeadResult:
    """Fit a logistic head and score the targets.

    With fewer than two examples in either class, leave-one-out cannot train
    on both classes, so the result is untrusted (auc and thresholds None) and
    the scores are all 0.0.
    """
    n_pos, n_neg = len(X_pos), len(X_neg)
    if min(n_pos, n_neg) < 2:
        return HeadResult(None, n_pos, n_neg, False, None, None, np.zeros(len(X_targets)))

    X = np.vstack([X_pos, X_neg])
    y = np.array([1] * n_pos + [0] * n_neg)

    loo = loo_scores(X, y)
    auc = float(roc_auc_score(y, loo))
    threshold = _precision_threshold(loo, y)
    neg_threshold = _precision_threshold(1 - loo, 1 - y)
    trusted = (
        n_pos >= config.MIN_POS
        and n_neg >= config.MIN_NEG
        and auc >= config.MIN_AUC
        and threshold is not None
    )
    scores = _new_model().fit(X, y).predict_proba(X_targets)[:, 1]
    return HeadResult(auc, n_pos, n_neg, trusted, threshold, neg_threshold, scores)
