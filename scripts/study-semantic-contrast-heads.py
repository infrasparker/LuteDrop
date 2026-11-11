"""Search explicit positive-minus-negative semantic heads and calibrated ORs.

All scores use actual MuLan text/audio embeddings and the unchanged frozen
reference mapping. No labels are inferred from titles or album metadata.
Research selection uses the existing flags: it is not held-out validation.
"""
import argparse
import itertools
import json
from pathlib import Path
import re

import numpy as np
from scipy.special import expit


def unit(X):
    return X / np.maximum(np.linalg.norm(X, axis=1, keepdims=True), 1e-12)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory',type=Path)
    a=p.parse_args()
    d=json.loads((a.directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:X,R=unit(z['means'].astype(float)),unit(z['reference'].astype(float))
    with np.load(a.directory/'text-geometry.npz') as z:texts,T=z['texts'].tolist(),z['vectors'].astype(float)
    m=next(m for m in d['moods'] if m['id']=='grim');hs=d['hashes'];flags=m['flags'];idx=[hs.index(f['hash']) for f in flags]
    names={e['hash']:e['name'] for e in d['entries']}
    prior=np.array([m['scores'][h] for h in hs]);mapping=m['calibration']
    current=T[[texts.index(line) for line in m['prompts']]].mean(0)
    texts.append('CURRENT HORROR');T=np.vstack((T,current))
    pos=[i for i,t in enumerate(texts) if re.search(r'horror|sinister|ominous|dark|grim|dread|creep|haunt|spook|eerie|unsett|macabr|morbid|spectr|ghost|threat|ancient|cold|barren|desolat|eldritch|occult|mysteri|nightmare|ritual|demonic|forbidd|suspens|tense|discord|dissonan|unnerv|fear|terr|whisper|abyss|cavern|impending|brood|malevol',t.lower())]
    neg=[i for i,t in enumerate(texts) if re.search(r'calm|soft|sooth|peace|seren|tranquil|comfort|relax|gentle|tender|intimate|reassur|warm|heroic|triumph|victor|inspir|noble|glori|reveren|sacred|holy|divine|heaven|angel|devot|prayer|uplift',t.lower())]
    # Direct geometry revealed these as Kyne-like, not generic fear antonyms.
    neg += [texts.index(t) for t in ['sustained strings','warm pads','synth pads'] if t in texts]
    neg=list(dict.fromkeys(neg));P,N=T[pos],T[neg]
    PL,NL=X@P.T,X@N.T;PR,NR=R@P.T,R@N.T
    pmean,nmean=PR.mean(0),NR.mean(0);pv,nv=PR.var(0),NR.var(0)
    covariance=(PR-pmean).T@(NR-nmean)/len(R)
    lo=np.array([np.nan if f['min'] is None else f['min'] for f in flags]);hi=np.array([np.nan if f['max'] is None else f['max'] for f in flags])
    upper=np.flatnonzero(~np.isnan(hi));lower=np.flatnonzero(~np.isnan(lo));baselineHeld=np.nan_to_num(np.maximum(0,lo-prior[idx]))+np.nan_to_num(np.maximum(0,prior[idx]-hi))<1e-7
    best=[];masks={};count=0
    def record(ps,ns,strength,b):
        nonlocal count,best
        fv=b[:,idx]
        misses=np.nan_to_num(np.maximum(0,lo-fv))+np.nan_to_num(np.maximum(0,fv-hi))
        broken=misses>1e-7
        objective=misses.sum(1)+.15*np.abs(b-prior).mean(1)
        for k in np.argsort(objective)[:6]:
            best.append((float(objective[k]),ps[k],ns[k],float(strength),int((~broken[k]).sum())))
        best=sorted(best)[:30]
        for k in np.flatnonzero((fv[:,upper]<=hi[upper]+1e-7).all(1)):
            mask=sum(1<<j for j,p in enumerate(lower) if fv[k,p]>=lo[p]-1e-7)
            item=(float(objective[k]),ps[k],ns[k],float(strength),int((~broken[k]).sum()))
            pool=masks.setdefault(mask,[]);pool.append(item);pool.sort();del pool[4:]
        count+=len(b)
    for strength in [0.,.025,.05,.075,.1,.15,.2,.3,.4,.5,.65,.8,1.,1.25,1.5,2.,3.,4.]:
        for j in range(len(neg)) if strength else [-1]:
            scale=np.sqrt(np.maximum(pv if j<0 else pv+strength**2*nv[j]-2*strength*covariance[:,j],1e-20))
            raw=PL if j<0 else PL-strength*NL[:,j,None]
            mean=pmean if j<0 else pmean-strength*nmean[j]
            z=(raw-mean)/scale
            b=expit(mapping['slope']*(z.T-mapping['midpoint_z']))
            record(list(range(len(pos))),[j]*len(pos),strength,b)
    def head(item):
        objective,pi,ni,strength,held=item
        w=P[pi]-(0 if ni<0 else strength*N[ni])
        raw=X@w;ref=R@w;z=(raw-ref.mean())/ref.std()
        return {'positive':texts[pos[pi]],'negative':None if ni<0 else texts[neg[ni]],'strength':strength,'held':held,'objective':objective,'vector':w.tolist()},z,(ref-ref.mean())/ref.std()
    candidates=[];seen=set()
    for item in best+[v for group in masks.values() for v in group]:
        key=item[1:4]
        if key not in seen:seen.add(key);candidates.append(head(item))
    composites=[]
    for size in [1,2,3]:
        for group in itertools.combinations(range(len(candidates)),size):
            raw=np.maximum.reduce([candidates[i][1] for i in group]);ref=np.maximum.reduce([candidates[i][2] for i in group])
            b=expit(mapping['slope']*((raw-ref.mean())/ref.std()-mapping['midpoint_z']))
            fv=b[idx];misses=np.nan_to_num(np.maximum(0,lo-fv))+np.nan_to_num(np.maximum(0,fv-hi));held=misses<1e-7
            key=(int((baselineHeld&~held).sum()),-int(held.sum()),float(misses.sum()+.15*np.abs(b-prior).mean()),len(group))
            composites.append((key,group,b))
        composites=sorted(composites,key=lambda c:c[0])[:20]
    output={'examinedLinearHeads':count,'positiveTexts':len(pos),'negativeTexts':len(neg),'candidateHeads':len(candidates),'coverageMasks':list(masks),'results':[],
        'heads':[dict(h[0],flagScores=expit(mapping['slope']*(h[1][idx]-mapping['midpoint_z'])).tolist()) for h in candidates]}
    for key,group,b in composites[:10]:
        output['results'].append({'key':key,'heads':[candidates[i][0] for i in group],
            'flags':[dict(f,song=names[f['hash']],score=float(b[ix]),holds=bool((f['min'] is None or b[ix]>=f['min']-1e-7) and (f['max'] is None or b[ix]<=f['max']+1e-7))) for f,ix in zip(flags,idx)],
            'meanDrift':float(np.abs(b-prior).mean()),'saturated':int(((b<.05)|(b>.95)).sum()),'scores':dict(zip(hs,b.tolist()))})
    (a.directory/'semantic-contrast-heads-study.json').write_text(json.dumps(output,indent=2)+'\n',encoding='utf-8')
    print('HEADS',count,'candidates',len(candidates),'masks',len(masks))
    for c in output['results'][:3]:
        print('RESULT',c['key'],'drift',c['meanDrift'])
        print([(h['positive'],h['negative'],h['strength']) for h in c['heads']])
        print([(f['song'],round(f['score'],3),f['holds']) for f in c['flags']])


if __name__=='__main__':main()
