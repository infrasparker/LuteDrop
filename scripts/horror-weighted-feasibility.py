"""Offline diagnostic of whether unequal positive prompt weights help Horror.

This is not a supported app vocabulary format and never writes app settings.
Uses exact reference moments recovered and independently checked by the
exhaustive study. Multi-start optimization is exploratory, not infeasibility proof.
"""
import argparse
import itertools
import json
import math
from pathlib import Path

import numpy as np
from scipy.optimize import minimize


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('report',type=Path);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    d=json.loads(a.report.read_text(encoding='utf-8'));U=np.array(d['numericBasis']['raw']);mu=np.array(d['numericBasis']['referenceMean']);C=np.array(d['numericBasis']['referenceCovariance'])
    eig,V=np.linalg.eigh(C);C=(V*np.maximum(eig,0)[None,:])@V.T
    hashes=[e['hash'] for e in d['entries']];idx=[hashes.index(f['hash']) for f in d['flags']];centered=(U[:,idx]-mu[:,None]).T;n=len(mu)
    midpoint=.674;slope=math.log(3)/(1.2816-midpoint)
    def logitz(v):return midpoint+math.log(v/(1-v))/slope
    lows=[i for i,f in enumerate(d['flags']) if f['side']=='high'];rest=[i for i in range(len(idx)) if i not in lows]
    rng=np.random.default_rng(47)
    current=np.zeros(n)
    currentlines=next(m['prompts'] for m in d['vocabulary']['moods'] if m['id']=='grim')
    for line in currentlines:current[next(i for i,s in enumerate(d['selected']) if s['line']==line)]=1/len(currentlines)
    starts=[current,np.ones(n)/n]+[rng.dirichlet(np.ones(n)*.4) for _ in range(3)]
    def solve(positions):
        rows=[];targets=[];signs=[]
        for i in positions:
            f=d['flags'][i]
            if f['min'] is not None:rows.append(centered[i]);targets.append(logitz(f['min']));signs.append(1)
            if f['max'] is not None:rows.append(centered[i]);targets.append(logitz(f['max']));signs.append(-1)
        A=np.array(rows);t=np.array(targets);sign=np.array(signs)
        def con(x):
            w=x[:-1];sd=np.sqrt(max(w@C@w,1e-20));return sign*(A@w/sd-t)-x[-1]
        def jac(x):
            w=x[:-1];cw=C@w;sd=np.sqrt(max(w@cw,1e-20));z=A@w
            grad=A/sd-z[:,None]*cw[None,:]/sd**3
            return np.column_stack((sign[:,None]*grad,-np.ones(len(A))))
        best=None
        for w0 in starts:
            x0=np.r_[w0,-1.0]
            res=minimize(lambda x:-x[-1],x0,jac=lambda x:np.r_[np.zeros(n),-1.],method='SLSQP',
                bounds=[(0,1)]*n+[(-5,5)],constraints=[{'type':'eq','fun':lambda x:x[:-1].sum()-1,'jac':lambda x:np.r_[np.ones(n),0]},
                    {'type':'ineq','fun':con,'jac':jac}],options={'maxiter':250,'ftol':1e-9})
            if np.min(con(res.x))< -1e-5:continue
            if best is None or res.x[-1]>best.x[-1]:best=res
        if best is None:return None
        w=best.x[:-1];sd=np.sqrt(w@C@w);allz=(U.T@w-mu@w)/sd;bars=1/(1+np.exp(np.clip(-slope*(allz-midpoint),-700,700)))
        return {'marginZ':float(best.x[-1]),'optimizerSuccess':bool(best.success),'positions':positions,
            'weights':[{'line':d['selected'][i]['line'],'weight':float(v)} for i,v in enumerate(w) if v>1e-5],
            'flagScores':[{'song':f['song'],'score':float(bars[i]),'held':bool((f['min'] is None or bars[i]>=f['min']-1e-6) and (f['max'] is None or bars[i]<=f['max']+1e-6))} for f,i in zip(d['flags'],idx)],
            'scores':dict(zip(hashes,bars.tolist()))}
    report={'single':solve(list(range(len(idx)))),'partitions':[],'source':str(a.report)}
    for mask in range(0,1<<(len(lows)-1)):
        left=[lows[0]]+[lows[i] for i in range(1,len(lows)) if mask&(1<<(i-1))]
        right=[i for i in lows if i not in left]
        if not right:continue
        x,y=solve(left+rest),solve(right+rest)
        report['partitions'].append({'left':[d['flags'][i]['song'] for i in left],'right':[d['flags'][i]['song'] for i in right],'a':x,'b':y,
            'feasible':x is not None and y is not None and min(x['marginZ'],y['marginZ'])>=-1e-5})
        print('PARTITION',len(report['partitions']),None if x is None else round(x['marginZ'],4),None if y is None else round(y['marginZ'],4),flush=True)
    a.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print('SINGLE',None if report['single'] is None else report['single']['marginZ'],flush=True)
    print('FEASIBLE PARTITIONS',sum(p['feasible'] for p in report['partitions']),flush=True)
    if report['single']:
        print(report['single']['weights'],flush=True);print(report['single']['flagScores'],flush=True)


if __name__=='__main__':main()
