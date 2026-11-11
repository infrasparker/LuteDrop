"""Export existing cached reference windows for temporal scoring experiments."""
import json
from pathlib import Path
import numpy as np

manifest=json.loads(Path('/app/tagger/reference/manifest.json').read_text())
windows=[np.load(Path('/tmp/reference-cache')/(e['sha256']+'.npy')) for e in manifest]
np.savez_compressed('/work/reference-windows.npz',windows=np.vstack(windows),offsets=np.cumsum([0]+[len(w) for w in windows]))
print('Reference windows',sum(map(len,windows)))
