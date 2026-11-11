"""Small constrained graded learner over independent, named emotion evidence.

Unlike prior embedding-direction fits, this has eight interpretable inputs.
The emotion heads never see these songs or judgments. Regularization is chosen
inside each leave-one-song-out fold; comparisons touching the held song are
removed. No claim of external validation until another frozen blind check.
"""
import argparse
import json
from pathlib import Path
import time

import numpy as np
from scipy.optimize import minimize


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);p.add_argument('--include-style-evidence',action='store_true');a=p.parse_args()
    d=json.loads((a.directory/'ten-label-case/audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    evidence=json.loads((a.directory/'publisher-audio-evidence.json').read_text(encoding='utf-8'))
    if a.include_style_evidence:
        original=json.loads((a.directory/'frozen-two-evidence-horror.json').read_text(encoding='utf-8'))
        evidence['heads'].append({'label':'Historical fear style with unnerving evidence','scores':{e['song']:e['combined'] for e in original['evidenceScores']}})
    hs=d['hashes'];byname={e['name']:e for e in d['entries']};names={e['hash']:e['name'] for e in d['entries']};mood=next(m for m in d['moods'] if m['id']=='grim')
    flags=[dict(f,index=hs.index(f['hash'])) for f in mood['flags']]
    for name,lo,hi,side in [('Wind from the Depths',.75,None,'high'),('Burial Isle',.75,.9,'mid'),('Far Horizons',None,.5,'low'),('The Cove Battle',.75,None,'high')]:
        flags.append({'hash':byname[name]['hash'],'index':hs.index(byname[name]['hash']),'min':lo,'max':hi,'side':side,'weight':1.,'fresh':True})
    comparisons=[(hs.index(byname['Forlorn Cliffs']['hash']),hs.index(byname[lower]['hash'])) for lower in ['Temple of Woedica','Burial Isle']]
    labels=[head['label'] for head in evidence['heads']]
    if a.include_style_evidence:
        evidence['heads'][-1]['scores']={byname[name]['hash']:score for name,score in evidence['heads'][-1]['scores'].items()}
    X=np.column_stack([np.ones(len(hs))]+[np.array([head['scores'][h] for h in hs]) for head in evidence['heads']])
    bounds=[(-1,1)]+[(0,None) if label in ['Eerie','Unnerving','Suspenseful','Dark','Historical fear style with unnerving evidence'] else (None,0) if label in ['Calming','Relaxed','Uplifting'] else (None,None) for label in labels]
    grid=[.00001,.0001,.001,.01,.1,1.]
    cache={}
    def fit(train,reg):
        key=(tuple(train),reg)
        if key in cache:return cache[key]
        ix=np.array([flags[i]['index'] for i in train]);A=X[ix]
        lo=np.array([0 if flags[i]['min'] is None else min(1.,flags[i]['min']+.03) for i in train]);hi=np.array([1 if flags[i]['max'] is None else max(0.,flags[i]['max']-.03) for i in train])
        pairs=[X[high]-X[low] for high,low in comparisons if high in ix and low in ix];P=np.array(pairs).reshape((-1,X.shape[1]))
        def objective(w):
            pred=A@w;below=np.minimum(pred-lo,0);above=np.maximum(pred-hi,0)
            objective=np.mean(below**2+above**2)+reg*(w[1:]@w[1:]+.1*w[0]**2)
            grad=2*A.T@(below+above)/len(train)+2*reg*np.r_[.1*w[0],w[1:]]
            if len(P):
                margin=np.minimum(P@w-.03,0);objective+=np.mean(margin**2);grad+=2*P.T@margin/len(P)
            return float(objective),grad
        initial=np.zeros(X.shape[1]);initial[0]=.2
        res=minimize(objective,initial,jac=True,bounds=bounds,method='L-BFGS-B',options={'maxiter':1000,'ftol':1e-12,'gtol':1e-8})
        if not res.success:raise ValueError(res.message)
        pred=np.clip(X@res.x,0,1);cache[key]=(res.x,pred);return cache[key]
    def miss(v,f):return max(0,(f['min'] or 0)-v)+max(0,v-(1 if f['max'] is None else f['max']))
    def choose(train):
        ranking=[]
        for reg in grid:
            errors=[]
            for held in train:
                _,pred=fit([i for i in train if i!=held],reg);errors.append(miss(pred[flags[held]['index']],flags[held]))
            ranking.append((float(np.mean(errors)),-reg,reg))
        return min(ranking)[2]
    rows=[]
    for held in range(len(flags)):
        train=[i for i in range(len(flags)) if i!=held];reg=choose(train);w,pred=fit(train,reg);v=float(pred[flags[held]['index']])
        rows.append({'song':names[flags[held]['hash']],'score':v,'min':flags[held]['min'],'max':flags[held]['max'],'holds':bool(miss(v,flags[held])<1e-7),'regularization':reg})
        print('NESTED GRADED',rows[-1],flush=True)
    reg=choose(list(range(len(flags))));w,pred=fit(list(range(len(flags))),reg)
    report={'protocol':__doc__,'features':labels,'selectedRegularization':reg,'coefficients':w.tolist(),'nestedLeaveOneOut':rows,
        'nestedHeld':sum(r['holds'] for r in rows),'training':[{'song':names[f['hash']],'score':float(pred[f['index']]),'holds':bool(miss(pred[f['index']],f)<1e-7)} for f in flags],
        'scores':dict(zip(hs,pred.tolist())),'flags':flags,'source':'Eight publisher-trained emotion inputs; personal grades learned from fourteen examples.'}
    (a.directory/('graded-audio-style-evidence-study.json' if a.include_style_evidence else 'graded-audio-evidence-study.json')).write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print('RESULT nested',report['nestedHeld'],'/',len(flags),'train',sum(r['holds'] for r in report['training']),'REG',reg)
    print('WEIGHTS',list(zip(['intercept']+labels,w)))


if __name__=='__main__':main()
