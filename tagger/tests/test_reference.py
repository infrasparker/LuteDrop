import numpy as np
import pytest

from tagger.reference import ReferenceSet, load_reference, reference_stats, write_reference


def unit(rows: np.ndarray) -> np.ndarray:
    return rows / np.linalg.norm(rows, axis=1, keepdims=True)


def test_reference_stats_match_hand_computation():
    rng = np.random.default_rng(0)
    V = unit(rng.normal(size=(5, 8)))
    P = {
        "grim": unit(rng.normal(size=(2, 8))),
        "energy+": unit(rng.normal(size=(1, 8))),
        "energy-": unit(rng.normal(size=(1, 8))),
    }
    ref = ReferenceSet("test-set", "fake", V)
    out = reference_stats(ref, P, ["grim"])

    grim = (V @ P["grim"].T).mean(axis=1)
    energy = (V @ P["energy+"].T).mean(axis=1) - (V @ P["energy-"].T).mean(axis=1)
    assert out["set"] == "test-set"
    assert out["tracks"] == 5 == ref.tracks
    assert set(out["stats"]) == {"grim", "energy"}
    assert out["stats"]["grim"]["mean"] == pytest.approx(grim.mean())
    assert out["stats"]["grim"]["sd"] == pytest.approx(np.std(grim, ddof=0))
    assert out["stats"]["energy"]["mean"] == pytest.approx(energy.mean())
    assert out["stats"]["energy"]["sd"] == pytest.approx(np.std(energy, ddof=0))


def test_load_reference_round_trips_and_normalises(tmp_path):
    rng = np.random.default_rng(1)
    raw = rng.normal(size=(4, 8)) * 5
    write_reference(tmp_path / "reference.npz", "test-set", "fake", raw)
    ref = load_reference(tmp_path, "fake")
    assert ref is not None
    assert ref.set_id == "test-set" and ref.model == "fake"
    assert ref.vectors.shape == (4, 8)
    assert np.allclose(np.linalg.norm(ref.vectors, axis=1), 1)
    assert np.allclose(ref.vectors, unit(raw), atol=1e-6)


def test_reference_for_other_model_is_ignored(tmp_path):
    write_reference(tmp_path / "reference.npz", "test-set", "other", np.ones((3, 8)))
    assert load_reference(tmp_path, "fake") is None


def test_load_reference_missing_file_is_none(tmp_path):
    assert load_reference(tmp_path, "fake") is None


def test_load_reference_unreadable_file_is_none(tmp_path):
    (tmp_path / "reference.npz").write_bytes(b"not an npz")
    assert load_reference(tmp_path, "fake") is None
