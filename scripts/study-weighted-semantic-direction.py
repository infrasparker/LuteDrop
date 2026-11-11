"""Try constrained, interpretable positive/negative text mixtures with fixed calibration."""
import argparse
import json
from pathlib import Path
import numpy as np
from scipy.optimize import minimize
from scipy.special import expit,logit


def unit(X):return X/np.maximum(np.linalg.norm(X,axis=1,keepdims=True),1e-12)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:X,R=unit(z['means'].astype(float)),unit(z['reference'].astype(float))
    with np.load(a.directory/'text-geometry.npz') as z:texts,T=z['texts'].tolist(),unit(z['vectors'].astype(float))
    m=next(m for m in d['moods'] if m['id']=='grim');hs=d['hashes'];flags=m['flags'];idx=[hs.index(f['hash']) for f in flags];mapping=m['calibration']
    names={e['hash']:e['name'] for e in d['entries']}
    positive=['sinister whispers','a creeping horror','an ancient mystery','a haunted house','a horror movie','creeping dread','spooky','uncanny','eldritch','dread','ominous','horror','haunted','eerie','bleak','dissonant','a dark ritual','secrets in the dark','a cursed place','supernatural','demonic','cavernous','desolate','barren','spectral','grim','ghostly','foreboding','dark','creepy','unsettling','menacing','an ominous presence','a graveyard at night','abyssal','ominous, desolate, cavernous','cold, desolate, barren, hollow, ominous music','ancient, spectral, ominous, solemn, cold instrumental music']
    negative=['soft and quiet','soft and soothing','tender, intimate, inspiring','warm pads','sustained strings','a sacred choir','heroic','triumphant','reverent','intimate','calm and sparse','a noble quest','awe and wonder','soft flute, sentimental','synth pads','solo cello','peaceful and relaxing','gentle','tender','comforting','calm']
    current=T[[texts.index(t) for t in m['prompts']]].mean(0)
    terms=[('CURRENT HORROR',1,current)]+[(t,1,T[texts.index(t)]) for t in positive if t in texts]+[(t,-1,T[texts.index(t)]) for t in negative if t in texts]
    rawB=np.array([v*sign for _,sign,v in terms]).T
    scales=(R@rawB).std(0)
    B=rawB/scales
    mu=R.mean(0);Z=(X-mu)@B;Zr=(R-mu)@B;C=Zr.T@Zr/len(R)
    q0=np.zeros(len(terms));q0[0]=1.
    prior=expit(mapping['slope']*(Z@q0-mapping['midpoint_z']))
    lower=np.array([-np.inf if f['min'] is None else mapping['midpoint_z']+logit(min(.999,f['min']+.01))/mapping['slope'] for f in flags])
    upper=np.array([np.inf if f['max'] is None else mapping['midpoint_z']+logit(max(.001,f['max']-.01))/mapping['slope'] for f in flags])
    report={'terms':[(t,s) for t,s,_ in terms],'models':[]}
    def solve(train,sparsity):
        A=Z[np.array(idx)[train]];lo,hi=lower[train],upper[train]
        il,iu=np.flatnonzero(np.isfinite(lo)),np.flatnonzero(np.isfinite(hi))
        F=np.vstack((A[il],-A[iu]));target=np.r_[lo[il],-hi[iu]]
        loose=np.ones(len(X),dtype=bool);loose[np.array(idx)[train]]=False
        H=Z[loose].T@Z[loose]/loose.sum()+.1*C
        penalty=np.r_[0.,np.full(len(terms)-1,sparsity)]
        result=minimize(lambda q:.5*(q-q0)@H@(q-q0)+penalty@q,q0.copy(),jac=lambda q:H@(q-q0)+penalty,
            bounds=[(0,None)]*len(terms),constraints=[{'type':'eq','fun':lambda q:q@C@q-1,'jac':lambda q:2*C@q},
                {'type':'ineq','fun':lambda q:F@q-target,'jac':lambda q:F}],
            method='SLSQP',options={'maxiter':500,'ftol':1e-10})
        ok=bool(abs(result.x@C@result.x-1)<1e-5 and (F@result.x-target).min()>=-1e-5)
        actual_sd=np.sqrt(max(result.x@C@result.x,1e-20))
        return result.x,expit(mapping['slope']*(Z@result.x/actual_sd-mapping['midpoint_z'])),ok,result.message
    for sparsity in [0.,.001,.01,.1]:
        q,after,ok,message=solve(list(range(len(flags))),sparsity)
        components=[{'text':t,'sign':sign,'coefficient':float(v),'rawCoefficient':float(v/scale)} for (t,sign,_),v,scale in zip(terms,q,scales) if v>1e-5]
        components.sort(key=lambda c:-c['coefficient'])
        item={'sparsity':sparsity,'feasible':ok,'message':str(message),'components':components,'vector':(B@q).tolist(),'meanDrift':float(np.abs(after-prior).mean()),
            'scores':dict(zip(hs,after.tolist())),'flags':[dict(song=names[f['hash']],score=float(after[i]),holds=bool((f['min'] is None or after[i]>=f['min']-1e-6) and (f['max'] is None or after[i]<=f['max']+1e-6))) for f,i in zip(flags,idx)]}
        report['models'].append(item)
        (a.directory/'weighted-semantic-direction-study.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
        print('SEMANTIC',sparsity,ok,'held',sum(f['holds'] for f in item['flags']),'components',len(components),'drift',item['meanDrift'],flush=True)
        print([(c['text'],c['sign'],round(c['coefficient'],3)) for c in components[:12]],flush=True)


if __name__=='__main__':main()
