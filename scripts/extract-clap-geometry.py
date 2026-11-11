"""Independent LAION CLAP music model on the same song windows/reference audio.

Run in an ephemeral research container. Never touches production embeddings.
Weights are the published laion/larger_clap_music checkpoint; no tuning on flags.
"""
import json
from pathlib import Path
import sys
import time

import numpy as np
from scipy.signal import resample_poly
import torch
from transformers import ClapModel, ClapProcessor

sys.path.insert(0,'/app')
from tagger.embed import load_windows

work=Path('/work')
manifest=json.loads((work/'clap-manifest.json').read_text())
torch.set_num_threads(4)
model=ClapModel.from_pretrained('laion/larger_clap_music').eval().to('cuda')
processor=ClapProcessor.from_pretrained('laion/larger_clap_music')
def normalized(X):return X/np.maximum(np.linalg.norm(X,axis=1,keepdims=True),1e-12)
def audio(path):
    wavs=load_windows(path)
    wavs=resample_poly(wavs,2,1,axis=1).astype(np.float32)
    vectors=[]
    for off in range(0,len(wavs),4):
        inp=processor(audios=list(wavs[off:off+4]),sampling_rate=48000,return_tensors='pt').to('cuda')
        with torch.inference_mode():vectors.extend(model.get_audio_features(**inp).cpu().numpy())
    mean=normalized(np.array(vectors)).mean(0)
    return mean/max(np.linalg.norm(mean),1e-12)
texts=manifest['texts'];textvec=[]
for t in texts:
    inp=processor(text=[t],return_tensors='pt').to('cuda')
    with torch.inference_mode():textvec.extend(model.get_text_features(**inp).cpu().numpy())
print('TEXTS',len(textvec),flush=True)
library=[];hashes=[];reference=[]
start=time.monotonic()
for i,e in enumerate(manifest['entries']):
    library.append(audio(e['path']));hashes.append(e['hash'])
    if i%10==0:print('LIBRARY',i+1,'/',len(manifest['entries']),'seconds',round(time.monotonic()-start),flush=True)
    np.savez_compressed(work/'clap-library.npz',means=np.array(library),hashes=np.array(hashes),texts=np.array(texts),vectors=np.array(textvec))
refmanifest=json.loads(Path('/app/tagger/reference/manifest.json').read_text())
for i,e in enumerate(refmanifest):
    reference.append(audio(str(Path('/reference-audio')/e['file'])))
    if i%20==0:print('REFERENCE',i+1,'/',len(refmanifest),'seconds',round(time.monotonic()-start),flush=True)
    np.savez_compressed(work/'clap-reference.npz',reference=np.array(reference))
print('DONE',len(library),len(reference),round(time.monotonic()-start),flush=True)
