"""Matched retrospective comparison of signed text prompts and audio evidence.

Fourteen development judgments use nested song/album exclusion. Three later
judgments stay out of fitting, but were already known when this study was
designed: they are a retrospective check, not a new blind test. Frozen earlier
pilots remain immutable. This script never writes to the app or analyzer model.
"""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import re
import sqlite3

import numpy as np
from scipy.optimize import minimize
from scipy.special import expit


LABELS = ['Eerie', 'Unnerving', 'Suspenseful', 'Dark', 'Somber', 'Calming', 'Relaxed', 'Uplifting']
WORDS = [
    ['eerie', 'haunting', 'ghostly', 'spectral'],
    ['unnerving', 'unsettling', 'disturbing', 'sinister'],
    ['suspenseful', 'tense', 'foreboding', 'impending danger'],
    ['dark', 'ominous', 'dread', 'horror'],
    ['somber', 'desolate', 'bleak', 'funereal'],
    ['calming', 'peaceful', 'serene', 'reassuring'],
    ['relaxed', 'gentle', 'comfortable', 'intimate'],
    ['uplifting', 'hopeful', 'inspiring', 'triumphant'],
]
GRID = [1e-6, 1e-5, 1e-4, .001, .01, .1, 1.]


def fetch_text_axes(base, hashes, current_prompts):
    path = base / 'signed-prompt-axis-scores.json'
    axes = []
    for mode in ['single', 'lines', 'commas']:
        for label, words in zip(LABELS, WORDS):
            prompts = [words[0]] if mode == 'single' else words if mode == 'lines' else [', '.join(words)]
            axes.append({'label': label, 'mode': mode, 'prompts': prompts})
    axes.append({'label': 'Current Horror', 'mode': 'current', 'prompts': current_prompts})
    if path.exists():
        cached = json.loads(path.read_text(encoding='utf-8'))
        if cached['hashes'] != hashes or [a['prompts'] for a in cached['axes']] != [a['prompts'] for a in axes]:
            raise ValueError('Cached inputs differ; choose a new research directory')
        return cached
    root = Path(__file__).resolve().parents[1]
    spec = importlib.util.spec_from_file_location('export', root / 'ui-preview/export_fixtures.py')
    export = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(export)
    db = Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db'
    with sqlite3.connect(db.resolve().as_uri() + '?mode=ro', uri=True) as conn:
        meta = export.meta(conn)
    keys = [chr(ord('a') + i) for i in range(len(axes))]
    response = export.post(meta['autotag_url'].rstrip('/'), meta['autotag_token'], '/score',
                           {'targets': hashes, 'mood_prompts': {k: a['prompts'] for k, a in zip(keys, axes)}}, 180)
    if set(response.get('mood', {})) != set(hashes):
        raise ValueError('Analyzer did not return all frozen target hashes')
    for key, axis in zip(keys, axes):
        axis['reference'] = response['mood_reference']['stats'][key]
        axis['raw'] = [response['mood'][h]['moods'][key] for h in hashes]
    report = {'createdAt': datetime.now(timezone.utc).isoformat(), 'hashes': hashes, 'axes': axes,
              'model': response['model'], 'reference': response['mood_reference']['set']}
    path.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    return report


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    p.add_argument('--include-later-in-cv', action='store_true', help='Internal CV over all 17 judgments; no later holdout claim')
    p.add_argument('--word-bank', action='store_true', help='Also compare broad and sparse signed word banks')
    p.add_argument('--residuals-only', action='store_true', help='Test restrained updates around Current, preserving the earlier comparisons')
    p.add_argument('--directional-judgments', action='store_true', help='Sensitivity: Understandable has no ceiling; uncertain Stretch has no floor')
    p.add_argument('--fusion-only', action='store_true', help='Explore joint audio and signed-text evidence; latest four judgments remain excluded')
    a = p.parse_args(); base = a.directory
    data = json.loads((base / 'ten-label-case/audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    mood = next(m for m in data['moods'] if m['id'] == 'grim')
    evidence = json.loads((base / 'publisher-audio-evidence.json').read_text(encoding='utf-8'))
    hashes = data['hashes']; byname = {e['name']: e for e in data['entries']}; byhash = {e['hash']: e for e in data['entries']}
    flags = [dict(f) for f in mood['flags']]
    # The user's clarification removes Lead's old, incorrectly assumed floor.
    lead = byname['Lead Your Fights']['hash']
    for flag in flags:
        if flag['hash'] == lead: flag['min'] = None; flag['side'] = 'low'
    for name, lo, hi in [('Wind from the Depths', .75, None), ('Burial Isle', .75, .9), ('Far Horizons', None, .5), ('The Cove Battle', .75, None)]:
        flags.append({'hash': byname[name]['hash'], 'min': lo, 'max': hi})
    if a.directional_judgments:
        for f in flags:
            if f['hash'] == byname['Burial Isle']['hash']: f['max'] = None
            if f['hash'] == byname['Temple of Woedica']['hash']: f['min'] = None
    for f in flags:
        f.update(index=hashes.index(f['hash']), song=byhash[f['hash']]['name'], album=byhash[f['hash']]['album'])
    pairs = [(hashes.index(byname['Forlorn Cliffs']['hash']), hashes.index(byname[n]['hash'])) for n in ['Temple of Woedica', 'Burial Isle']]
    axes = fetch_text_axes(base, hashes, mood['prompts'])
    mapping = mood['calibration']
    def zscore(axis):
        return (np.array(axis['raw']) - axis['reference']['mean']) / axis['reference']['sd']
    def bars(axis):
        return expit(mapping['slope'] * (zscore(axis) - mapping['midpoint_z']))
    current = bars(axes['axes'][-1])
    signs = [(0, None)] * 4 + [(None, None)] + [(None, 0)] * 3
    families = []
    for mode in ['single', 'lines', 'commas']:
        selected = [axis for axis in axes['axes'] if axis['mode'] == mode]
        for transform in ['linear', 'calibrated']:
            values = np.column_stack([(zscore(axis) if transform == 'linear' else bars(axis)) for axis in selected])
            families.append((f'text-{mode}-{transform}', values, signs, [x['prompts'] for x in selected]))
    E = np.column_stack([[head['scores'][h] for h in hashes] for head in evidence['heads']])
    families += [('learned-audio', E, signs, LABELS),
                 ('learned-audio-plus-current', np.column_stack([E, current]), signs + [(0, None)], LABELS + ['Current Horror'])]
    later_names = ['21 The Inquisition Marches', '23 Arkemyr', next(n for n in byname if 'Periki' in n)]
    later = [{'song': n, 'index': hashes.index(byname[n]['hash']), 'min': None, 'max': .5 if n == '23 Arkemyr' else .75} for n in later_names]
    if a.include_later_in_cv:
        for f in later:
            f = dict(f)
            f.update(hash=hashes[f['index']], album=byname[f['song']]['album'])
            flags.append(f)
    if a.word_bank:
        with np.load(base / 'text-geometry.npz', allow_pickle=False) as z:
            texts = z['texts'].tolist(); T = z['vectors'].astype(float)
        with np.load(base / 'audio-geometry.npz', allow_pickle=False) as z:
            library, reference = z['means'].astype(float), z['reference'].astype(float)
        def unit(v): return v / np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)
        library, reference, T = unit(library), unit(reference), unit(T)
        positive = r'horror|sinister|ominous|dark|grim|dread|creep|haunt|spook|eerie|unsett|macabr|morbid|spectr|ghost|threat|barren|desolat|eldritch|occult|mysteri|nightmare|ritual|demonic|forbidd|suspens|tense|dissonan|unnerv|fear|terr|abyss|cavern|impending|brood|malevol|absurd|somber'
        negative = r'calm|sooth|peace|seren|tranquil|comfort|relax|gentle|tender|intimate|reassur|warm|heroic|triumph|victor|inspir|noble|glori|reveren|sacred|holy|divine|heaven|angel|devot|prayer|uplift|hopeful'
        # Plain vocabulary only; no title-derived or newly hand-picked phrases.
        pi = [i for i, text in enumerate(texts) if len(text.split()) <= 2 and ',' not in text and re.search(positive, text.lower())]
        ni = [i for i, text in enumerate(texts) if len(text.split()) <= 2 and ',' not in text and re.search(negative, text.lower()) and i not in pi]
        ix = pi + ni; raw = library @ T[ix].T; ref = reference @ T[ix].T
        bank = (raw - ref.mean(axis=0)) / np.maximum(ref.std(axis=0), 1e-10)
        limits = [(0, None)] * len(pi) + [(None, 0)] * len(ni)
        words = [texts[i] for i in ix]
        families += [('text-bank-linear', bank, limits, words), ('text-bank-sparse-3-per-side', bank, limits, words)]
    if a.residuals_only:
        if not a.word_bank: raise ValueError('Residual comparison also needs --word-bank')
        families = [(f'learned-audio-residual-{strength}', E, signs, LABELS) for strength in [.01, .03, .1]] + [(f'text-bank-sparse-residual-{strength}', bank, limits, words) for strength in [.01, .03, .1]]
    if a.fusion_only:
        semantic = [family for family in families if family[0] in ['text-lines-linear', 'text-lines-calibrated', 'text-commas-linear']]
        families = [(f'fusion-audio-{name}', np.column_stack([E, values]), signs + bounds, LABELS + description) for name, values, bounds, description in semantic]
        if a.word_bank:
            families.append(('fusion-sparse-word-bank', np.column_stack([E, bank]), signs + limits, LABELS + words))
    output = base / ('signed-prompts-vs-audio-17.json' if a.include_later_in_cv else 'signed-prompts-vs-audio-14-expanded.json' if a.word_bank else 'signed-prompts-vs-audio.json')
    if a.residuals_only: output = base / 'signed-prompts-vs-audio-residuals.json'
    if a.fusion_only: output = base / 'signed-prompts-vs-audio-fusion.json'
    if a.directional_judgments: output = output.with_stem(output.stem + '-directional')
    report = {'protocol': __doc__, 'createdAt': datetime.now(timezone.utc).isoformat(), 'laterJudgmentsIncludedInCv': a.include_later_in_cv, 'directionalJudgmentSensitivity': a.directional_judgments, 'developmentFlags': flags,
              'featureLabels': LABELS, 'regularizationGrid': GRID, 'models': [], 'baseline': []}
    def miss(v, f):
        return max(0, (f['min'] or 0) - v) + max(0, v - (1 if f['max'] is None else f['max']))
    def row(v, f):
        return {'song': f['song'], 'score': float(v), 'min': f['min'], 'max': f['max'], 'miss': float(miss(v, f)), 'holds': bool(miss(v, f) < 1e-7)}
    report['baseline'] = [row(current[f['index']], f) for f in flags]
    for name, V, bounds, description in families:
        X = np.column_stack([np.ones(len(hashes)), V]); cache = {}
        residual = '-residual-' in name
        drift_weight = float(name.rsplit('-', 1)[1]) if residual else 0.
        anchor = current if residual else np.zeros(len(hashes))
        unflagged = [i for i in range(len(hashes)) if i not in {f['index'] for f in flags}]
        def fit(train, reg, screening=False):
            key = (tuple(sorted(train)), reg, screening)
            if key in cache: return cache[key]
            active = list(range(X.shape[1]))
            if (name.startswith('text-bank-sparse') or name == 'fusion-sparse-word-bank') and not screening:
                initial, _ = fit(train, .001, screening=True)
                first_word = 8 if name == 'fusion-sparse-word-bank' else 0
                pos = [i + 1 for i, bound in enumerate(bounds) if i >= first_word and bound[0] == 0]
                neg = [i + 1 for i, bound in enumerate(bounds) if i >= first_word and bound[1] == 0]
                active = list(range(first_word + 1)) + sorted(pos, key=lambda i: -initial[i])[:3] + sorted(neg, key=lambda i: initial[i])[:3]
            features = X[:, active]
            local_bounds = [(-3, 3)] + [bounds[i - 1] for i in active[1:]]
            idx = np.array([flags[i]['index'] for i in train]); A = features[idx]
            stability = features[unflagged].T @ features[unflagged] / len(unflagged)
            lo = np.array([0 if flags[i]['min'] is None else min(1, flags[i]['min'] + .03) for i in train])
            hi = np.array([1 if flags[i]['max'] is None else max(0, flags[i]['max'] - .03) for i in train])
            kept_pairs = [(u, v) for u, v in pairs if u in idx and v in idx]
            P = np.array([features[u] - features[v] for u, v in kept_pairs]).reshape((-1, features.shape[1]))
            pair_anchor = np.array([anchor[u] - anchor[v] for u, v in kept_pairs])
            def objective(w):
                v = anchor[idx] + A @ w; error = np.minimum(v - lo, 0) + np.maximum(v - hi, 0)
                loss = np.mean(error**2) + reg * (w[1:] @ w[1:] + .1 * w[0]**2) + drift_weight * (w @ stability @ w)
                grad = 2 * A.T @ error / len(train) + 2 * reg * np.r_[.1*w[0], w[1:]] + 2 * drift_weight * stability @ w
                if len(P):
                    err = np.minimum(pair_anchor + P @ w - .03, 0); loss += np.mean(err**2); grad += 2 * P.T @ err / len(P)
                return float(loss), grad
            w = np.zeros(features.shape[1]); w[0] = .2
            result = minimize(objective, w, jac=True, bounds=local_bounds, method='L-BFGS-B', options={'maxiter': 1500, 'ftol': 1e-12, 'gtol': 1e-8})
            if not result.success:
                result = minimize(objective, result.x, jac=True, bounds=local_bounds, method='SLSQP', options={'maxiter': 1500, 'ftol': 1e-11})
            if not result.success: raise ValueError(result.message)
            full = np.zeros(X.shape[1]); full[active] = result.x
            cache[key] = full, np.clip(anchor + X @ full, 0, 1)
            return cache[key]
        def choose(train, grouped=False):
            folds = [[i for i in train if flags[i]['album'] == g] for g in sorted({flags[i]['album'] for i in train})] if grouped else [[i] for i in train]
            scores = []
            for reg in GRID:
                errors = []
                for held in folds:
                    _, pred = fit([i for i in train if i not in held], reg)
                    errors.extend(miss(pred[flags[i]['index']], flags[i]) for i in held)
                scores.append((float(np.mean(errors)), -reg, reg))
            return min(scores)[2]
        validations = {}
        for grouped in [False, True]:
            results = []
            folds = [[i for i in range(len(flags)) if flags[i]['album'] == g] for g in sorted({f['album'] for f in flags})] if grouped else [[i] for i in range(len(flags))]
            for held in folds:
                train = [i for i in range(len(flags)) if i not in held]; reg = choose(train, grouped)
                _, pred = fit(train, reg)
                results.extend(dict(row(pred[flags[i]['index']], flags[i]), regularization=reg) for i in held)
            validations['nestedAlbumOut' if grouped else 'nestedSongOut'] = {'held': sum(r['holds'] for r in results), 'meanMiss': float(np.mean([r['miss'] for r in results])), 'rows': results}
        reg = choose(list(range(len(flags)))); w, pred = fit(list(range(len(flags))), reg)
        model = {'name': name, 'features': description, 'coefficients': w.tolist(), 'regularization': reg, **validations,
                 'residualAroundCurrent': residual, 'stabilityWeight': drift_weight,
                 'training': [row(pred[f['index']], f) for f in flags], 'laterRetrospective': [] if a.include_later_in_cv else [row(pred[f['index']], f) for f in later],
                 'unratedAlexius': float(pred[hashes.index(byname['09 Alexius']['hash'])]),
                 'meanLibraryMovement': float(np.mean(abs(pred-current))), 'atLeastUnderstandable': int((pred >= .75).sum()),
                 'endpointScores': int(((pred < .05) | (pred > .95)).sum()), 'scores': dict(zip(hashes, pred.tolist()))}
        report['models'].append(model)
        output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
        print(name, 'train', sum(r['holds'] for r in model['training']), 'song', model['nestedSongOut']['held'], round(model['nestedSongOut']['meanMiss'], 4), 'album', model['nestedAlbumOut']['held'], round(model['nestedAlbumOut']['meanMiss'], 4), 'later', sum(r['holds'] for r in model['laterRetrospective']), flush=True)
    print('DONE: no app/model changes', flush=True)


if __name__ == '__main__': main()
