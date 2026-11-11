"""Audio-space interval learning, with nested held-out selection and library drift.

Read-only experiment. Every fold fits only the other flags. Unsupervised geometry
comes from the fixed reference set, not flag labels. Hard interval fitting is
explicitly distinguished from held-out evidence; no training-fit success claim.
"""
import argparse
import json
from pathlib import Path
import sys
import time

import numpy as np
from scipy.optimize import minimize
from scipy.spatial.distance import pdist


def unit(X):
    return X / np.maximum(np.linalg.norm(X, axis=1, keepdims=True), 1e-12)


def fit(K, prior, flags, train, drift=1., margin=.01):
    if not len(train):
        return prior.copy()
    idx = np.array([flags[i]['index'] for i in train])
    A = K[np.ix_(idx, idx)] + np.eye(len(idx)) * 1e-8
    inv = np.linalg.inv(A)
    transfer = K[:, idx] @ inv
    # Numerically exact own-example interpolation, including regularized linear kernels.
    transfer[idx] = np.eye(len(idx))
    loose = np.ones(len(K), dtype=bool)
    loose[idx] = False
    H = inv + drift * transfer[loose].T @ transfer[loose] / max(1, loose.sum())
    H = (H + H.T) / 2
    H /= max(np.linalg.norm(H, ord=2), 1e-12)
    bounds = []
    for i, row in zip(train, idx):
        f = flags[i]
        lo = 0 if f['min'] is None else min(1., f['min'] + margin)
        hi = 1 if f['max'] is None else max(0., f['max'] - margin)
        bounds.append((lo - prior[row], hi - prior[row]))
    initial = np.array([np.clip(0., lo, hi) for lo, hi in bounds])
    res = minimize(lambda y: .5 * y @ H @ y, initial, jac=lambda y: H @ y,
                   bounds=bounds, method='L-BFGS-B', options={'ftol': 1e-13, 'gtol': 1e-9, 'maxiter': 300})
    if not res.success:
        res = minimize(lambda y: .5 * y @ H @ y, res.x, jac=lambda y: H @ y,
                       bounds=bounds, method='SLSQP', options={'ftol': 1e-12, 'maxiter': 300})
        if not res.success:
            raise ValueError(res.message)
    return np.clip(prior + transfer @ res.x, 0, 1)


