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
