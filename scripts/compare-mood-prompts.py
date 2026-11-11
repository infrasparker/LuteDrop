"""Read-only prompt experiments against the live analyzer's cached music.

python scripts/compare-mood-prompts.py --out <results.json>
python scripts/compare-mood-prompts.py --candidates <experiment.json> --top 20

Reads credentials from the app's DB; never prints/saves them. No vocabulary,
calibration, manual scores or flags are changed. Add named prompt lists to the
experiment JSON. Compare rank correlation and top-track overlap with another
mood, plus existing user roles and mood flags as review aids, not ground truth.
Raw similarities of different prompt sets are not comparable calibrated bars.
"""
from contextlib import closing
import argparse
import json
import os
from pathlib import Path
import sqlite3
import statistics
import urllib.request


def ranks(values):
    ordered = sorted(enumerate(values), key=lambda pair: pair[1])
    result = [0.0] * len(values)
    start = 0
    while start < len(ordered):
        end = start + 1
        while end < len(ordered) and ordered[end][1] == ordered[start][1]:
            end += 1
        rank = (start + end - 1) / 2
        for index, _ in ordered[start:end]:
            result[index] = rank
        start = end
    return result


def correlation(a, b):
    if len(a) < 2 or len(set(a)) < 2 or len(set(b)) < 2:
        return None
    return statistics.correlation(ranks(a), ranks(b))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, default=Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db')
    parser.add_argument('--candidates', type=Path, default=Path(__file__).with_name('mood-prompt-experiments.json'))
    parser.add_argument('--out', type=Path)
    parser.add_argument('--top', type=int, default=15)
    args = parser.parse_args()
    if args.top < 1:
        parser.error('--top must be positive')
    experiment = json.loads(args.candidates.read_text(encoding='utf-8'))
    with closing(sqlite3.connect(args.db.resolve().as_uri() + '?mode=ro', uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        meta = dict(conn.execute('SELECT key,value FROM meta').fetchall())
        vocabulary = json.loads(meta['mood_vocabulary'])
        moods = {m['id']: m for m in vocabulary['moods']}
        mood_id = experiment['mood']
        other_id = experiment['compareTo']
        tracks = [dict(row) for row in conn.execute("SELECT id,name,file_path,content_hash,COALESCE(role_user,'nonCombat') role FROM library_entries WHERE entry_type='music' AND content_hash IS NOT NULL")]
        flags = [dict(row) for row in conn.execute('SELECT e.name,e.content_hash,f.kind,f.band,f.bound FROM mood_flags f JOIN library_entries e ON e.id=f.library_entry_id WHERE f.mood=?', (mood_id,))]
    targets = sorted({t['content_hash'] for t in tracks})
    def preview(lines):
        if not lines or any(not isinstance(line, str) or not line.strip() for line in lines):
            raise ValueError('Every candidate needs nonempty prompt lines')
        request = urllib.request.Request(meta['autotag_url'].rstrip('/') + '/preview',
            data=json.dumps({'prompts': lines, 'targets': targets}).encode(),
            headers={'Content-Type': 'application/json', 'X-LuteDrop-Token': meta['autotag_token']})
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.load(response)
    baseline = preview(moods[other_id]['prompts'])
    variants = {'Current ' + moods[mood_id]['name']: moods[mood_id]['prompts'], **experiment['candidates']}
    report = {'mood': mood_id, 'compareTo': other_id, 'libraryTracks': len(tracks), 'targetHashes': len(targets), 'results': []}
    for name, lines in variants.items():
        scores = preview(lines)
        common = sorted(set(scores) & set(baseline))
        ordered = sorted((t for t in tracks if t['content_hash'] in scores), key=lambda t: (-scores[t['content_hash']], t['name']))
        top = ordered[:args.top]
        other_top = sorted((t for t in tracks if t['content_hash'] in baseline), key=lambda t: -baseline[t['content_hash']])[:args.top]
        rank_by_hash = {t['content_hash']: i + 1 for i, t in reversed(list(enumerate(ordered)))}
        result = {'name': name, 'prompts': lines, 'scoredTracks': len(ordered),
            'rankCorrelation': correlation([scores[h] for h in common], [baseline[h] for h in common]),
            'topOverlap': len({t['id'] for t in top} & {t['id'] for t in other_top}),
            'topCombat': sum(t['role'] == 'combat' for t in top),
            'flags': [{**f, 'rank': rank_by_hash.get(f['content_hash'])} for f in flags],
            'top': [{'name': t['name'], 'album': Path(t['file_path']).parent.name, 'role': t['role'], 'raw': scores[t['content_hash']]} for t in top]}
        report['results'].append(result)
        rho = result['rankCorrelation']
        rho_text = 'n/a' if rho is None else f'{rho:.3f}'
        print(f"{name}: {len(ordered)} tracks; correlation with {other_id} {rho_text}; top-{len(top)} overlap {result['topOverlap']}; combat {result['topCombat']}", flush=True)
        print('  ' + ' / '.join(t['name'] for t in top[:5]), flush=True)
        print('  Flags: ' + ', '.join(f"{f['name']} #{f['rank']}" for f in result['flags']), flush=True)
    if args.out:
        args.out.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