def error(v, f):
    return max(0, (f['min'] or 0) - v) + max(0, v - (1 if f['max'] is None else f['max']))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    a = p.parse_args()
    started = time.monotonic()
    data = json.loads((a.directory / 'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    with np.load(a.directory / 'audio-geometry.npz') as z:
        X, R = unit(z['means'].astype(float)), unit(z['reference'].astype(float))
        windows = unit(z['windows'].astype(float)) if 'windows' in z else None
        offsets = z['offsets'].copy() if 'offsets' in z else None
    hs = data['hashes']
    names = {e['hash']: e['name'] for e in data['entries']}
    mean = R.mean(0)
    _, s, V = np.linalg.svd(R - mean, full_matrices=False)
    variance = s ** 2 / len(R)
    transforms = {'raw': X, 'centered': unit(X - mean)}
    for remove in [1, 3, 10]:
        Z = X - mean
        Z = Z - (Z @ V[:remove].T) @ V[:remove]
        transforms[f'remove-{remove}-reference-pcs'] = unit(Z)
    for ridge in [.1, 1., 10.]:
        # Smooth reference whitening suppresses dominant shared directions.
        scale = np.sqrt(variance + np.median(variance[variance > 1e-10]) * ridge)
        transforms[f'whiten-{ridge}'] = unit(((X - mean) @ V.T) / scale)
    kernels = {}
    for name, Z in transforms.items():
        cosine = np.clip(Z @ Z.T, -1, 1)
        kernels[name + '/linear'] = cosine
        for width in [.1, .25, .5, 1.]:
            kernels[name + f'/rbf-{width}'] = np.exp(-(1 - cosine) / width)
    specs = [(name, drift) for name in kernels for drift in [0., 10.]]
    report = {'geometry': {'dimensions': X.shape[1], 'songs': len(X), 'reference': len(R),
                          'referenceEffectiveRank': float(variance.sum() ** 2 / (variance @ variance)),
                          'referenceTop10Variance': float(variance[:10].sum() / variance.sum())}, 'moods': []}
    for mood in data['moods']:
        if len(mood['flags']) < 4:
            continue
        flags = [dict(f, index=hs.index(f['hash']), song=names[f['hash']]) for f in mood['flags'] if f['fresh']]
        n = len(flags)
        if n < 4:
            continue
        prior = np.array([mood['scores'][h] for h in hs])
        cache = {}
        def predict(spec, train):
            key = (spec, tuple(train))
            if key not in cache:
                name, drift = spec
                cache[key] = fit(kernels[name], prior, flags, train, drift)
            return cache[key]
        def choose(train):
            ranking = []
            for spec in specs:
                misses = []
                for held in train:
                    pred = predict(spec, [i for i in train if i != held])
                    misses.append(error(pred[flags[held]['index']], flags[held]))
                fitted = predict(spec, train)
                # Held-out interval error first, then library drift; no test-label use.
                ranking.append((float(np.mean(misses)), float(np.abs(fitted-prior).mean()), spec))
            return min(ranking)[2]
        rows = []
        for held in range(n):
            train = [i for i in range(n) if i != held]
            best = choose(train)
            value = float(predict(best, train)[flags[held]['index']])
            rows.append({'song': flags[held]['song'], 'before': float(prior[flags[held]['index']]),
                         'after': value, 'holds': error(value, flags[held]) < 1e-7,
                         'min': flags[held]['min'], 'max': flags[held]['max'], 'selected': best})
        best = choose(list(range(n)))
        after = predict(best, list(range(n)))
        experiments = []
        for spec in specs:
            values = [float(predict(spec, [i for i in range(n) if i != held])[flags[held]['index']]) for held in range(n)]
            experiments.append({'spec': spec, 'looHeld': sum(error(v, f) < 1e-7 for v, f in zip(values, flags)),
                'looError': sum(error(v, f) for v, f in zip(values, flags)), 'scores': values})
        result = {'id': mood['id'], 'name': mood['name'], 'flags': flags, 'selected': best,
            'baselineHeld': int(sum(error(prior[f['index']], f) < 1e-7 for f in flags)),
            'nestedLooHeld': sum(r['holds'] for r in rows), 'nestedLoo': rows, 'fixedModelLoo': experiments,
            'trainingHeld': int(sum(error(after[f['index']], f) < 1e-7 for f in flags)),
            'meanDrift': float(np.abs(after-prior).mean()), 'saturated': int(((after < .05) | (after > .95)).sum()),
            'scores': dict(zip(hs, after.tolist())), 'movers': [dict(song=names[hs[i]], before=float(prior[i]), after=float(after[i])) for i in np.argsort(-np.abs(after-prior))[:20]]}
        report['moods'].append(result)
        print(mood['name'], 'baseline', result['baselineHeld'], '/', n, 'nested LOO', result['nestedLooHeld'], 'train', result['trainingHeld'], 'model', best, 'drift', round(result['meanDrift'],3), flush=True)
        (a.directory / 'audio-geometry-study.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    horror = next(m for m in data['moods'] if m['id'] == 'grim')
    indices = [hs.index(f['hash']) for f in horror['flags']]
    report['horrorCosines'] = {'songs': [names[hs[i]] for i in indices], 'matrix': (X[indices] @ X[indices].T).tolist()}
    report['horrorNeighbors'] = []
    for i in indices:
        others = np.argsort(-(X @ X[i]))
        report['horrorNeighbors'].append({'song': names[hs[i]], 'windowCoherence': [] if windows is None else (windows[offsets[i]:offsets[i+1]] @ X[i]).tolist(),
            'neighbors': [{'song': names[hs[j]], 'cosine': float(X[j] @ X[i])} for j in others if j != i][:10]})
    report['seconds'] = time.monotonic() - started
    (a.directory / 'audio-geometry-study.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
