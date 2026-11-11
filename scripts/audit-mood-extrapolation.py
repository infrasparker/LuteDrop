"""Read-only ridge sensitivity and held-out checks using cached analyzer features."""
import json
import argparse
import os
import sqlite3
import sys
from pathlib import Path

import numpy as np
from scipy.optimize import minimize

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tagger'))
from tagger import personal, tune, adaptive

BASE = Path(r'C:\Users\Infrasparker\.codex\visualizations\2026\11\08\01a1b8e6-30b2-7ea0-b750-7cc1eefbb98e')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--current-flags', action='store_true')
    parser.add_argument('--selection', action='store_true')
    parser.add_argument('--local', action='store_true')
    parser.add_argument('--adaptive', action='store_true')
    args = parser.parse_args()
    report = json.loads((BASE / 'reference-anchored-mood-upgrade.json').read_text())
    cache = np.load(BASE / 'reference-anchoring-features.npz')
    hashes = cache['hashes'].tolist()
    index = {h: i for i, h in enumerate(hashes)}
    db = Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db'
    with sqlite3.connect(db.as_uri() + '?mode=ro', uri=True) as conn:
        names = dict(conn.execute("SELECT content_hash,name FROM library_entries WHERE entry_type='music'"))
        live_flags = conn.execute('SELECT e.content_hash,f.mood,f.kind,f.band,f.bound,f.bar_at_flag FROM mood_flags f JOIN library_entries e ON e.id=f.library_entry_id').fetchall()
        live_levels = conn.execute('SELECT e.content_hash,m.mood,m.user_level FROM library_entry_moods m JOIN library_entries e ON e.id=m.library_entry_id WHERE m.user_level IS NOT NULL').fetchall()
    selected = [index[h] for h in names if h in index]
    output = []
    for record in report['reviews']:
        mid = record['mood']
        request = record['request']
        features, anchors = cache[mid + '_features'], cache[mid + '_anchors']
        X = np.column_stack([np.ones(len(features)), features])
        R = np.column_stack([np.ones(len(anchors)), anchors])
        signs = [-1 if s.startswith('- ') else 1 for s in request['current_lines']]
        scale = np.pi * 1.8 / 4
        base = np.r_[-.674 * scale, np.zeros(19), [scale*s/signs.count(s) for s in signs]]
        bounds = [(-8, 8)]*20 + [(0, 8) if s > 0 else (-8, 0) for s in signs]
        flags = [tune.FlagTarget(index[f['hash']], f.get('min'), f.get('max'), 1., True, 'mid',
                   target=f.get('target'), polarity=f.get('polarity'), confidence=f.get('confidence', 1),
                   pivot=f.get('pivot', .5), feedback_version=5) for f in request['flags']]
        if args.current_flags:
            flags = []
            flagged = set()
            for h, mood, kind, band, bound, at in live_flags:
                if mood != mid:
                    continue
                if h not in index:
                    raise ValueError(f'Cached features missing for {names.get(h, h)}')
                lo = {'definitely':.9, 'understandable':.75, 'stretch':.5, 'no':None}.get(band)
                hi = {'definitely':None, 'understandable':.9, 'stretch':.75, 'no':.5}.get(band)
                polarity = None
                pivot = .5
                if kind in ('higher', 'lower'):
                    polarity = 1 if kind == 'higher' else -1
                    pivot = at
                    lo, hi = (min(1, at+.1), None) if kind == 'higher' else (None, max(0, at-.1))
                else:
                    if bound == 'at_least': hi = None
                    elif bound == 'at_most': lo = None
                    positive = band in ('definitely', 'understandable')
                    if not ((positive and bound == 'at_most') or (not positive and bound == 'at_least')):
                        polarity = 1 if positive else -1
                flags.append(tune.FlagTarget(index[h],lo,hi,1.,True,'mid',polarity=polarity,confidence=4 if band in ('definitely','no') else 1,pivot=pivot,feedback_version=5))
                flagged.add(h)
            for h, mood, level in live_levels:
                if mood == mid and h not in flagged and h in index:
                    flags.append(tune.FlagTarget(index[h],None,None,1.,True,'mid',target=level,confidence=4,feedback_version=5))
        comparisons = [(index[a], index[b]) for a,b in request.get('comparisons', [])]

        def fit(rows, ridge):
            fs = [flags[i] for i in rows]
            if not fs and not comparisons:
                return base, personal.smooth_score(X @ base)
            A = X[[f.index for f in fs]]
            weight = np.array([f.confidence/4 for f in fs])
            directions = {f.polarity for f in fs if f.polarity is not None and f.pivot == .5}
            strength = .03 if directions == {-1, 1} else .3
            omitted = {f.index for i,f in enumerate(flags) if i not in rows}
            D = np.array([X[a]-X[b] for a,b in comparisons if a not in omitted and b not in omitted]).reshape((-1, X.shape[1]))

            def objective(w):
                losses, grad = personal.directional_losses(A @ w, fs)
                delta = w-base
                drift = R @ delta
                pair = D @ w
                pcost = .02*np.mean(np.logaddexp(0, -pair)) if len(D) else 0
                pgrad = -.02*D.T @ (1/(1+np.exp(np.clip(pair, -50, 50))))/len(D) if len(D) else 0
                return float(weight @ losses + strength*np.mean(drift**2) + ridge*(delta@delta) + pcost), A.T @ (weight*grad) + 2*strength*R.T@drift/len(R) + 2*ridge*delta + pgrad

            opt = minimize(objective, base, jac=True, bounds=bounds, method='L-BFGS-B', options={'maxiter':2000, 'ftol':1e-12, 'gtol':1e-8})
            if not opt.success:
                raise RuntimeError(opt.message)
            return opt.x, personal.smooth_score(X @ opt.x)

        rows = list(range(len(flags)))
        if args.adaptive:
            data = np.load(BASE/'preprojection-features.npz')
            assert data['hashes'].tolist()==hashes
            diagnostics = {}
            ref_hidden = np.load(BASE/'preprojection-reference-features.npz')['means']
            w,scores,_ = adaptive.fit(features,data['means'],flags,signs,19,reference_features=anchors,reference_geometry=personal.audio_geometry(ref_hidden),comparisons=comparisons,diagnostics=diagnostics)
            result = dict(mood=request['name'],method=diagnostics['method'],width=diagnostics['local'].width if diagnostics['local'] else None,
                validation_loss=diagnostics['validation_loss'],count=len(flags),above75=int(sum(scores[selected]>.75)),
                examples={names[h]:round(float(scores[i]),3) for h,i in index.items() if any(s in names.get(h,'').lower() for s in ['lead your fights','forlorn cliffs',"kyne's peace",'bloodletting','fangle forest','04 peril','dancing with'])})
            output.append(result)
            print(json.dumps(result),flush=True)
            continue
        if args.local:
            all_audio = np.load(BASE/'preprojection-features.npz')
            H = np.asarray(all_audio['means'],float)
            H /= np.maximum(np.linalg.norm(H,axis=2,keepdims=True),1e-12)
            cosine = np.mean(np.einsum('ild,jld->ijl',H,H),axis=2)
            projected = np.asarray(all_audio['finals'],float)
            projected /= np.maximum(np.linalg.norm(projected,axis=1,keepdims=True),1e-12)
            head = features[:,:19]/np.maximum(np.std(anchors[:,:19],axis=0),.05)
            head_dist = np.maximum(0,np.sum(head**2,axis=1)[:,None]+np.sum(head**2,axis=1)[None,:]-2*head@head.T)
            latent = X@base
            results = []
            for space,dist,widths in [('hidden',np.maximum(0,1-cosine),[.01,.03,.1,.3]),('projected',np.maximum(0,1-projected@projected.T),[.03,.1,.3]),('heads',head_dist,[2.,8.,32.])]:
                for width in widths:
                    K = np.exp(-dist/width)
                    for regularization in [.001,.01,.1,1.]:
                        def local_fit(keep):
                            if not keep:
                                return personal.smooth_score(latent)
                            fs = [flags[i] for i in keep]
                            ids = [f.index for f in fs]
                            A = K[np.ix_(ids,ids)] + np.eye(len(ids))*1e-6
                            weights = np.array([f.confidence/4 for f in fs])
                            def objective(coef):
                                losses,gradient = personal.directional_losses(latent[ids]+A@coef,fs)
                                return float(weights@losses+regularization*coef@A@coef), A@(weights*gradient)+2*regularization*A@coef
                            opt = minimize(objective,np.zeros(len(ids)),jac=True,method='L-BFGS-B',options={'maxiter':1000,'ftol':1e-11})
                            if not opt.success: raise ValueError(opt.message)
                            return personal.smooth_score(latent+K[:,ids]@opt.x)
                        score = local_fit(rows)
                        losses = []
                        for i,f in enumerate(flags):
                            hold = local_fit([j for j in rows if j!=i])
                            losses.append(personal.directional_losses([np.tan(np.pi*(hold[f.index]-.5))],[f])[0][0])
                        result = dict(mood=request['name'],space=space,width=width,ridge=regularization,
                            loss=float(np.average(losses,weights=[f.confidence for f in flags])) if flags else 0,
                            examples={names[h]:round(float(score[i]),3) for h,i in index.items() if any(s in names.get(h,'').lower() for s in ['lead your fights','forlorn cliffs',"kyne's peace",'bloodletting','fangle forest','04 peril','dancing with'])})
                        results.append(result)
            output.extend(results)
            best = sorted(results,key=lambda r:r['loss'])[:3]
            print(json.dumps(dict(mood=request['name'],best=best)),flush=True)
            continue
        candidates = []
        for ridge in [.0001, .001, .01, .03, .1, 1., 10.]:
            w, scores = fit(rows, ridge)
            held = []
            for i,f in enumerate(flags):
                wh, _ = fit([j for j in rows if j != i], ridge)
                held.append(personal.directional_losses([X[f.index] @ wh], [f])[0][0])
            result = dict(mood=request['name'], ridge=ridge,
                heldLoss=float(np.average(held, weights=[f.confidence for f in flags])) if held else None,
                above75=int(sum(scores[selected] > .75)), above90=int(sum(scores[selected] > .9)),
                examples={names[h]: round(float(scores[i]), 3) for h,i in index.items() if any(s in names.get(h, '').lower() for s in ['lead your fights', 'forlorn cliffs', "kyne's peace", 'bloodletting', 'dank bodies', 'fangle forest', '04 peril', 'jerall', 'dancing with'])})
            output.append(result)
            candidates.append((ridge, w, np.array(held)))
            if not args.selection:
                print(json.dumps(result), flush=True)
        if args.selection:
            weight = np.array([f.confidence for f in flags],dtype=float)
            weight /= weight.sum() if len(weight) else 1
            prior_loss = personal.directional_losses([X[f.index]@base for f in flags],flags)[0]
            choices = [(float('inf'),base,prior_loss)] + list(reversed(candidates))
            best = min(choices,key=lambda c:float(weight@c[2]))
            eligible = []
            for choice in choices:
                difference = choice[2]-best[2]
                mean = float(weight@difference)
                se = np.sqrt(float(weight@((difference-mean)**2))*float(weight@weight))
                if mean <= se+1e-12:
                    eligible.append(choice)
            chosen = eligible[0]
            score = personal.smooth_score(X@chosen[1])
            print(json.dumps(dict(mood=request['name'],count=len(flags),chosen=chosen[0],loss=round(float(weight@chosen[2]),4),prior=round(float(weight@prior_loss),4),examples={names[h]:round(float(score[i]),3) for h,i in index.items() if any(s in names.get(h,'').lower() for s in ['lead your fights','forlorn cliffs',"kyne's peace",'bloodletting','fangle forest','04 peril','dancing with'])})),flush=True)
    destination = BASE / ('mood-adaptive-current.json' if args.adaptive else 'mood-extrapolation-current.json' if args.current_flags else 'mood-extrapolation-audit.json')
    destination.write_text(json.dumps(output, indent=2))


if __name__ == '__main__':
    main()
