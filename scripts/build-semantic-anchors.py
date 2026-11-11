"""Build fixed, aligned reference-audio anchors from the existing reference study."""
import hashlib,sys
from pathlib import Path
import numpy as np
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tagger'))
from tagger.personal import Evidence,audio_geometry
base=Path(sys.argv[1]);destination=Path(sys.argv[2])
reference=base/'active-reference.npz'
with np.load(reference,allow_pickle=False) as z:
    vectors=np.asarray(z['vectors'],float);set_id=str(z['set_id'])
with np.load(base/'preprojection-reference-features.npz',allow_pickle=False) as z:
    final=np.asarray(z['finals'],float);hidden=z['means'].copy()
vectors/=np.linalg.norm(vectors,axis=1,keepdims=True);final/=np.linalg.norm(final,axis=1,keepdims=True)
assert np.min(np.sum(vectors*final,axis=1))>.99999,'Reference rows are not aligned'
evidence=Evidence.load('tagger/tagger/reference')
np.savez_compressed(destination,audio=evidence.features(hidden),geometry=audio_geometry(hidden).astype('<f4'),evidence=np.array(evidence.id),reference_set=np.array(set_id),reference_sha256=np.array(hashlib.sha256(reference.read_bytes()).hexdigest()))
print('Wrote',len(vectors),'fixed reference anchors')
