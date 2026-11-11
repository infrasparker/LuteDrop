"""Test whether track averaging hides semantic moments, using actual cached windows."""
import argparse
import json
from pathlib import Path
import numpy as np
from scipy.special import expit


def unit(x):return x/np.maximum(np.linalg.norm(x,axis=1,keepdims=True),1e-12)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:W,off=unit(z['windows'].astype(float)),z['offsets'].copy()
    with np.load(a.directory/'reference-windows.npz') as z:RW,roff=unit(z['windows'].astype(float)),z['offsets'].copy()
    with np.load(a.directory/'text-geometry.npz') as z:texts,T=z['texts'].tolist(),z['vectors'].astype(float)
    m=next(m for m in d['moods'] if m['id']=='grim');hs=d['hashes'];flags=m['flags'];ix=[hs.index(f['hash']) for f in flags]
    names={e['hash']:e['name'] for e in d['entries']};prior=np.array([m['scores'][h] for h in hs]);mapping=m['calibration']
    T=np.vstack([T,T[[texts.index(t) for t in m['prompts']]].mean(0)]);texts.append('CURRENT HORROR')
    U=W@T.T;RU=RW@T.T
    lower=np.array([np.nan if f['min'] is None else f['min'] for f in flags]);upper=np.array([np.nan if f['max'] is None else f['max'] for f in flags])
    oldheld=np.nan_to_num(np.maximum(0,lower-prior[ix]))+np.nan_to_num(np.maximum(0,prior[ix]-upper))<1e-7
    results=[]
    for pooling,fn in [('window mean',lambda x:x.mean(0)),('window median',lambda x:np.median(x,axis=0)),
         ('window 75th percentile',lambda x:np.quantile(x,.75,axis=0)),('window 90th percentile',lambda x:np.quantile(x,.9,axis=0)),
         ('window maximum',lambda x:x.max(0)),('window mean + sd',lambda x:x.mean(0)+x.std(0)),
         ('window mean - sd',lambda x:x.mean(0)-x.std(0)),('window variation',lambda x:x.std(0))]:
        raw=np.array([fn(U[off[i]:off[i+1]]) for i in range(len(hs))])
        ref=np.array([fn(RU[roff[i]:roff[i+1]]) for i in range(len(roff)-1)])
        bars=expit(mapping['slope']*((raw-ref.mean(0))/np.maximum(ref.std(0),1e-12)-mapping['midpoint_z']))
        for j,text in enumerate(texts):
            b=bars[:,j];fv=b[ix];miss=np.nan_to_num(np.maximum(0,lower-fv))+np.nan_to_num(np.maximum(0,fv-upper));held=miss<1e-7
            results.append({'text':text,'pooling':pooling,'held':int(held.sum()),'newBreaks':int((oldheld&~held).sum()),'miss':float(miss.sum()),'meanDrift':float(np.abs(b-prior).mean()),
                'flagScores':[dict(song=names[f['hash']],score=float(v),holds=bool(h)) for f,v,h in zip(flags,fv,held)],'scores':dict(zip(hs,b.tolist()))})
    results.sort(key=lambda c:(c['newBreaks'],-c['held'],c['miss'],c['meanDrift']))
    report={'examined':len(results),'best':results[:20], 'currentComparisons':[r for r in results if r['text']=='CURRENT HORROR'],
        'focusWindows':[dict(song=names[f['hash']],currentCosines=U[off[i]:off[i+1],-1].tolist(),meanCoherence=float(np.linalg.norm(W[off[i]:off[i+1]].mean(0)))) for f,i in zip(flags,ix)]}
    (a.directory/'temporal-mood-pooling-study.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    for r in results[:5]:print(r['held'],r['newBreaks'],r['pooling'],r['text'],[(f['song'],round(f['score'],3)) for f in r['flagScores']])
    print('CURRENT',[(r['pooling'],r['held'],[(f['song'],round(f['score'],3)) for f in r['flagScores']]) for r in report['currentComparisons']])


if __name__=='__main__':main()
