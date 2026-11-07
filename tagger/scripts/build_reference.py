"""Build tagger/reference/reference.npz from a manifest of audio files.

    python scripts/build_reference.py --manifest tagger/reference/manifest.json \
        --audio-dir <dir> --out tagger/reference/reference.npz [--download] [--cache-dir <dir>]

The manifest is a JSON list of {file, sha256, url?}. Every file's sha256 is
checked before embedding, so the set is exactly the audited one. Each track is
reduced the way EmbeddingCache.get reduces it: normalise the window
embeddings, average, normalise again. Needs the model (torch + muq).
"""

import argparse
import hashlib
import json
import shutil
import sys
import time
import urllib.request
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from tagger.embed import MuqEmbedder, load_windows  # noqa: E402
from tagger.reference import write_reference  # noqa: E402


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def reduce_windows(windows: np.ndarray) -> np.ndarray:
    norms = np.linalg.norm(windows, axis=1, keepdims=True)
    mean = (windows / np.where(norms == 0, 1, norms)).mean(axis=0)
    n = np.linalg.norm(mean)
    return mean / n if n else mean


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--manifest", type=Path, required=True)
    ap.add_argument("--audio-dir", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--set-id", default="incompetech-300-v1")
    ap.add_argument("--download", action="store_true", help="fetch missing files from each entry's url")
    ap.add_argument("--cache-dir", type=Path, help="reuse <sha256>.npy window embeddings from here")
    args = ap.parse_args()

    entries = json.loads(args.manifest.read_text())
    args.audio_dir.mkdir(parents=True, exist_ok=True)
    if args.cache_dir:
        args.cache_dir.mkdir(parents=True, exist_ok=True)

    embedder = MuqEmbedder()
    rows = []
    for i, entry in enumerate(entries, 1):
        path = args.audio_dir / entry["file"]
        if not path.is_file() and args.download:
            print(f"[{i}/{len(entries)}] downloading {entry['file']}")
            part = path.with_name(path.name + ".part")
            try:
                with urllib.request.urlopen(entry["url"], timeout=60) as resp, open(part, "wb") as f:
                    shutil.copyfileobj(resp, f)
                part.replace(path)
            finally:
                part.unlink(missing_ok=True)
            time.sleep(1)
        if not path.is_file():
            sys.exit(f"missing audio file: {path} (use --download?)")
        if sha256_of(path) != entry["sha256"]:
            sys.exit(f"sha256 mismatch for {entry['file']}; aborting")

        cached = args.cache_dir / f"{entry['sha256']}.npy" if args.cache_dir else None
        if cached and cached.is_file():
            windows = np.load(cached)
        else:
            print(f"[{i}/{len(entries)}] embedding {entry['file']}")
            windows = embedder.embed_windows(load_windows(str(path)))
            if cached:
                np.save(cached, np.asarray(windows, dtype=np.float32))
        rows.append(reduce_windows(np.asarray(windows, dtype=np.float64)))

    write_reference(args.out, args.set_id, MuqEmbedder.name, np.vstack(rows))
    print(f"wrote {len(rows)} tracks to {args.out}")


if __name__ == "__main__":
    main()
