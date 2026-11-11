"""Audit KEEP-current + ADD-Reflective against live prompts, overrides and flags.

Also evaluates replacing Serene with Reflective as a rejected counterfactual.
No app data or vocabulary is written. Uses live /score and /tune compare.
"""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import sqlite3

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('refine',ROOT/'scripts/refine-mood-sets.py')
refine=importlib.util.module_from_spec(spec)
spec.loader.exec_module(refine)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--db',type=Path,default=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db')
    args=parser.parse_args()
    candidates=json.loads((ROOT/'scripts/qualifier-set-alternatives.json').read_text(encoding='utf-8'))
    reflective=next(q for q in candidates if q['name']=='Reflective')['sets']
    with closing(sqlite3.connect(args.db.resolve().as_uri()+'?mode=ro',uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn)
        meta=refine.export.meta(conn);vocabulary=json.loads(meta['mood_vocabulary'])
        if any(m['id']=='reflective' for m in vocabulary['moods']):
            raise RuntimeError('Reflective now exists; review current state instead of assuming a new mood')
        health=refine.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/health',None,15)
        entries=[dict(zip(['id','name','hash','path'],r)) for r in conn.execute("SELECT id,name,content_hash,file_path FROM library_entries WHERE entry_type='music' ORDER BY name,id")]
        byhash={e['hash']:e for e in entries};targets=refine.export.music_hashes(conn)
        groups={m['id']:m['prompts'] for m in vocabulary['moods']};groups['trialreflective']=reflective
        response=refine.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/score',{'targets':targets,'mood_prompts':groups,'energy_prompts':vocabulary['energy']},120)
        stats=response['mood_reference']['stats']
        scores={m:{h:refine.spread.calibrated(r['moods'][m],stats[m]) for h,r in response['mood'].items()} for m in groups}
        stored={(eid,mid):{'auto':auto,'user':user} for eid,mid,auto,user in conn.execute('SELECT library_entry_id,mood,auto_score,user_level FROM library_entry_moods')}
        raw_flags={(eid,mid):{'kind':kind,'band':band,'bound':bound} for eid,mid,kind,band,bound in conn.execute('SELECT library_entry_id,mood,kind,band,bound FROM mood_flags')}
        plan=[{'id':m['id'],'name':m['name'],'action':'KEEP','prompts':m['prompts']} for m in vocabulary['moods']]
        plan.append({'id':'reflective','name':'Reflective','action':'ADD','prompts':reflective})
        flag_rows=[];mood_checks=[];wrong_rows=[]
        for mood in vocabulary['moods']:
            mid=mood['id']
            versions=[mood['prompts']]
            if mid=='peaceful':versions.append(reflective)
            request,skipped,_=refine.export.build_request(conn,meta,mid,'compare',[],[],versions,focus='')
            answer=refine.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/tune',request,120)
            unchanged=answer['proposals'][0]
            baseline_flags={f['hash']:f for f in answer['baseline']['flags']}
            mood_checks.append({'id':mid,'name':mood['name'],'flags':len(unchanged['flags']),'held':sum(f['holds'] for f in unchanged['flags']),'skipped':skipped})
            bounds={f['hash']:f for f in request['flags']}
            for flag in unchanged['flags']:
                e=byhash[flag['hash']];target=bounds[flag['hash']];raw=raw_flags.get((e['id'],mid),{})
                flag_rows.append({'entryId':e['id'],'song':e['name'].replace('\u00a0',' '),'moodId':mid,'mood':mood['name'],
                    'minimum':target['min'],'maximum':target['max'],'current':flag['before'],'proposed':flag['after'],
                    'currentHeld':baseline_flags[flag['hash']]['holds'],'proposedHeld':flag['holds'],'fresh':flag['fresh'],**raw})
            if mid=='peaceful':
                for flag in answer['proposals'][1]['flags']:
                    e=byhash[flag['hash']];target=bounds[flag['hash']]
                    wrong_rows.append({'entryId':e['id'],'song':e['name'].replace('\u00a0',' '),'mood':'Serene','current':flag['before'],'replaced':flag['after'],'held':flag['holds'],'minimum':target['min'],'maximum':target['max']})
        orchard=next(e for e in entries if e['name'].strip().lower()=='white orchard')
        track_rows=[]
        for e in entries:
            values=[]
            for m in plan:
                key='trialreflective' if m['action']=='ADD' else m['id'];s=stored.get((e['id'],m['id']),{})
                auto=scores[key][e['hash']];user=s.get('user');value=user if user is not None else auto
                f=next((f for f in flag_rows if f['entryId']==e['id'] and f['moodId']==m['id']),None)
                values.append({'id':m['id'],'name':m['name'],'action':m['action'],'current':s.get('user') if s.get('user') is not None else s.get('auto'),
                    'proposed':value,'auto':auto,'manual':user,'flag':f})
            track_rows.append({'id':e['id'],'name':e['name'].replace('\u00a0',' '),'album':refine.export.album_of(e['path']),'moods':values})
        regressions=[f for f in flag_rows if f['currentHeld'] and not f['proposedHeld']]
        failures=[f for f in flag_rows if not f['currentHeld']]
        wrong_regressions=[f for f in wrong_rows if not f['held'] and next(x for x in flag_rows if x['entryId']==f['entryId'] and x['moodId']=='peaceful')['currentHeld']]
        report={'capturedAt':datetime.now(timezone.utc).isoformat(),'model':health['model'],'reference':response['mood_reference']['set'],
            'currentVocabulary':vocabulary,'plan':plan,'tracks':track_rows,'whiteOrchardId':orchard['id'],'flags':flag_rows,'moodChecks':mood_checks,
            'currentFailures':failures,'newFlagRegressions':regressions,'wrongReplacement':wrong_rows,'wrongNewRegressions':wrong_regressions,
            'summary':{'music':len(entries),'moodsBefore':len(vocabulary['moods']),'moodsAfter':len(plan),'flagCount':len(flag_rows),
                'heldBefore':sum(f['currentHeld'] for f in flag_rows),'heldAfter':sum(f['proposedHeld'] for f in flag_rows),'newRegressions':len(regressions),
                'wrongReplacementHeld':sum(f['currentHeld'] for f in flag_rows if f['moodId']!='peaceful')+sum(f['held'] for f in wrong_rows),
                'wrongReplacementNewRegressions':len(wrong_regressions),'calls':{'health':1,'score':1,'tuneCompare':len(vocabulary['moods'])}}}
        args.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        print(json.dumps(report['summary']))
        print('WHITE ORCHARD',json.dumps(next(t for t in track_rows if t['id']==orchard['id']),ensure_ascii=True))
        print('EXISTING MISSES',json.dumps(failures,ensure_ascii=True))
        print('WRONG REPLACEMENT NEW MISSES',json.dumps(wrong_regressions,ensure_ascii=True))


if __name__=='__main__':main()
