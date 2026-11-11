"""Standardize signal features against the frozen reference, then test audio feedback."""
import json
from pathlib import Path
import sys
import numpy as np

base=Path(sys.argv[1])
with np.load(base/'acoustic-geometry.npz') as z:
    A,AR,names=z['features'].copy(),z['reference'].copy(),z['names'].tolist()
with np.load(base/'audio-geometry.npz') as z:X,R=z['means'].copy(),z['reference'].copy()
center=np.median(AR,axis=0);scale=np.std(AR,axis=0);valid=scale>1e-10
A=np.clip((A[:,valid]-center[valid])/scale[valid],-5,5);AR=np.clip((AR[:,valid]-center[valid])/scale[valid],-5,5)
def unit(x):return x/np.maximum(np.linalg.norm(x,axis=1,keepdims=True),1e-12)
A,AR=unit(A),unit(AR)
data=json.loads((base/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
for name,means,reference in [('acoustic-case',A,AR),('hybrid-acoustic-case',np.column_stack((unit(X),A)),np.column_stack((unit(R),AR)))]:
    directory=base/name;directory.mkdir(exist_ok=True)
    np.savez_compressed(directory/'audio-geometry.npz',means=unit(means),reference=unit(reference))
    (directory/'audio-geometry-snapshot.json').write_text(json.dumps(data)+'\n',encoding='utf-8')
print('Prepared acoustic and hybrid tests:',sum(valid),'signal features')
