"""Build broad, fixed publisher evidence; never train on personal library flags."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import roc_auc_score
from sklearn.model_selection import StratifiedGroupKFold


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    a = p.parse_args(); root = Path(__file__).resolve().parents[1]
    spec = importlib.util.spec_from_file_location('history', root/'scripts/test-historical-horror-generalization.py')
    history = importlib.util.module_from_spec(spec); spec.loader.exec_module(history)
    manifest = json.loads((root/'tagger/tagger/reference/manifest.json').read_text())
    with np.load(a.directory/'preprojection-reference-features.npz') as z:
        R = z['means'].astype(float); layers = [3, 6, 9, 12]
    destination = root/'tagger/tagger/reference'
    old = json.loads((destination/'audio-evidence.json').read_text())
    with np.load(destination/'audio-evidence.npz') as z:
        weights = {k: z[k].copy() for k in z.files}
    heads = old['heads'][:]
    groups = np.array([e['title'].split()[0].lower() for e in manifest])
    labels = sorted({tag for e in manifest for tag in e['feel']})
    for label in labels:
        if any(h['label'] == label for h in heads): continue
        y = np.array([int(label in e['feel']) for e in manifest])
        if min(sum(y), sum(1-y)) < 8: continue
        members = []; predictions = []
        for j, layer in enumerate(layers):
            X = history.unit(R[:, j]); oof = np.zeros(len(X))
            for train, test in StratifiedGroupKFold(5, shuffle=True, random_state=104729).split(X, y, groups):
                clf = LogisticRegression(C=1., class_weight='balanced', max_iter=2500, random_state=0).fit(X[train], y[train])
                oof[test] = clf.decision_function(X[test])
            cal = history.platt(oof, y)
            clf = LogisticRegression(C=1., class_weight='balanced', max_iter=2500, random_state=0).fit(X, y)
            key = f'head_{len(heads)}_{j}'; weights[key] = clf.coef_[0]
            members.append(dict(layer=layer, coefficient=key, intercept=float(clf.intercept_[0]), calibration=cal.tolist()))
            predictions.append(oof)
        heads.append(dict(label=label, members=members, sourcePositives=int(sum(y)), crossfitAuc=float(roc_auc_score(y, np.mean(predictions, axis=0)))))
        print(label, int(sum(y)), flush=True)
    np.savez_compressed(destination/'audio-evidence.npz', **weights)
    digest = hashlib.sha256((destination/'audio-evidence.npz').read_bytes()).hexdigest()
    metadata = dict(old, id=f'muq-feel-v2-{digest[:12]}', heads=heads, weightsSha256=digest,
                    compatibleEvidence=old.get('compatibleEvidence', [old['id']]))
    (destination/'audio-evidence.json').write_text(json.dumps(metadata, indent=2)+'\n')
    print('Bundled', len(heads), 'publisher-trained signals', metadata['id'])


if __name__ == '__main__': main()
