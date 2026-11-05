import os
import re
import threading
from pathlib import Path
from typing import Protocol

import numpy as np

from . import config

_SAFE_HASH = re.compile(r"^[A-Za-z0-9_-]+$")


class Embedder(Protocol):
    name: str

    def embed_windows(self, wavs: np.ndarray) -> np.ndarray:
        """Embed windows (windows x samples at SAMPLE_RATE) into windows x dim."""
        ...


class MuqEmbedder:
    """MuQ-MuLan audio embedder. torch and muq are imported on first use so
    the rest of the service loads without them."""

    name = "muq-mulan-large"

    def __init__(self) -> None:
        self._model = None
        self._device = None
        self._lock = threading.Lock()

    def _load(self) -> None:
        import torch
        from muq import MuQMuLan

        self._device = "cuda" if torch.cuda.is_available() else "cpu"
        self._model = MuQMuLan.from_pretrained(config.MODEL_ID).to(self._device).eval()

    def embed_windows(self, wavs: np.ndarray) -> np.ndarray:
        import torch

        with self._lock:
            if self._model is None:
                self._load()
            with torch.no_grad():
                x = torch.tensor(wavs, dtype=torch.float32, device=self._device)
                return self._model(wavs=x).cpu().numpy()


class EmbeddingCache:
    """Per-track window embeddings stored as `<root>/<hash>.npy`."""

    def __init__(self, root: Path) -> None:
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True)

    def _path(self, hash: str) -> Path:
        if not _SAFE_HASH.match(hash):
            raise ValueError(f"invalid hash: {hash!r}")
        return self.root / f"{hash}.npy"

    def has(self, hash: str) -> bool:
        try:
            return self._path(hash).is_file()
        except ValueError:
            return False

    def get_windows(self, hash: str) -> np.ndarray | None:
        if not self.has(hash):
            return None
        return np.load(self._path(hash))

    def get(self, hash: str) -> np.ndarray | None:
        """Mean of the track's window embeddings."""
        windows = self.get_windows(hash)
        return None if windows is None else windows.mean(axis=0)

    def put(self, hash: str, windows: np.ndarray) -> None:
        path = self._path(hash)
        tmp = path.with_name(f"{hash}.{os.getpid()}.{threading.get_ident()}.tmp")
        with open(tmp, "wb") as f:
            np.save(f, np.asarray(windows, dtype=np.float32))
        os.replace(tmp, path)

    def count(self) -> int:
        return sum(1 for _ in self.root.glob("*.npy"))


def load_windows(path: str) -> np.ndarray:
    """Decode a file at SAMPLE_RATE and cut up to MAX_WINDOWS evenly spaced
    WINDOW_SECONDS windows (windows x samples). Audio shorter than one window
    is zero-padded into a single window."""
    import librosa

    y, _ = librosa.load(path, sr=config.SAMPLE_RATE, mono=True)
    size = config.WINDOW_SECONDS * config.SAMPLE_RATE
    if len(y) == 0:
        raise ValueError("no audio decoded")
    if len(y) <= size:
        return np.pad(y, (0, size - len(y)))[None, :].astype(np.float32)
    count = min(config.MAX_WINDOWS, len(y) // size)
    starts = np.linspace(0, len(y) - size, count).astype(int)
    return np.stack([y[s : s + size] for s in starts]).astype(np.float32)
