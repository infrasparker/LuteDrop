"""Run inside the analyzer container. Export cached vectors only; no model loads."""
import json
import sys
import numpy as np
from pathlib import Path

sys.path.insert(0, '/app')
from tagger.embed import EmbeddingCache

manifest = json.loads(Path(sys.argv[1]).read_text())
cache = EmbeddingCache(Path('/cache'))
hashes = manifest['hashes']
missing = [h for h in hashes if not cache.has(h)]
if missing:
    raise ValueError(f'{len(missing)} missing cached audio vectors')
means = np.vstack([cache.get(h) for h in hashes])
windows = [cache.get_windows(h) for h in hashes]
with np.load('/app/tagger/reference/reference.npz', allow_pickle=False) as ref:
    reference = ref['vectors'].copy()
np.savez_compressed(sys.argv[2], hashes=np.array(hashes), means=means, windows=np.vstack(windows),
                    offsets=np.cumsum([0] + [len(w) for w in windows]), reference=reference)
print('Exported', means.shape, 'window vectors', sum(map(len, windows)), 'reference', reference.shape)
