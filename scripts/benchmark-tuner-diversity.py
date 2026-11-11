"""Compare search revisions on saved, analyzer-verified library/reference scores.

No model approximation or app writes: reference moments reconstruct the exact
mean/std of every tested prompt combination. Does not test new comma embeddings.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import sys
import time
import types

import numpy as np
from scipy.linalg import helmert

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tagger'))
from tagger import tune


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('report', type=Path)
    p.add_argument('--out', type=Path, required=True)
    a = p.parse_args()
    d = json.loads(a.report.read_text(encoding='utf-8'))
    old = types.ModuleType('previous_tune')
    sys.modules[old.__name__] = old
    exec(subprocess.check_output(['git', 'show', 'HEAD:tagger/tagger/tune.py'], cwd=ROOT, text=True), old.__dict__)
    U = np.array(d['numericBasis']['raw'])
    mu = np.array(d['numericBasis']['referenceMean'])
    eig, vectors = np.linalg.eigh(d['numericBasis']['referenceCovariance'])
    n = len(U)
    R = mu[:, None] + math.sqrt(n + 1) * ((vectors * np.sqrt(np.maximum(eig, 0))[None, :]) @ helmert(n + 1, full=False))
    units = [s['line'] for s in d['selected']]
    hashes = [e['hash'] for e in d['entries']]
    flags = [tune.FlagTarget(hashes.index(f['hash']), f['min'], f['max'], f['weight'], f['fresh'], f['side'], f.get('focus', False)) for f in d['flags']]
    current = next(m['prompts'] for m in d['vocabulary']['moods'] if m['id'] == 'grim')
    seed = tuple(units.index(line) for line in current)
    mapping = tune.Mapping(.674, math.log(3) / (1.2816 - .674), 'reference')
    before = tune._combo_bars(U, R, list(seed), mapping)
    output = {'source': str(a.report), 'songs': len(hashes), 'units': n, 'runs': []}
    for label, module in [('previous', old), ('revised', tune)]:
        start = time.monotonic()
        found = module.beam_search(U, R, units, flags, mapping, seeds=[seed], before=before)
        results = []
        for c in found:
            b = tune._combo_bars(U, R, [units.index(line) for line in c.lines], mapping)
            results.append({'lines': c.lines, 'loss': c.loss, 'held': len(flags) - len(tune.broken(b, flags)), 'stability': tune.stability(b, before, flags)})
        output['runs'].append({'revision': label, 'seconds': time.monotonic() - start, 'results': results})
    a.out.write_text(json.dumps(output, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(output, indent=2))


if __name__ == '__main__':
    main()
