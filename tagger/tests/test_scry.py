import numpy as np
import pytest

from tagger import scry


def unit(*v):
    a = np.array(v, dtype=np.float64)
    return a / np.linalg.norm(a)


def test_similar_ranks_by_cosine_to_query():
    cands = np.vstack([unit(1, 0), unit(1, 1), unit(0, 1), unit(-1, 0)])
    out = scry.similar(unit(1, 0.1), cands, 10)
    assert [i for i, _ in out] == [0, 1, 2, 3]
    assert out[0][1] > out[1][1] > out[2][1] > out[3][1]


def test_similar_respects_limit_and_empty():
    cands = np.vstack([unit(1, 0), unit(0, 1)])
    assert len(scry.similar(unit(1, 0), cands, 1)) == 1
    assert scry.similar(unit(1, 0), np.empty((0, 2)), 5) == []


def test_rank_by_text_orders_by_cosine():
    cands = np.vstack([unit(0, 1), unit(1, 0)])
    out = scry.rank_by_text(unit(1, 0), cands, 5)
    assert out[0][0] == 1
    assert out[0][1] == pytest.approx(1.0)


@pytest.mark.parametrize("n", [0, 1, 2, 4, 12])
def test_layout_shape_and_range(n):
    rng = np.random.default_rng(0)
    pts = scry.layout_2d(rng.normal(size=(n, 16)))
    assert pts.shape == (n, 2)
    if n:
        assert pts.min() >= 0.0 and pts.max() <= 1.0


def test_layout_single_point_is_centered():
    assert scry.layout_2d(np.ones((1, 4))).tolist() == [[0.5, 0.5]]


@pytest.mark.parametrize("n", [4, 6])
@pytest.mark.parametrize("fill", [0.0, 1.0])
def test_layout_degenerate_vectors_sit_at_center(n, fill):
    pts = scry.layout_2d(np.full((n, 4), fill))
    assert pts.tolist() == [[0.5, 0.5]] * n
