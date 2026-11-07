import numpy as np
import pytest
import soundfile as sf

from tagger import config
from tagger.embed import EmbeddingCache, load_windows


def write_tone(path, seconds):
    t = np.arange(int(seconds * config.SAMPLE_RATE)) / config.SAMPLE_RATE
    sf.write(path, 0.3 * np.sin(2 * np.pi * 440 * t), config.SAMPLE_RATE)


def test_long_file_gives_max_windows(tmp_path):
    p = tmp_path / "long.wav"
    write_tone(p, 120)
    w = load_windows(str(p))
    assert w.shape == (config.MAX_WINDOWS, config.WINDOW_SECONDS * config.SAMPLE_RATE)


def test_short_file_gives_one_padded_window(tmp_path):
    p = tmp_path / "short.wav"
    write_tone(p, 3)
    w = load_windows(str(p))
    assert w.shape == (1, config.WINDOW_SECONDS * config.SAMPLE_RATE)
    assert w[0, -1] == 0.0


def test_cache_roundtrip(tmp_path):
    cache = EmbeddingCache(tmp_path)
    assert cache.get("abc") is None
    cache.put("abc", np.array([[1.0, 3.0], [3.0, 5.0]]))
    assert cache.get_windows("abc").shape == (2, 2)
    assert np.isclose(np.linalg.norm(cache.get("abc")), 1.0)
    assert cache.count() == 1


def test_cache_rejects_path_like_hash(tmp_path):
    cache = EmbeddingCache(tmp_path)
    assert cache.get("../evil") is None
    with pytest.raises(ValueError):
        cache.put("../evil", np.zeros((1, 2)))


def test_cache_mean_is_normalized_window_mean(tmp_path):
    cache = EmbeddingCache(tmp_path)
    cache.put("abc", np.array([[3.0, 0.0], [0.0, 1.0]]))
    v = cache.get("abc")
    assert np.allclose(v, [2**-0.5, 2**-0.5])
    assert np.isclose(np.linalg.norm(v), 1.0)


def test_decode_track_survives_rhythm_failure(tmp_path, monkeypatch, caplog):
    from tagger import embed, rhythm

    p = tmp_path / "t.wav"
    write_tone(p, 3)

    def boom(y, sr):
        raise RuntimeError("beat tracker exploded")

    monkeypatch.setattr(rhythm, "track_features", boom)
    windows, feats = embed.decode_track(str(p))
    assert windows.shape[0] == 1
    assert feats is None
    assert "beat tracker exploded" in caplog.text


def test_embed_text_runs_the_model_once_per_text(monkeypatch):
    """Batching pads short texts to the longest one, which shifts their vectors,
    so each text must go through the model alone."""
    import contextlib
    import sys
    import types

    from tagger.embed import MuqEmbedder

    # torch is not installed in the test venv; embed_text only needs no_grad.
    fake_torch = types.SimpleNamespace(no_grad=contextlib.nullcontext)
    monkeypatch.setitem(sys.modules, "torch", fake_torch)

    class FakeTensor:
        def __init__(self, arr):
            self.arr = arr

        def cpu(self):
            return self

        def numpy(self):
            return self.arr

    calls = []

    def fake_model(texts):
        calls.append(list(texts))
        # Row depends on batch length, so a batched call would differ from singles.
        return FakeTensor(np.array([[3.0 * len(texts), 4.0] for _ in texts]))

    emb = MuqEmbedder()
    emb._model = fake_model
    emb._device = "cpu"
    out = emb.embed_text(["a", "bb", "ccc"])

    assert calls == [["a"], ["bb"], ["ccc"]]
    assert out.shape == (3, 2)
    assert np.allclose(np.linalg.norm(out, axis=1), 1.0)
    assert np.allclose(out, [[0.6, 0.8]] * 3)
