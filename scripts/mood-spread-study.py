"""Read-only study of mood coverage and whether the analyzer follows word meaning.

python scripts/mood-spread-study.py --out <report.json>

Compares bare words, 'X music', literal definitions, and semantic synonym sets.
All are scored against the analyzer's fixed reference set, never against the
best-looking tracks in this library. The conservative paired fit is the lower
of 'X music' and its synonym-set score (definition when no synonyms are supplied).
The 0.75 diagnostic is the app's reference-calibrated strong band, not a truth
label or a probability. Reports all scores, rank correlations,
top tracks, current coverage gaps, and conservative candidate gains. No app
settings or library rows are changed. Definitions are explicit hypotheses,
not human listening labels; strong model scores alone do not prove semantic fit.
"""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import json
import math
import os
from pathlib import Path
import sqlite3
import urllib.request

spec = importlib.util.spec_from_file_location('prompt_comparison', Path(__file__).with_name('compare-mood-prompts.py'))
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


def calibrated(raw, stats):
    if stats is None or stats['sd'] < 1e-12:
        return None
    z = (raw - stats['mean']) / stats['sd']
    slope = math.log(3) / (1.2816 - .674)
    return 1 / (1 + math.exp(max(-700, min(700, -slope * (z - .674)))))


def letter_id(index):
    return 'p' + chr(97 + index // 26) + chr(97 + index % 26)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, default=Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db')
    parser.add_argument('--words', type=Path, default=Path(__file__).with_name('mood-spread-words.json'))
    parser.add_argument('--phrases', type=Path, default=Path(__file__).with_name('mood-spread-phrases.json'))
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    definitions = json.loads(args.words.read_text(encoding='utf-8'))
    phrases = json.loads(args.phrases.read_text(encoding='utf-8'))
    with closing(sqlite3.connect(args.db.resolve().as_uri() + '?mode=ro', uri=True)) as conn:
        conn.row_factory = sqlite3.Row
        conn.execute('BEGIN')  # One consistent snapshot even while the app is in use.
        meta = dict(conn.execute('SELECT key,value FROM meta').fetchall())
        vocabulary = json.loads(meta['mood_vocabulary'])
        tracks = [dict(r) for r in conn.execute("SELECT id,name,file_path,content_hash,COALESCE(role_user,'nonCombat') role FROM library_entries WHERE entry_type='music'")]
        stored = {t['id']: {} for t in tracks}
        for r in conn.execute("SELECT library_entry_id,mood,COALESCE(user_level,auto_score) score FROM library_entry_moods WHERE mood!='energy'"):
            if r['library_entry_id'] in stored:
                stored[r['library_entry_id']][r['mood']] = r['score']
    for track in tracks:
        track['album'] = Path(track.pop('file_path')).parent.name
        track['stored'] = stored[track['id']]
        track['storedMax'] = max((x for x in track['stored'].values() if x is not None), default=0)
    targets = sorted({t['content_hash'] for t in tracks if t['content_hash']})
    variants = []
    for mood in vocabulary['moods']:
        variants.append({'key': 'current:' + mood['id'], 'name': mood['name'], 'kind': 'current', 'prompts': mood['prompts']})
    for word, definition in definitions.items():
        variants.append({'key': 'word:' + word, 'name': word, 'kind': 'word', 'prompts': [word.lower()]})
        variants.append({'key': 'music:' + word, 'name': word, 'kind': 'music', 'prompts': [word.lower() + ' music']})
        # Word plus a definition, avoiding instruments/locations/track titles as shortcuts.
        variants.append({'key': 'meaning:' + word, 'name': word, 'kind': 'meaning', 'prompts': ['Music that feels ' + word.lower() + '. ' + definition]})
        if word in phrases:
            variants.append({'key': 'phrases:' + word, 'name': word, 'kind': 'phrases', 'prompts': [phrases[word] + ' music']})
    reference = None
    for offset in range(0, len(variants), 24):
        batch = variants[offset:offset + 24]
        request = urllib.request.Request(meta['autotag_url'].rstrip('/') + '/score',
            data=json.dumps({'targets': targets, 'mood_prompts': {letter_id(i): v['prompts'] for i, v in enumerate(batch)}, 'energy_prompts': vocabulary['energy']}).encode(),
            headers={'Content-Type': 'application/json', 'X-LuteDrop-Token': meta['autotag_token']})
        with urllib.request.urlopen(request, timeout=120) as response:
            answer = json.load(response)
        ref = answer.get('mood_reference')
        if not ref:
            raise RuntimeError('Fixed reference statistics are required for comparable candidate scores')
        current_reference = {'set': ref['set'], 'tracks': ref['tracks']}
        if reference is not None and current_reference != reference:
            raise RuntimeError('Reference set changed during the study; rerun for a consistent comparison')
        reference = current_reference
        for i, variant in enumerate(batch):
            key = letter_id(i)
            stat = ref['stats'][key]
            variant['referenceStats'] = stat
            variant['raw'] = {h: r['moods'][key] for h, r in answer['mood'].items()}
            variant['bars'] = {h: calibrated(raw, stat) for h, raw in variant['raw'].items()}
        print(f'Scored {min(offset+24,len(variants))}/{len(variants)} prompt sets', flush=True)
    by_key = {v['key']: v for v in variants}
    current = [v for v in variants if v['kind'] == 'current']
    common = [h for h in targets if all(h in v['raw'] for v in variants)]
    if not common:
        raise RuntimeError('No music embeddings were scored consistently')
    rank_maps = {}
    for v in current:
        rank_maps[v['key']] = {h: i+1 for i,h in enumerate(sorted(common,key=lambda h:-v['raw'][h]))}
    for t in tracks:
        h = t['content_hash']
        t['bestCurrentRank'] = min((r.get(h,len(common)+1) for r in rank_maps.values()), default=len(common)+1)
        t['currentMax'] = max((v['bars'].get(h,0) for v in current), default=0)
    candidates = []
    for word, meaning in definitions.items():
        bare, defined = by_key['word:' + word], by_key['meaning:' + word]
        musical = by_key['music:' + word]
        anchor = by_key.get('phrases:' + word, defined)
        paired = {h: min(musical['bars'][h], anchor['bars'][h]) for h in common}
        similarities = sorted([{'name': v['name'], 'correlation': comparison.correlation([paired[h] for h in common], [v['raw'][h] for h in common])} for v in current],key=lambda x: -(x['correlation'] or 0))
        ordered = sorted([t for t in tracks if t['content_hash'] in paired],key=lambda t:-paired[t['content_hash']])
        robust_high = [t for t in ordered if paired[t['content_hash']] >= .75]
        gaps = [t for t in robust_high if t['currentMax'] < .75]
        overlooked = [t for t in ordered[:20] if paired[t['content_hash']] >= .75 and t['bestCurrentRank'] > 20]
        result = {'name':word,'meaning':meaning, 'wordMeaningCorrelation':comparison.correlation([bare['raw'][h] for h in common],[defined['raw'][h] for h in common]),
            'musicAnchorCorrelation':comparison.correlation([musical['raw'][h] for h in common],[anchor['raw'][h] for h in common]),
            'similarTo':similarities[:3], 'robustHigh':len(robust_high),'gapsReached':len(gaps),'overlookedReached':len(overlooked),
            'gapTracks':[t['id'] for t in gaps],'overlookedTracks':[t['id'] for t in overlooked],
            'top':[{'id':t['id'],'name':t['name'],'album':t['album'],'fit':paired[t['content_hash']],'word':bare['bars'][t['content_hash']],'music':musical['bars'][t['content_hash']],'anchor':anchor['bars'][t['content_hash']],'meaning':defined['bars'][t['content_hash']],'currentBest':t['currentMax'],'bestCurrentRank':t['bestCurrentRank']} for t in ordered[:15]]}
        candidates.append(result)
    report = {'createdAt':datetime.now(timezone.utc).isoformat(), 'libraryTracks':len(tracks),'scoredHashes':len(common),'reference':reference,
        'vocabulary':vocabulary,'tracks':tracks,'variants':variants,'candidates':candidates,
        'summary':{'belowPoint75':sum(t['currentMax'] < .75 for t in tracks),'outsideAllTop20':sum(t['bestCurrentRank']>20 for t in tracks)}}
    args.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report['summary']))
    for c in sorted(candidates,key=lambda c:(-c['overlookedReached'],-c['gapsReached'])):
        print(f"{c['name']:15} music/synonym agreement {c['musicAnchorCorrelation']:.2f} | robust high {c['robustHigh']:3} | weak coverage {c['gapsReached']:2} | newly surfaced top20 {c['overlookedReached']:2} | like {c['similarTo'][0]['name']} {c['similarTo'][0]['correlation']:.2f} | " + ' / '.join(t['name'] for t in c['top'][:3]))


if __name__ == '__main__':
    main()
