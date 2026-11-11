"""Read hidden music features before MuLan's 512-dimensional text projection.

Transient research only; reuse frozen published weights, no training, no model
or cache changes. Also reconstruct fresh final embeddings to audit cache parity.
"""
import os
os.environ['OMP_NUM_THREADS']='2'
import json
from pathlib import Path
import sys
import time

import numpy as np
import torch

sys.path.insert(0,'/app')
from tagger.embed import MuqEmbedder,load_windows

work=Path('/work');manifest=json.loads((work/'clap-manifest.json').read_text())
reference_only='--reference' in sys.argv
if reference_only:
    reference=json.loads(Path('/app/tagger/reference/manifest.json').read_text())
    manifest={'entries':[{'hash':e['sha256'],'path':str(Path('/reference-audio')/e['file'])} for e in reference]}
output_file=work/('preprojection-reference-features.npz' if reference_only else 'preprojection-features.npz')
torch.set_num_threads(2)
embedder=MuqEmbedder();embedder.load();model=embedder._model
audio=model.mulan.audio
layers=[3,6,9,12]
means=[];stds=[];finals=[];hashes=[]
start=time.monotonic()
for i,e in enumerate(manifest['entries']):
    wavs=load_windows(e['path']);permean=[];persecond=[];final=[]
    for off in range(0,len(wavs),2):
        x=torch.tensor(wavs[off:off+2],dtype=torch.float32,device='cuda')
        with torch.inference_mode():
            hidden=audio.model(x,output_hidden_states=True).hidden_states
            if i==0 and off==0:print('HIDDEN LAYERS',len(hidden),'shape',tuple(hidden[-1].shape),flush=True)
            selected=torch.stack([hidden[k] for k in layers],dim=1)
            permean.extend(selected.mean(dim=2).cpu().numpy())
            persecond.extend((selected**2).mean(dim=2).cpu().numpy())
            projected=audio.proj(hidden[audio.use_layer_idx])
            projected,_=audio.transformer(projected,return_all_layers=True)
            latent=model.mulan.audio_to_latents(projected.mean(dim=-2))
            latent=latent/torch.linalg.norm(latent,dim=-1,keepdim=True)
            final.extend(latent.cpu().numpy())
    mean=np.array(permean).mean(axis=0);std=np.sqrt(np.maximum(np.array(persecond).mean(axis=0)-mean**2,0))
    means.append(mean);stds.append(std);hashes.append(e['hash'])
    f=np.array(final).mean(0);finals.append(f/np.linalg.norm(f))
    np.savez_compressed(output_file,means=np.array(means),stds=np.array(stds),finals=np.array(finals),hashes=np.array(hashes),layers=np.array(layers))
    if i%10==0:print('PREPROJECTION',i+1,'/',len(manifest['entries']),'seconds',round(time.monotonic()-start),flush=True)
print('DONE',len(means),'seconds',round(time.monotonic()-start),flush=True)
