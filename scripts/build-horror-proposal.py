"""Pack existing, frozen experiments into a small comparison; no fitting."""
import argparse
import hashlib
import json
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    base = parser.parse_args().directory
    raw = (base / 'frozen-graded-horror-pilot.json').read_bytes()
    if hashlib.sha256(raw).hexdigest() != '28016d98aaa57e94a0ec1ac270cbd301dcd8f6427795fb28ca43ee33433a1d36':
        raise ValueError('Frozen candidate has changed')
    frozen = json.loads(raw)
    snapshot = json.loads((base / 'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    current = next(m for m in snapshot['moods'] if m['id'] == 'grim')['scores']
    byname = {e['name']: e for e in snapshot['entries']}
    latest = json.loads((base / 'frozen-graded-horror-independent-evaluation.json').read_text(encoding='utf-8'))
    earlier = json.loads((base / 'frozen-horror-independent-evaluation.json').read_text(encoding='utf-8'))
    audit = json.loads((base / 'frozen-pilot-audit.json').read_text(encoding='utf-8'))
    labels = {
        'Forlorn Cliffs': 'Definitely; above Burial Isle',
        "Kyne's Peace": 'No',
        'Dawn': 'No',
        'Lead Your Fights': 'Stretch or lower',
        'Temple of Woedica': 'Stretch to Understandable',
        'Burial Isle': 'Understandable; below Forlorn',
        'Wind from the Depths': 'Understandable or higher',
        'The Cove Battle': 'Understandable or higher',
    }
    known = {r['song']: r for r in frozen['knownJudgments']}
    checked = {r['song']: r for r in latest['results'] if r['rated']}
    data = {
        'examples': {'summary': 'Fitted examples · 14/14 target intervals held in training · not an independent accuracy test', 'rows': []},
        'independent': {'summary': 'Current candidate · 3/3 rated songs held; 1 unrated · current word scorer also held 3/3 · no refitting', 'rows': []},
        'earlier': {'summary': 'Previous candidate · 3/4 graded judgments held · Burial Isle was overstated · these answers later trained the current candidate', 'rows': []},
        'library': {'summary': f"235 songs scored · {audit['thresholdCounts']['0.75']} at least Understandable across {audit['strongAlbums']} albums · unreviewed predictions", 'rows': []},
    }
    for name, label in labels.items():
        row = known[name]
        data['examples']['rows'].append({'song': name, 'current': current[byname[name]['hash']], 'score': row['score'], 'label': label, 'result': 'Fits (trained)'})
    for row in latest['results']:
        data['independent']['rows'].append({'song': row['song'], 'current': row['current'], 'score': row['score'], 'label': row.get('label', 'Not rated'), 'result': 'Fits (unseen)' if row.get('holds') else 'Unrated'})
    for row in earlier['results']:
        data['earlier']['rows'].append({'song': row['song'], 'current': row['current'], 'score': row['score'], 'label': row['label'], 'result': 'Fits (unseen)' if row['bandHolds'] else 'Above limit'})
    for row in audit['biggestChanges']:
        name = row['song']
        label = checked[name]['label'] if name in checked else labels.get(name, 'Not rated')
        result = 'Fits (trained)' if name in known else 'Fits (unseen)' if name in checked and checked[name]['holds'] else 'Unreviewed'
        data['library']['rows'].append({'song': name, 'current': row['current'], 'score': row['pilot'], 'label': label, 'result': result})
    path = base / 'horror-proposal.html'
    text = path.read_text(encoding='utf-8')
    start = '<script type="application/json" id="horror-proposal-data">'
    before, rest = text.split(start, 1)
    _, after = rest.split('</script>', 1)
    path.write_text(before + start + json.dumps(data, ensure_ascii=True).replace('<', '\\u003c') + '</script>' + after, encoding='utf-8')
    print('Packed frozen comparison:', {key: len(value['rows']) for key, value in data.items()})


if __name__ == '__main__':
    main()
