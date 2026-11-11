"""Window-resampling stability of a frozen scorer. Does not fit or modify it."""
import json
import os
from pathlib import Path
import sys

import numpy as np
from scipy.special import expit
import torch

sys.path.insert(0,'/app')
from tagger.embed import MuqEmbedder,load_windows

work=Path('/work');profile=json.loads((work/'frozen-two-evidence-horror.json').read_text())
manifest=json.loads((work/'clap-manifest.json').read_text());selected=set(profile['allExcludedHashes'])
embedder=MuqEmbedder();embedder.load();model=embedder._model;audio=model.mulan.audio
torch.set_num_threads(2);rng=np.random.default_rng(20261109)
lh,rh=profile['libraryHead'],profile['referenceHead'];head=np.array(lh['coefficient']);refhead=np.array(rh['coefficient'])
knots=profile['personalScale']['input'];values=profile['personalScale']['output']
def unit(x):return x/max(np.linalg.norm(x),1e-12)
def score(H,F):
    style=expit(lh['calibration'][0]*(unit(H)@head+lh['intercept'])+lh['calibration'][1])
    unnerving=expit(rh['calibration'][0]*(unit(F)@refhead+rh['intercept'])+rh['calibration'][1])
    return float(np.interp(np.sqrt(style*unnerving),knots,values))
results=[]
for e in manifest['entries']:
    if e['hash'] not in selected:continue
    hiddenmeans=[];final=[];wavs=load_windows(e['path'])
    for off in range(0,len(wavs),2):
        x=torch.tensor(wavs[off:off+2],dtype=torch.float32,device='cuda')
        with torch.inference_mode():
            states=audio.model(x,output_hidden_states=True).hidden_states
            hiddenmeans.extend(states[9].mean(dim=1).cpu().numpy())
            projected=audio.proj(states[-1]);projected,_=audio.transformer(projected,return_all_layers=True)
            latent=model.mulan.audio_to_latents(projected.mean(dim=-2));latent/=torch.linalg.norm(latent,dim=-1,keepdim=True)
            final.extend(latent.cpu().numpy())
    H,F=np.array(hiddenmeans),np.array(final)
    full=score(H.mean(0),F.mean(0))
    samples=[score(H[rows].mean(0),F[rows].mean(0)) for rows in rng.integers(0,len(H),size=(256,len(H)))]
    results.append({'hash':e['hash'],'original':profile['scores'][e['hash']],'fresh':full,'bootstrap5':float(np.quantile(samples,.05)),
        'bootstrap50':float(np.quantile(samples,.5)),'bootstrap95':float(np.quantile(samples,.95)),
        'sampling':'Resample existing 10-second windows with replacement, 256 deterministic draws; not a correctness confidence interval'})
    print('STABILITY',len(results),'/',len(selected),flush=True)
(work/'frozen-horror-window-stability.json').write_text(json.dumps({'results':results},indent=2)+'\n')
print('DONE: frozen scorer unchanged',flush=True)
