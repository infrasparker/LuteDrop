import numpy as np
import pytest

from tagger.heads import fit_and_score, loo_scores

DIM = 16


def _cluster(rng, center_sign, n):
    base = np.zeros(DIM)
    base[0] = center_sign * 3.0
    return base + rng.normal(0, 0.5, size=(n, DIM))


def _axis(sign):
    v = np.zeros((1, DIM))
    v[0, 0] = sign * 3.0
    return v


@pytest.fixture
def rng():
    return np.random.default_rng(0)


def test_separable_tag_is_trusted(rng):
    pos = _cluster(rng, +1, 8)
    neg = _cluster(rng, -1, 8)
    targets = np.vstack([_axis(+1), _axis(-1)])
    r = fit_and_score(pos, neg, targets)
    assert r.trusted
    assert r.auc >= 0.95
    assert r.threshold is not None
    assert r.scores[0] > 0.5
    assert r.scores[1] < 0.5


def test_too_few_positives_not_trusted(rng):
    pos = _cluster(rng, +1, 4)
    neg = _cluster(rng, -1, 10)
    targets = rng.normal(size=(3, DIM))
    r = fit_and_score(pos, neg, targets)
    assert r.trusted is False
    assert r.pos == 4
    assert len(r.scores) == 3


def test_random_labels_not_trusted(rng):
    pos = rng.normal(size=(10, DIM))
    neg = rng.normal(size=(10, DIM))
    r = fit_and_score(pos, neg, rng.normal(size=(2, DIM)))
    assert r.trusted is False


def test_zero_positives(rng):
    neg = rng.normal(size=(10, DIM))
    r = fit_and_score(np.empty((0, DIM)), neg, rng.normal(size=(4, DIM)))
    assert r.auc is None
    assert r.trusted is False
    assert np.all(r.scores == 0.0)
    assert len(r.scores) == 4


def test_threshold_meets_precision(rng):
    pos = _cluster(rng, +1, 8)
    neg = _cluster(rng, -1, 8)
    r = fit_and_score(pos, neg, _axis(+1))
    assert r.threshold is not None
    labels = np.array([1] * 8 + [0] * 8)
    loo = loo_scores(np.vstack([pos, neg]), labels)
    sel = loo >= r.threshold
    assert sel.sum() > 0
    assert labels[sel].mean() >= 0.8


def test_single_positive_not_trusted(rng):
    neg = _cluster(rng, -1, 10)
    r = fit_and_score(_cluster(rng, +1, 1), neg, rng.normal(size=(3, DIM)))
    assert r.trusted is False
    assert r.pos == 1
    assert len(r.scores) == 3


def test_single_negative_not_trusted(rng):
    pos = _cluster(rng, +1, 10)
    r = fit_and_score(pos, _cluster(rng, -1, 1), rng.normal(size=(3, DIM)))
    assert r.trusted is False
    assert r.neg == 1
    assert len(r.scores) == 3


def test_scaled_head_handles_unscaled_features():
    rng = np.random.default_rng(1)
    n = 12
    scales = np.array([1.0, 1e-3, 1e3])

    def make(shift):
        # Only the tiny-scale column carries the class signal.
        X = rng.normal(0, 1, size=(n, 3)) * scales
        X[:, 1] += shift * 1e-3
        return X

    pos, neg = make(+2.0), make(-2.0)
    targets = np.vstack([make(+2.0)[:1], make(-2.0)[:1]])
    plain = fit_and_score(pos, neg, targets)
    scaled = fit_and_score(pos, neg, targets, scale=True)
    assert not plain.trusted or plain.auc < 0.9
    assert scaled.auc >= 0.9
    assert scaled.scores[0] > scaled.scores[1]
