"""Package joint text/audio inference, without song-specific scoring rules.

The architecture was explored after the latest four answers; those answers
were never added to training. Their results are diagnostic, not a blind test.
"""
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
import sys
import numpy as np

base = Path(sys.argv[1])
study = json.loads((base / 'signed-prompts-vs-audio-fusion-directional.json').read_text(encoding='utf-8'))
model = next(m for m in study['models'] if m['name'] == 'fusion-audio-text-lines-linear')
source = json.loads((base / 'publisher-audio-evidence.json').read_text(encoding='utf-8'))
axes = json.loads((base / 'signed-prompt-axis-scores.json').read_text(encoding='utf-8'))
weights, heads = {}, []
for i, head in enumerate(source['heads']):
    members = []
    for j, member in enumerate(head['models']):
        key = f'emotion_{i}_{j}'
        weights[key] = np.asarray(member['coefficient'], dtype=np.float64)
        members.append(dict(member, coefficient=key))
    heads.append({'label': head['label'], 'members': members})
np.savez_compressed(base / 'horror-fusion-evidence.npz', **weights)
profile = {
    'schema': 'lutedrop.experimental-joint-mood-fit/v1',
    'createdAt': datetime.now(timezone.utc).isoformat(),
    'model': 'OpenMuQ/MuQ-MuLan-large', 'modelRevision': '2e01c796b71dca71b45251384c04cd7b237c9020',
    'sampleRate': 24000, 'windowSeconds': 10, 'maxWindows': 8,
    'heads': heads,
    'textAxes': [{'label': a['label'], 'prompts': a['prompts'], 'reference': a['reference']} for a in axes['axes'] if a['mode'] == 'lines'],
    'coefficients': model['coefficients'],
    'weightsSha256': hashlib.sha256((base / 'horror-fusion-evidence.npz').read_bytes()).hexdigest(),
    'meaning': 'Personal Horror fit; not probability or corpus percentile.',
    'trainingJudgments': study['developmentFlags'],
    'evidence': {'trainingHeld': sum(r['holds'] for r in model['training']), 'trainingTotal': len(model['training']),
                 'nestedSongOut': model['nestedSongOut'], 'nestedAlbumOut': model['nestedAlbumOut']},
    'limitations': ['Architecture explored after the latest four human judgments; no independent-validation claim for those songs.',
                    'Those four songs remain excluded from fitting and hyperparameter selection.',
                    'Small, repeatedly studied development set; internal validation is exploratory.',
                    'No automatic update on importing songs, playing music, or changing flags.'],
    'status': 'experimental; not deployed',
}
out = base / 'horror-fusion-candidate.json'
if out.exists(): raise ValueError('Do not overwrite an exported candidate')
out.write_text(json.dumps(profile, indent=2) + '\n', encoding='utf-8', newline='\n')
print('Exported joint candidate:', hashlib.sha256(out.read_bytes()).hexdigest())
