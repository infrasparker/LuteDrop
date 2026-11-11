"""Independent signal features: tonality, chromatic tension, timbre and dynamics.

Read-only audio input. No pretrained semantic encoder, titles or metadata used.
Features summarize the same eight 10-second windows as the production model.
"""
import os
os.environ['OMP_NUM_THREADS']='1'
os.environ['OPENBLAS_NUM_THREADS']='1'
import json
from pathlib import Path
import sys
from concurrent.futures import ThreadPoolExecutor

import numpy as np
from scipy.signal import resample_poly

sys.path.insert(0,'/app')
from tagger.embed import load_windows

work=Path('/work')
manifest=json.loads((work/'clap-manifest.json').read_text())
refmanifest=json.loads(Path('/app/tagger/reference/manifest.json').read_text())
items=[(e['hash'],e['path']) for e in manifest['entries']]+[(e['sha256'],str(Path('/reference-audio')/e['file'])) for e in refmanifest]
freq=np.fft.rfftfreq(2048,1/12000)
valid=(freq>=55)&(freq<=5000)
pitch=69+12*np.log2(np.maximum(freq[valid],1e-9)/440)
pc=np.round(pitch).astype(int)%12
weights=np.maximum(0,np.cos(np.pi*(pitch-np.round(pitch))))**2
chromamap=np.zeros((12,len(freq)))
for c in range(12):chromamap[c,np.flatnonzero(valid)[pc==c]]=weights[pc==c]
major=np.array([6.35,2.23,3.48,2.33,4.38,4.09,2.52,5.19,2.39,3.66,2.29,2.88])
minor=np.array([6.33,2.68,3.52,5.38,2.60,3.53,2.54,4.75,3.98,2.69,3.34,3.17])
profiles=np.array([np.roll(p,i) for p in [major,minor] for i in range(12)])
profiles-=profiles.mean(1,keepdims=True);profiles/=np.linalg.norm(profiles,axis=1,keepdims=True)

def features(item):
    h,path=item
    wav=resample_poly(load_windows(path),1,2,axis=1).reshape(-1)
    frames=np.lib.stride_tricks.sliding_window_view(wav,2048)[::512]
    mag=np.abs(np.fft.rfft(frames*np.hanning(2048),axis=1))+1e-10
    power=mag**2;energy=power.sum(1);distribution=power/energy[:,None]
    centroid=distribution@freq
    bandwidth=np.sqrt((distribution*(freq[None,:]-centroid[:,None])**2).sum(1))
    flatness=np.exp(np.log(power).mean(1))/power.mean(1)
    entropy=-(distribution*np.log(distribution+1e-12)).sum(1)/np.log(len(freq))
    rms=np.sqrt((frames**2).mean(1))
    flux=np.r_[0,np.maximum(0,np.diff(np.sqrt(distribution),axis=0)).mean(1)]
    chroma=power@chromamap.T;chroma/=np.maximum(chroma.sum(1,keepdims=True),1e-12)
    cent=chroma-chroma.mean(1,keepdims=True)
    key=cent@profiles.T/np.maximum(np.linalg.norm(cent,axis=1,keepdims=True),1e-12)
    maj=key[:,:12].max(1);mino=key[:,12:].max(1)
    chromaentropy=-(chroma*np.log(chroma+1e-12)).sum(1)/np.log(12)
    seq=[centroid,bandwidth,flatness,entropy,rms,flux,maj,mino,maj-mino,chromaentropy,chroma.max(1)]
    labels=['spectral-centroid','spectral-bandwidth','spectral-flatness','spectral-entropy','rms','spectral-flux','major-key-fit','minor-key-fit','major-minus-minor','pitch-entropy','pitch-dominance']
    for interval in range(1,7):
        seq.append((chroma*np.roll(chroma,interval,axis=1)).sum(1));labels.append('pitch-interval-'+str(interval))
    for low,high in [(0,120),(120,300),(300,1000),(1000,3000),(3000,6001)]:
        seq.append(distribution[:,(freq>=low)&(freq<high)].sum(1));labels.append(f'energy-{low}-{high}hz')
    result=[];names=[]
    for label,values in zip(labels,seq):
        for suffix,v in [('mean',values.mean()),('sd',values.std()),('q10',np.quantile(values,.1)),('q90',np.quantile(values,.9))]:
            result.append(float(v));names.append(label+'/'+suffix)
    return h,result,names

rows=[];hashes=[]
with ThreadPoolExecutor(4) as pool:
    for i,(h,values,names) in enumerate(pool.map(features,items)):
        rows.append(values);hashes.append(h)
        if i%20==0:print('ACOUSTICS',i+1,'/',len(items),flush=True)
np.savez_compressed(work/'acoustic-geometry.npz',features=np.array(rows[:len(manifest['entries'])]),reference=np.array(rows[len(manifest['entries']):]),hashes=np.array(hashes[:len(manifest['entries'])]),names=np.array(names))
print('DONE',len(rows),len(names),flush=True)
