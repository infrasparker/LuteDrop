"""Run saved joint mood weights; no fitting, LLM, or app/database writes.

Input is actual pooled hidden audio features plus actual analyzer /score raw
values for the saved text axes. Content hashes only align these cached inputs;
no song identities, stored target scores or training flags enter prediction.
"""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np
from scipy.special import expit


def predict(profile, weights, hidden, layers, text_raw):
    hidden = np.asarray(hidden, dtype=float)
    text_raw = np.asarray(text_raw, dtype=float)
    if hidden.ndim != 3 or hidden.shape[2] != 1024 or text_raw.shape != (len(hidden), len(profile['textAxes'])):
        raise ValueError('Unexpected audio or text feature shape')
    if not np.isfinite(hidden).all() or not np.isfinite(text_raw).all():
        raise ValueError('Features must be finite')
    features = []
    for head in profile['heads']:
        members = []
        for member in head['members']:
            if member['layer'] not in layers: raise ValueError('Required hidden layer missing')
            H = hidden[:, layers.index(member['layer'])]
            norms = np.linalg.norm(H, axis=1, keepdims=True)
            if (norms < 1e-12).any(): raise ValueError('Empty hidden audio feature')
            H = H / norms
            A, B = member['calibration']
            members.append(expit(A * (H @ weights[member['coefficient']] + member['intercept']) + B))
        features.append(np.mean(members, axis=0))
    for j, axis in enumerate(profile['textAxes']):
        stat = axis['reference']
        if stat['sd'] <= 0: raise ValueError('Invalid frozen text calibration')
        features.append((text_raw[:, j] - stat['mean']) / stat['sd'])
    X = np.column_stack([np.ones(len(hidden))] + features)
    coefficients = np.asarray(profile['coefficients'])
    if coefficients.shape != (X.shape[1],) or not np.isfinite(coefficients).all():
        raise ValueError('Invalid joint coefficients')
    return np.clip(X @ coefficients, 0, 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--features', type=Path, help='New audio export; defaults to existing research features')
    parser.add_argument('--text', type=Path, help='Actual analyzer word-axis export for the same content hashes')
    parser.add_argument('--out', type=Path, required=True)
    a = parser.parse_args(); base = a.directory
    profile = json.loads((base / 'horror-fusion-candidate.json').read_text(encoding='utf-8'))
    path = base / 'horror-fusion-evidence.npz'
    if hashlib.sha256(path.read_bytes()).hexdigest() != profile['weightsSha256']: raise ValueError('Evidence weights changed')
    with np.load(path, allow_pickle=False) as z: weights = {k: z[k].copy() for k in z.files}
    with np.load(a.features or base / 'preprojection-features.npz', allow_pickle=False) as z:
        hidden, layers, hashes = z['means'], z['layers'].tolist(), z['hashes'].tolist()
    text = json.loads((a.text or base / 'signed-prompt-axis-scores.json').read_text(encoding='utf-8'))
    if text['hashes'] != hashes or text['model'] != 'muq-mulan-large': raise ValueError('Text/audio inputs do not align')
    axes = [a for a in text['axes'] if a['mode'] == 'lines']
    if [a['prompts'] for a in axes] != [a['prompts'] for a in profile['textAxes']]: raise ValueError('Text prompts changed')
    raw = np.column_stack([axis['raw'] for axis in axes])
    scores = predict(profile, weights, hidden, layers, raw)
    a.out.write_text(json.dumps({'meaning': profile['meaning'], 'scores': [{'hash': h, 'fit': float(v)} for h, v in zip(hashes, scores)]}, indent=2) + '\n', encoding='utf-8')
    print('Joint inference:', len(scores), 'tracks; no fitting or app writes')


if __name__ == '__main__': main()
