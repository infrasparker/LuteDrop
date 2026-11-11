"""Constrained learned directions with original reference calibration and held-out checks.

Unlike prompt-only tuning, allow a direction to leave the positive text cone.
Variance is fixed at one on the shipped reference; no sigmoid steepening, no
per-song overrides. Fit flags while minimizing drift from the original lens.
"""
import argparse
import json
from pathlib import Path
import time

import numpy as np
from scipy.optimize import minimize
from scipy.special import expit, logit


def unit(X):
    return X / np.maximum(np.linalg.norm(X, axis=1, keepdims=True), 1e-12)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    p.add_argument('--window-fold', type=int, choices=[0,1])
    p.add_argument('--robust-windows', action='store_true')
    a = p.parse_args()
    data = json.loads((a.directory / 'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    with np.load(a.directory / 'audio-geometry.npz') as f:
        X, R = unit(f['means'].astype(float)), unit(f['reference'].astype(float))
        testX = None
        if a.window_fold is not None:
            windows=unit(f['windows'].astype(float));offsets=f['offsets']
            trainrows=[];testrows=[]
            for i in range(len(X)):
                w=windows[offsets[i]:offsets[i+1]]
                if len(w)<2:raise ValueError('Window validation needs at least two windows per song')
                trainrows.append(w[a.window_fold::2].mean(0));testrows.append(w[1-a.window_fold::2].mean(0))
            X=unit(np.array(trainrows));testX=unit(np.array(testrows))
    with np.load(a.directory / 'text-geometry.npz') as f:
        texts, T = f['texts'].tolist(), f['vectors'].astype(float)
    mood = next(m for m in data['moods'] if m['id'] == 'grim')
    flags = [dict(f, index=data['hashes'].index(f['hash'])) for f in mood['flags'] if f['fresh']]
    names = {e['hash']: e['name'] for e in data['entries']}
    mapping = mood['calibration']
    mu = R.mean(0)
    Rc = R - mu
    _, s, V = np.linalg.svd(Rc, full_matrices=False)
    sd = s / np.sqrt(len(R))
    w0 = T[[texts.index(t) for t in mood['prompts']]].mean(0)
    original_sd = np.std(R @ w0)
    prior = expit(mapping['slope'] * (((X-mu)@w0)/original_sd - mapping['midpoint_z']))
    print('Original reconstruction error', np.max(np.abs(prior-np.array([mood['scores'][h] for h in data['hashes']]))), flush=True)
    report = {'models': [], 'originalScores': dict(zip(data['hashes'], prior.tolist())), 'flags': [dict(f, song=names[f['hash']]) for f in flags]}
    zmin = np.array([-np.inf if f['min'] is None else mapping['midpoint_z']+logit(min(.999,f['min']+.01))/mapping['slope'] for f in flags])
    zmax = np.array([np.inf if f['max'] is None else mapping['midpoint_z']+logit(max(.001,f['max']-.01))/mapping['slope'] for f in flags])
    def holds(b, f):
        return bool((f['min'] is None or b>=f['min']-1e-6) and (f['max'] is None or b<=f['max']+1e-6))
    outputname='learned-mood-direction-study.json' if a.window_fold is None else f'learned-direction-window-fold-{a.window_fold}{"-robust" if a.robust_windows else ""}.json'
    for k in ([32,64,128,256] if a.robust_windows else [128] if a.window_fold is not None else [4, 8, 16, 32, 64, 128]):
        # Reference PCs plus whatever part of the current prompt lies outside
        # their span. This preserves the current lens exactly at every dimension.
        B = V[:k].T / sd[:k]
        Zr = Rc @ B
        p0 = w0 / original_sd
        beta = Zr.T @ (Rc @ p0) / len(R)
        residual = p0 - B @ beta
        residual_sd = np.std(Rc @ residual)
        if residual_sd > 1e-8:
            B = np.column_stack((B, residual / residual_sd))
            q0 = np.r_[beta, residual_sd]
        else:
            q0 = beta
        Z = (X-mu) @ B
        for reg in ([.1] if a.window_fold is not None else [.1, 1., 10.]):
            def solve(train):
                idx = np.array([flags[i]['index'] for i in train])
                A = Z[idx]
                loose = np.ones(len(X),dtype=bool)
                loose[idx] = False
                H = Z[loose].T@Z[loose]/max(1,loose.sum())+reg*np.eye(len(q0))
                H /= np.linalg.norm(H, ord=2)
                lower, upper = zmin[train], zmax[train]
                if a.robust_windows:
                    samples=[];sample_lo=[];sample_hi=[]
                    for i in train:
                        songrow=flags[i]['index']
                        w=windows[offsets[songrow]:offsets[songrow+1]][a.window_fold::2]
                        samples.extend((w-mu)@B)
                        sample_lo.extend([zmin[i]]*len(w));sample_hi.extend([zmax[i]]*len(w))
                    A=np.array(samples);lower=np.array(sample_lo);upper=np.array(sample_hi)
                il, iu = np.flatnonzero(np.isfinite(lower)), np.flatnonzero(np.isfinite(upper))
                C = np.vstack((A[il], -A[iu]))
                target = np.r_[lower[il], -upper[iu]]
                result = minimize(lambda q: .5*(q-q0)@H@(q-q0), q0.copy(), jac=lambda q:H@(q-q0),
                    constraints=[{'type':'eq','fun':lambda q:q@q-1.,'jac':lambda q:2*q},
                                 {'type':'ineq','fun':lambda q:C@q-target,'jac':lambda q:C}],
                    method='SLSQP',options={'maxiter':250,'ftol':1e-10})
                feasible = bool(abs(result.x@result.x-1)<1e-5 and (C@result.x-target).min()>=-1e-5)
                values = expit(mapping['slope'] * (Z @ result.x / max(np.linalg.norm(result.x),1e-12) - mapping['midpoint_z']))
                return result.x, values, feasible
            start = time.monotonic()
            q, after, feasible = solve(list(range(len(flags))))
            loo = []
            for held in range(len(flags)):
                _, predicted, ok = solve([i for i in range(len(flags)) if i!=held])
                loo.append({'song':names[flags[held]['hash']], 'score':float(predicted[flags[held]['index']]), 'holds':holds(predicted[flags[held]['index']],flags[held]), 'feasible':ok})
            w = B @ q
            item = {'dimensions':k+1,'regularization':reg,'feasible':feasible,'trainHeld':sum(holds(after[f['index']],f) for f in flags),
                'looHeld':sum(r['holds'] for r in loo),'loo':loo,'meanDrift':float(np.abs(after-prior).mean()),'saturated':int(((after<.05)|(after>.95)).sum()),
                'directionCosine':float(q@q0),'vector':w.tolist(),'scores':dict(zip(data['hashes'],after.tolist())),
                'flagScores':[dict(song=names[f['hash']],score=float(after[f['index']]),holds=holds(after[f['index']],f)) for f in flags],
                'seconds':time.monotonic()-start}
            if testX is not None:
                predicted=expit(mapping['slope']*((testX-mu)@w/np.std(R@w)-mapping['midpoint_z']))
                item['unseenWindows']=[dict(song=names[f['hash']],score=float(predicted[f['index']]),holds=holds(predicted[f['index']],f)) for f in flags]
                print('UNSEEN WINDOWS',sum(r['holds'] for r in item['unseenWindows']),item['unseenWindows'],flush=True)
            # Semantic descriptions of the *change*, not claims about interpretable
            # individual coordinates. Rank all supplied text embeddings.
            delta = w-w0/original_sd
            alignment = T@delta/np.maximum(np.linalg.norm(T,axis=1)*np.linalg.norm(delta),1e-12)
            item['addedSemantics']=[{'text':texts[i],'alignment':float(alignment[i])} for i in np.argsort(-alignment)[:12]]
            item['removedSemantics']=[{'text':texts[i],'alignment':float(alignment[i])} for i in np.argsort(alignment)[:12]]
            report['models'].append(item)
            (a.directory/outputname).write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
            print('DIRECTION',k+1,reg,'feasible',feasible,'train',item['trainHeld'],'LOO',item['looHeld'],'drift',round(item['meanDrift'],3),'saturated',item['saturated'],flush=True)


if __name__=='__main__':
    main()
