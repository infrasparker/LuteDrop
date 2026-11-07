"""A shipped reference set of soundtrack tracks. Each /score reports the mean
and standard deviation of the raw mood similarity over this set, so the app can
read a track's score relative to typical soundtrack music."""

import logging
from dataclasses import dataclass
from pathlib import Path

import numpy as np

log = logging.getLogger(__name__)

FILENAME = "reference.npz"


@dataclass(frozen=True)
class ReferenceSet:
    set_id: str
    model: str
    vectors: np.ndarray  # N x D float64, unit rows

    @property
    def tracks(self) -> int:
        return len(self.vectors)


def _unit_rows(vectors: np.ndarray) -> np.ndarray:
    v = np.asarray(vectors, dtype=np.float64)
    norms = np.linalg.norm(v, axis=1, keepdims=True)
    return v / np.where(norms == 0, 1, norms)


def write_reference(path: Path, set_id: str, model: str, vectors: np.ndarray) -> None:
    """Save a reference set (one track vector per row) as an .npz."""
    np.savez(
        path,
        vectors=np.asarray(vectors, dtype=np.float32),
        set_id=np.array(set_id),
        model=np.array(model),
    )


def load_reference(directory: Path, model: str) -> ReferenceSet | None:
    """The reference set in `directory`, or None (with the reason logged) when
    the file is missing, unreadable, or was embedded with another model: its
    vectors are only comparable with prompts from the same model."""
    path = Path(directory) / FILENAME
    if not path.is_file():
        log.info("No reference set at %s; mood bars will not be calibrated", path)
        return None
    try:
        with np.load(path, allow_pickle=False) as data:
            vectors = data["vectors"]
            set_id = str(data["set_id"])
            ref_model = str(data["model"])
    except Exception:
        log.exception("Reference set %s could not be read", path)
        return None
    if ref_model != model:
        log.warning("Reference set was built with %s, not %s; ignoring it", ref_model, model)
        return None
    if vectors.ndim != 2 or len(vectors) == 0:
        log.warning("Reference set %s has no vectors; ignoring it", path)
        return None
    return ReferenceSet(set_id, ref_model, _unit_rows(vectors))


def reference_stats(
    ref: ReferenceSet, prompt_vecs: dict[str, np.ndarray], mood_ids: list[str]
) -> dict:
    """Mean and population sd of each mood's raw score (and Energy's) over the
    reference tracks. `prompt_vecs` holds unit prompt vectors per mood id plus
    "energy+" and "energy-"; scores are computed as /score computes them: the
    mean cosine with a mood's prompts, and Energy as positive minus negative."""
    sims = {name: (ref.vectors @ v.T).mean(axis=1) for name, v in prompt_vecs.items()}
    raw = {mood_id: sims[mood_id] for mood_id in mood_ids}
    raw["energy"] = sims["energy+"] - sims["energy-"]
    return {
        "set": ref.set_id,
        "tracks": ref.tracks,
        "stats": {
            name: {"mean": float(s.mean()), "sd": float(s.std())} for name, s in raw.items()
        },
    }
