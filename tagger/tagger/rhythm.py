"""Rhythm features measured from decoded audio, used to train the pace head.

Per window:
  pulse       - how strongly the onset envelope repeats at a beat-like period
                (autocorrelation peak, 0.25-2 s)
  plp, plp_sd - mean / spread of the predominant-local-pulse curve
  perc_ratio  - share of energy in the percussive part (HPSS)
  perc_onsets - onsets per second of the percussive part
  perc_pulse  - pulse clarity of the percussive onset envelope
  low_pulse   - pulse clarity of the low band (< 200 Hz)
  beat_cv     - irregularity of the intervals between PLP peaks
  flux        - mean spectral flux
  loud_var    - loudness variation
"""
import numpy as np

FEATURES = (
    "pulse",
    "plp",
    "plp_sd",
    "perc_ratio",
    "perc_onsets",
    "perc_pulse",
    "low_pulse",
    "beat_cv",
    "flux",
    "loud_var",
)

HOP = 512
WINDOW_SECONDS = 30
WINDOW_CENTRES = (0.25, 0.5, 0.75)
MIN_SECONDS = 5
LOW_BAND_HZ = 200
PULSE_MIN_S, PULSE_MAX_S = 0.25, 2.0


def _clarity(env: np.ndarray, sr: int) -> float:
    env = env - env.mean()
    if not np.any(env):
        return 0.0
    ac = np.correlate(env, env, "full")[len(env) - 1 :]
    ac = ac / (ac[0] + 1e-9)
    lo, hi = int(PULSE_MIN_S * sr / HOP), int(PULSE_MAX_S * sr / HOP)
    seg = ac[lo:hi]
    return float(seg.max()) if len(seg) else 0.0


def _peaks(curve: np.ndarray) -> np.ndarray:
    """Frames where `curve` has a local maximum above its mean."""
    if len(curve) < 3:
        return np.empty(0, dtype=int)
    mid = curve[1:-1]
    is_peak = (mid > curve[:-2]) & (mid >= curve[2:]) & (mid > curve.mean())
    return np.flatnonzero(is_peak) + 1


def features(y: np.ndarray, sr: int) -> dict[str, float]:
    """Rhythm features of one window of mono audio."""
    import librosa

    y = np.asarray(y, dtype=np.float32)
    H, P = librosa.effects.hpss(y)
    total = np.sqrt(np.mean(y**2)) + 1e-9
    o_all = librosa.onset.onset_strength(y=y, sr=sr, hop_length=HOP)
    o_perc = librosa.onset.onset_strength(y=P, sr=sr, hop_length=HOP)
    S = np.abs(librosa.stft(y, hop_length=HOP))
    freqs = librosa.fft_frequencies(sr=sr)
    low = S[freqs < LOW_BAND_HZ]
    o_low = librosa.onset.onset_strength(S=librosa.amplitude_to_db(low), sr=sr, hop_length=HOP)
    plp = librosa.beat.plp(onset_envelope=o_all, sr=sr, hop_length=HOP)
    # Beats are taken as the peaks of the PLP curve rather than from
    # librosa.beat.beat_track, whose numba kernel segfaults on some
    # numba/numpy builds (a crash no try/except can catch).
    ibi = np.diff(_peaks(plp))
    on_perc = librosa.onset.onset_detect(onset_envelope=o_perc, sr=sr, hop_length=HOP)
    rms = librosa.feature.rms(y=y, hop_length=HOP)[0]
    out = {
        "pulse": _clarity(o_all, sr),
        "plp": float(plp.mean()),
        "plp_sd": float(plp.std()),
        "perc_ratio": float(np.sqrt(np.mean(P**2)) / total),
        "perc_onsets": len(on_perc) / (len(y) / sr),
        "perc_pulse": _clarity(o_perc, sr),
        "low_pulse": _clarity(o_low, sr),
        "beat_cv": float(ibi.std() / ibi.mean()) if len(ibi) > 3 and ibi.mean() > 0 else 1.0,
        "flux": float(o_all.mean()),
        "loud_var": float(rms.std() / (rms.mean() + 1e-9)),
    }
    return {k: (v if np.isfinite(v) else 0.0) for k, v in out.items()}


def track_features(y: np.ndarray, sr: int) -> dict[str, float]:
    """Mean of `features` over three 30 s windows centred at 25/50/75% of the
    track. A track shorter than 30 s is measured whole, once."""
    size = WINDOW_SECONDS * sr
    if len(y) <= size:
        windows = [y]
    else:
        windows = []
        for f in WINDOW_CENTRES:
            start = int(min(max(0, f * len(y) - size / 2), len(y) - size))
            windows.append(y[start : start + size])
    rows = [features(w, sr) for w in windows if len(w) > MIN_SECONDS * sr]
    if not rows:
        rows = [features(y, sr)] if len(y) >= HOP * 4 else []
    if not rows:
        return {k: 0.0 for k in FEATURES}
    return {k: float(np.mean([r[k] for r in rows])) for k in FEATURES}
