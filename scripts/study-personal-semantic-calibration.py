"""Joint signed semantic search and bounded monotone, example-based calibration.

Changes the interpretation from reference rarity to personal mood fit. A curve
has fixed endpoints, is monotone and has slope at most 3, preventing arbitrary
spikes and the previous steep sigmoid collapse. Full outer leave-one-song-out
repeats word selection and curve fitting without the held label.
"""
import argparse
import json
from pathlib import Path
import re
import time

import numpy as np
from scipy.optimize import minimize, LinearConstraint
from scipy.special import expit


def unit(X):return X/np.maximum(np.linalg.norm(X,axis=1,keepdims=True),1e-12)


def interpolate_matrix(values,knots):
    idx=np.clip(np.searchsorted(knots,values,side='right')-1,0,len(knots)-2)
    fraction=(values-knots[idx])/(knots[idx+1]-knots[idx])
    M=np.zeros((len(values),len(knots)));rows=np.arange(len(values))
    M[rows,idx]=1-fraction;M[rows,idx+1]=fraction
    return M


def curve(base,prior,flags,train,max_slope=3.,min_slope=.1,preferences=()):
    idx=[flags[i]['index'] for i in train]
    knots=np.unique(np.r_[0.,.25,.5,.75,1.,base[idx]])
    lo=np.zeros(len(knots));hi=np.ones(len(knots));lo[-1]=hi[-1]=1;lo[0]=hi[0]=0
    for i in train:
        f=flags[i];j=np.searchsorted(knots,base[f['index']])
        if f['min'] is not None:lo[j]=max(lo[j],min(1.,f['min']+.03))
        if f['max'] is not None:hi[j]=min(hi[j],max(0.,f['max']-.03))
    if (lo>hi).any():return None
    M=interpolate_matrix(base,knots)
    H=M.T@M/len(base)+.01*np.eye(len(knots));target=M.T@prior/len(base)+.01*knots
    D=np.diff(np.eye(len(knots)),axis=0)
    constraints=[LinearConstraint(D,min_slope*np.diff(knots),max_slope*np.diff(knots))]
    for preference in preferences:
        positions=[preference['higherIndex'],preference['lowerIndex']]
        if not all(i in idx for i in positions):continue
        pair=interpolate_matrix(base[positions],knots)
        constraints.append(LinearConstraint((pair[0]-pair[1])[None,:],[preference['trainingMargin']],[np.inf]))
    result=minimize(lambda y:.5*y@H@y-target@y,np.clip(knots,lo,hi),jac=lambda y:H@y-target,
        bounds=list(zip(lo,hi)),constraints=constraints,
        method='SLSQP',options={'ftol':1e-11,'maxiter':200})
    if not result.success or (D@result.x < min_slope*np.diff(knots)-1e-7).any() or (D@result.x>max_slope*np.diff(knots)+1e-7).any():return None
    return M@result.x,knots,result.x


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:X,R=unit(z['means'].astype(float)),unit(z['reference'].astype(float))
    with np.load(a.directory/'text-geometry.npz') as z:texts,T=z['texts'].tolist(),unit(z['vectors'].astype(float))
    m=next(m for m in d['moods'] if m['id']=='grim');hs=d['hashes'];mapping=m['calibration'];flags=[dict(f,index=hs.index(f['hash'])) for f in m['flags'] if f['fresh']]
    names={e['hash']:e['name'] for e in d['entries']};prior=np.array([m['scores'][h] for h in hs])
    preferences=[dict(pref,higherIndex=hs.index(pref['higher']),lowerIndex=hs.index(pref['lower'])) for pref in d.get('orderingPreferences',[])]
    current=T[[texts.index(t) for t in m['prompts']]].mean(0);texts.append('CURRENT HORROR');T=np.vstack([T,current])
    pos=[i for i,t in enumerate(texts) if re.search(r'horror|sinister|ominous|dark|grim|dread|creep|haunt|spook|eerie|unsett|macabr|morbid|spectr|ghost|threat|ancient|cold|barren|desolat|eldritch|occult|mysteri|nightmare|ritual|demonic|forbidd|suspens|tense|discord|dissonan|unnerv|fear|terr|whisper|abyss|cavern|impending|brood|malevol',t.lower())]
    neg=[i for i,t in enumerate(texts) if re.search(r'calm|soft|sooth|peace|seren|tranquil|comfort|relax|gentle|tender|intimate|reassur|warm|heroic|triumph|victor|inspir|noble|glori|reveren|sacred|holy|divine|heaven|angel|devot|prayer|uplift',t.lower())]
    P,N=T[pos],T[neg];PL,NL=X@P.T,X@N.T;PR,NR=R@P.T,R@N.T
    pm,nm=PR.mean(0),NR.mean(0);pv,nv=PR.var(0),NR.var(0);cov=(PR-pm).T@(NR-nm)/len(R)
    matrices=[];specs=[]
    for strength in [0.,.05,.1,.2,.3,.5,.65,.8,1.,1.25,1.5,2.,3.,4.]:
        for j in range(len(neg)) if strength else [-1]:
            raw=PL if j<0 else PL-strength*NL[:,j,None]
            mean=pm if j<0 else pm-strength*nm[j]
            sd=np.sqrt(np.maximum(pv if j<0 else pv+strength**2*nv[j]-2*strength*cov[:,j],1e-20))
            matrices.append(expit(mapping['slope']*((raw-mean).T/sd[:,None]-mapping['midpoint_z'])).astype(np.float32))
            specs.extend((pos[i],None if j<0 else neg[j],strength) for i in range(len(pos)))
    B=np.vstack(matrices);idx=np.array([f['index'] for f in flags]);fv=B[:,idx]
    lower=np.array([0 if f['min'] is None else min(1.,f['min']+.03) for f in flags]);upper=np.array([1 if f['max'] is None else max(0.,f['max']-.03) for f in flags])
    drift=np.mean(np.abs(B-prior),axis=1)
    count=len(B)
    def choose(train):
        v=fv[:,train];l,u=lower[train],upper[train]
        valid=np.ones(count,dtype=bool)
        for j in range(len(train)):
            valid&=(l[j]<=3*v[:,j]+1e-7)&(1-u[j]<=3*(1-v[:,j])+1e-7)
            for k in range(len(train)):
                difference=v[:,j]-v[:,k]
                valid&=(l[j]-u[k]<=3*np.maximum(difference,0)+.1*np.minimum(difference,0)+1e-7)
        train_rows={flags[i]['index'] for i in train}
        for pref in preferences:
            if pref['higherIndex'] in train_rows and pref['lowerIndex'] in train_rows:
                valid&=3*(B[:,pref['higherIndex']]-B[:,pref['lowerIndex']])>=pref['trainingMargin']-1e-7
        eligible=np.flatnonzero(valid)
        if not len(eligible):return None
        # Shortlist by distance from the existing lens; calibration then minimizes
        # the same library movement. The held-out label is never read here.
        shortlist=eligible[np.argsort(drift[eligible])[:100]]
        best=None
        for row in shortlist:
            calibrated=curve(B[row].astype(float),prior,flags,train,preferences=preferences)
            if calibrated is None:continue
            after,knots,values=calibrated
            cost=float(np.abs(after-prior).mean())+.002*(specs[row][1] is not None)
            item=(cost,int(row),after,knots,values,len(eligible))
            if best is None or item[:2]<best[:2]:best=item
        return best
    def describe(result):
        cost,row,after,knots,values,eligible=result;positive,negative,strength=specs[row]
        return {'positive':texts[positive],'negative':None if negative is None else texts[negative],'strength':strength,
                'eligible':eligible,'cost':cost,'knots':knots.tolist(),'values':values.tolist(),'meanDrift':float(np.abs(after-prior).mean()),
                'saturated':int(((after<.05)|(after>.95)).sum()),'scores':dict(zip(hs,after.tolist())),
                'flags':[dict(song=names[f['hash']],score=float(after[f['index']]),min=f['min'],max=f['max'],holds=bool((f['min'] is None or after[f['index']]>=f['min']-1e-7) and (f['max'] is None or after[f['index']]<=f['max']+1e-7))) for f in flags]}
    full=choose(list(range(len(flags))))
    report={'examined':len(B),'mapping':'personal bounded monotone fit, not reference percentile','maxSlope':3.,'minSlope':.1,'full':None if full is None else describe(full),'outerLeaveOneOut':[]}
    report['orderingPreferences']=d.get('orderingPreferences',[])
    if full is not None:print('FULL',report['full']['positive'],report['full']['negative'],report['full']['strength'],'drift',report['full']['meanDrift'],'saturated',report['full']['saturated'],report['full']['flags'],flush=True)
    for held in range(len(flags)):
        best=choose([i for i in range(len(flags)) if i!=held])
        if best is None:continue
        model=describe(best);v=model['scores'][flags[held]['hash']];f=flags[held]
        item={'song':names[f['hash']],'value':v,'holds':bool((f['min'] is None or v>=f['min']-1e-7) and (f['max'] is None or v<=f['max']+1e-7)),
              'positive':model['positive'],'negative':model['negative'],'strength':model['strength']}
        report['outerLeaveOneOut'].append(item);print('OUTER LOO',item,flush=True)
    (a.directory/'personal-semantic-calibration-study.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')


if __name__=='__main__':main()
