import numpy as np

from tagger import rhythm

SR = 24000


def click_track(seconds=40, every=0.512):
    y = np.zeros(seconds * SR, dtype=np.float32)
    n = int(0.005 * SR)
    rng = np.random.default_rng(0)
    for t in np.arange(0, seconds, every):
        i = int(t * SR)
        y[i : i + n] = rng.uniform(-1, 1, size=min(n, len(y) - i))
    return y


def drone(seconds=40):
    t = np.arange(seconds * SR) / SR
    return (np.sin(2 * np.pi * 220 * t) * (0.6 + 0.2 * np.sin(2 * np.pi * 0.3 * t))).astype(np.float32)


def test_click_track_has_more_pulse_than_drone():
    c = rhythm.track_features(click_track(), SR)
    d = rhythm.track_features(drone(), SR)
    assert c["pulse"] > d["pulse"]
    assert c["perc_ratio"] > d["perc_ratio"]
    assert c["perc_onsets"] > d["perc_onsets"]


def test_track_features_has_all_keys_and_finite():
    f = rhythm.track_features(click_track(), SR)
    assert set(f) == set(rhythm.FEATURES)
    assert all(np.isfinite(v) for v in f.values())


def test_short_track_uses_whole_signal():
    f = rhythm.track_features(click_track(10), SR)
    assert set(f) == set(rhythm.FEATURES)
    assert all(np.isfinite(v) for v in f.values())


def test_silence_is_finite():
    f = rhythm.track_features(np.zeros(12 * SR, dtype=np.float32), SR)
    assert all(np.isfinite(v) for v in f.values())
