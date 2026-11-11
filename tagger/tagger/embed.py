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

    def load(self) -> None:
        """Prepare the model so the first embed does not wait for it."""
        ...

    def embed_windows(self, wavs: np.ndarray) -> np.ndarray:
        """Embed windows (windows x samples at SAMPLE_RATE) into windows x dim."""
        ...

    def embed_text(self, texts: list[str]) -> np.ndarray:
        """Embed texts into the audio embedding space, one L2-normalized row each."""
        ...


class MuqEmbedder:
    """MuQ-MuLan audio embedder. torch and muq are imported on first use so
    the rest of the service loads without them."""

    name = "muq-mulan-large"

    def __init__(self) -> None:
        self._model = None
        self._device = None
        self._lock = threading.Lock()

    def load(self) -> None:
        """Load the model weights, downloading them on first use."""
        with self._lock:
            if self._model is None:
                self._load()

    def _load(self) -> None:
        import torch
        from muq import MuQMuLan

        self._device = "cuda" if torch.cuda.is_available() else "cpu"
        self._model = MuQMuLan.from_pretrained(config.MODEL_ID, revision=config.MODEL_REVISION).to(self._device).eval()

    def embed_audio_features(self, wavs: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        """Search vectors and four pooled hidden layers from the same audio pass."""
        import torch
        final, hidden = [], []
        with self._lock:
            if self._model is None: self._load()
            model = self._model; audio = model.mulan.audio
            with torch.inference_mode():
                for offset in range(0, len(wavs), 2):
                    x = torch.tensor(wavs[offset:offset+2], dtype=torch.float32, device=self._device)
                    states = audio.model(x, output_hidden_states=True).hidden_states
                    hidden.extend(torch.stack([states[k].mean(dim=1) for k in [3, 6, 9, 12]], dim=1).cpu().numpy())
                    projected = audio.proj(states[audio.use_layer_idx])
                    projected, _ = audio.transformer(projected, return_all_layers=True)
                    latent = model.mulan.audio_to_latents(projected.mean(dim=-2))
                    latent /= torch.linalg.norm(latent, dim=-1, keepdim=True).clamp_min(1e-12)
                    final.extend(latent.cpu().numpy())
        return np.asarray(final), np.asarray(hidden).mean(axis=0)

    def embed_windows(self, wavs: np.ndarray) -> np.ndarray:
        import torch

        with self._lock:
            if self._model is None:
                self._load()
            with torch.no_grad():
                x = torch.tensor(wavs, dtype=torch.float32, device=self._device)
                return self._model(wavs=x).cpu().numpy()

    def embed_text(self, texts: list[str]) -> np.ndarray:
        """One L2-normalized row per text. Each text goes through the model on
        its own: a batch is padded to its longest text, and that padding shifts
        the vectors of the shorter ones, so a text's vector would depend on
        which other texts were embedded with it."""
        import torch

        with self._lock:
            if self._model is None:
                self._load()
            with torch.no_grad():
                out = np.concatenate(
                    [self._model(texts=[t]).cpu().numpy() for t in texts]
                )
        norms = np.linalg.norm(out, axis=1, keepdims=True)
        return out / np.where(norms == 0, 1, norms)


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
        """Unit-length mean of the track's L2-normalized window embeddings."""
        windows = self.get_windows(hash)
        if windows is None:
            return None
        norms = np.linalg.norm(windows, axis=1, keepdims=True)
        mean = (windows / np.where(norms == 0, 1, norms)).mean(axis=0)
        n = np.linalg.norm(mean)
        return mean / n if n else mean

    def put(self, hash: str, windows: np.ndarray) -> None:
        path = self._path(hash)
        tmp = path.with_name(f"{hash}.{os.getpid()}.{threading.get_ident()}.tmp")
        with open(tmp, "wb") as f:
            np.save(f, np.asarray(windows, dtype=np.float32))
        os.replace(tmp, path)

    def count(self) -> int:
        return sum(1 for _ in self.root.glob("*.npy"))


def _decode(path: str) -> np.ndarray:
    import librosa

    y, _ = librosa.load(path, sr=config.SAMPLE_RATE, mono=True)
    if len(y) == 0:
        raise ValueError("no audio decoded")
    return y


def _cut_windows(y: np.ndarray) -> np.ndarray:
    size = config.WINDOW_SECONDS * config.SAMPLE_RATE
    if len(y) <= size:
        return np.pad(y, (0, size - len(y)))[None, :].astype(np.float32)
    count = min(config.MAX_WINDOWS, len(y) // size)
    starts = np.linspace(0, len(y) - size, count).astype(int)
    return np.stack([y[s : s + size] for s in starts]).astype(np.float32)


def load_windows(path: str) -> np.ndarray:
    """Decode a file at SAMPLE_RATE and cut up to MAX_WINDOWS evenly spaced
    WINDOW_SECONDS windows (windows x samples). Audio shorter than one window
    is zero-padded into a single window."""
    return _cut_windows(_decode(path))


def decode_track(path: str) -> np.ndarray:
    """The embedding windows for one file, as `load_windows`. Runs in a
    worker process (see app.create_app)."""
    return load_windows(path)
