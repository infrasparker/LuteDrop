"""Actually compare subword sets against the CURRENT app vocabulary and flags.

Read-only: snapshot the app DB in memory, call the live analyzer /score and
/tune endpoints, and save results only to --out. Never accepts proposals or
updates vocabulary. Default: compare every line/word ablation and curated
alternative, then ask /tune to arrange the mood's own semantic word pool.
"""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import time

ROOT = Path(__file__).resolve().parents[1]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


export = module('fixture_export', ROOT / 'ui-preview/export_fixtures.py')
spread = module('spread_study', ROOT / 'scripts/mood-spread-study.py')


def ablations(lines):
    if len(lines) > 1:
        for i, line in enumerate(lines):
            yield 'Remove line: ' + line, lines[:i] + lines[i+1:]
    for i, line in enumerate(lines):
        words = [word.strip() for word in line.split(',') if word.strip()]
        if len(words) < 2:
            continue
        for j, word in enumerate(words):
            changed = ', '.join(words[:j] + words[j+1:])
            yield 'Remove subword: ' + word, lines[:i] + [changed] + lines[i+1:]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--db', type=Path, default=Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--no-arrange', action='store_true')
    args = parser.parse_args()
    alternatives = json.loads((ROOT / 'scripts/mood-set-alternatives.json').read_text(encoding='utf-8'))
    with closing(sqlite3.connect(args.db.resolve().as_uri() + '?mode=ro', uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn)
        meta = export.meta(conn)
        vocabulary = export.vocabulary(meta)
        health = export.post(meta['autotag_url'].rstrip('/'), meta['autotag_token'], '/health', None, 15)
        if not health.get('ready'):
            raise RuntimeError('Live analyzer is not ready')
        targets = export.music_hashes(conn)
        entries = [{'id': r[0], 'name': r[1], 'album': export.album_of(r[2]), 'hash': r[3]}
            for r in conn.execute("SELECT id,name,file_path,content_hash FROM library_entries WHERE entry_type='music' ORDER BY name,id")]
        songs = {e['hash']: e for e in entries}
        report = {'capturedAt':datetime.now(timezone.utc).isoformat(), 'model':health['model'], 'reference':health.get('reference'),
            'vocabulary':vocabulary, 'entries':entries, 'calls':[], 'moods':[]}

        def post(endpoint, body, timeout=120):
            started = time.monotonic()
            result = export.post(meta['autotag_url'].rstrip('/'), meta['autotag_token'], endpoint, body, timeout)
            report['calls'].append({'endpoint':endpoint,'seconds':round(time.monotonic()-started,3),
                'promptGroups':len(body.get('mood_prompts',{})), 'mode':body.get('mode'), 'moodId':body.get('mood_id')})
            return result

        def save():
            args.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')

        variants = []
        for mood in vocabulary['moods']:
            seen = set()
            choices = [('CURRENT',mood['prompts']), *ablations(mood['prompts']),
                *((f'Alternative {i+1}', lines) for i, lines in enumerate(alternatives.get(mood['id'],[])))]
            for label, lines in choices:
                key = json.dumps(lines)
                if key in seen:
                    continue
                seen.add(key)
                variants.append({'moodId':mood['id'],'label':label,'lines':lines})
        for offset in range(0,len(variants),24):
            batch = variants[offset:offset+24]
            response = post('/score',{'targets':targets,'mood_prompts':{spread.letter_id(i):v['lines'] for i,v in enumerate(batch)},'energy_prompts':vocabulary['energy']})
            if not response.get('mood_reference'):
                raise RuntimeError('Comparable reference statistics missing')
            for i, variant in enumerate(batch):
                key=spread.letter_id(i)
                stat=response['mood_reference']['stats'][key]
                variant['scores']={h:spread.calibrated(result['moods'][key],stat) for h,result in response['mood'].items()}
            print(f'/score: {min(offset+24,len(variants))}/{len(variants)} actual current-set variants scored',flush=True)

        for mood in vocabulary['moods']:
            mid=mood['id']
            choices=[v for v in variants if v['moodId']==mid]
            current=next(v for v in choices if v['label']=='CURRENT')
            request, skipped, _=export.build_request(conn,meta,mid,'compare',[],[],[],focus='')
            flags=request['flags']
            # Screening only. The live /tune response below is the authoritative
            # app objective (including stability); no invented score is adopted.
            def screen(v):
                violations=[]
                for f in flags:
                    bar=v['scores'].get(f['hash'])
                    if bar is not None:
                        violations.append(f['weight']*(max(0,(f['min'] or 0)-bar)+max(0,bar-(1 if f['max'] is None else f['max']))))
                drift=sum(abs(v['scores'][h]-current['scores'][h]) for h in targets if h in v['scores'] and h in current['scores'])/max(1,len(targets))
                v['screenViolation']=sum(violations)/max(1,len(flags))
                v['meanDrift']=drift
                return v['screenViolation']+.25*drift
            for v in choices:
                screen(v)
            # Compare all hand-written sets plus up to four best small edits.
            manual=[v for v in choices if v['label'].startswith('Alternative')]
            edits=sorted([v for v in choices if v['label'].startswith('Remove')],key=screen)[:max(0,8-len(manual))]
            selected=manual+edits
            request['candidates']=[v['lines'] for v in selected]
            response=post('/tune',request)
            summary={'id':mid,'name':mood['name'],'currentLines':mood['prompts'],'flagCount':len(flags),'skippedFlags':skipped,
                'baseline':response['baseline'],'variants':choices,
                'comparisons':[{'label':v['label'],'result':r} for v,r in zip(selected,response['proposals'])], 'arrange':None}
            report['moods'].append(summary)
            save()
            base=response['baseline']
            better=[p for p in summary['comparisons'] if p['result']['loss'] < base['loss']-1e-9]
            print(f"/tune compare: {mood['name']} {len(selected)} complete word sets; baseline loss {base['loss']:.4f}; {len(better)} better by app objective",flush=True)
            if not args.no_arrange and flags:
                # Pool is deliberately restricted to this mood's current words
                # and reviewed semantic alternatives; not every other mood's words.
                pool=export.dedupe([*mood['prompts'],*[word.strip() for lines in alternatives.get(mid,[]) for line in lines for word in line.split(',')]])
                arranged,_,_=export.build_request(conn,meta,mid,'arrange',pool,[],[],focus='')
                arranged['other_moods']={}
                summary['arrange']=post('/tune',arranged,timeout=180)
                print(f"/tune arrange: {mood['name']} {len(summary['arrange']['proposals'])} improvements from semantic pool",flush=True)
                save()
        report['variantCount']=len(variants)
        report['completedAt']=datetime.now(timezone.utc).isoformat()
        save()
        for mood in report['moods']:
            best=sorted(mood['comparisons'],key=lambda x:x['result']['loss'])[:2]
            print('\n'+mood['name']+' CURRENT '+str(mood['currentLines']))
            for p in best:
                r=p['result'];held=sum(f['holds'] for f in r['flags']);print(f"  {p['label']} loss {r['loss']:.4f} flags {held}/{len(r['flags'])}: {r['lines']}")


if __name__=='__main__':
    main()
