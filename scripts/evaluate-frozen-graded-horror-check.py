"""Evaluate later judgments without changing the frozen pilot or its training.

Stretch is an upper limit, not a required amount of Horror. Alexius was not
rated in the reply and must not count as either a success or a failure.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path


FROZEN_SHA = '28016d98aaa57e94a0ec1ac270cbd301dcd8f6427795fb28ca43ee33433a1d36'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    base = parser.parse_args().directory
    blob = (base / 'frozen-graded-horror-pilot.json').read_bytes()
    if hashlib.sha256(blob).hexdigest() != FROZEN_SHA:
        raise ValueError('The frozen pilot has changed; do not evaluate a refitted candidate')
    frozen = json.loads(blob)
    snapshot = json.loads((base / 'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    byname = {e['name']: e for e in snapshot['entries']}
    baseline = next(m for m in snapshot['moods'] if m['id'] == 'grim')['scores']
    judgments = {
        '21 The Inquisition Marches': ('Stretch or lower', .75),
        '23 Arkemyr': ('No', .5),
        next(n for n in frozen['blindSongs'] if 'Periki' in n): ('Stretch or lower; leaning lower', .75),
    }
    results = []
    for name in frozen['blindSongs']:
        h = byname[name]['hash']
        row = {'song': name, 'score': frozen['scores'][h], 'current': baseline[h], 'rated': name in judgments}
        if name in judgments:
            label, ceiling = judgments[name]
            row.update(label=label, min=None, max=ceiling,
                       holds=row['score'] <= ceiling, currentHolds=row['current'] <= ceiling)
        results.append(row)
    rated = [r for r in results if r['rated']]
    report = {
        'evaluatedAt': datetime.now(timezone.utc).isoformat(),
        'frozenModelByteSha256': FROZEN_SHA,
        'refittedAfterFeedback': False,
        'semantics': {'stretch': {'min': None, 'max': .75}, 'understandable': {'min': .75, 'max': None}},
        'results': results,
        'summary': {'ratedSongs': len(rated), 'held': sum(r['holds'] for r in rated),
                    'currentHeld': sum(r['currentHolds'] for r in rated), 'unratedSongs': len(results)-len(rated)},
        'limitations': [
            'Three judgments are a small independent check, not an estimate of library-wide accuracy.',
            'This mostly negative check does not measure sensitivity to unseen positive songs.',
            'The first pilot overestimated Burial Isle; the revised pilot used that feedback in training.',
            'Training fits, internal cross-validation and later blind judgments are different evidence.',
        ],
    }
    output = base / 'frozen-graded-horror-independent-evaluation.json'
    output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report['summary']))
    for row in results:
        print(row['song'], 'pilot', round(row['score'], 3), 'current', round(row['current'], 3),
              row.get('label', 'Unrated'), row.get('holds', ''))


if __name__ == '__main__':
    main()
