"""Read-only comparison of Horror search breadth and flag-first priorities.

All candidates are independently compared with the ORIGINAL unmodified flags
and current calibration. Search-only priorities never update user flags.
Deterministic phrase templates expand an explicit, bounded semantic word bank.
"""
import argparse
from contextlib import closing
from copy import deepcopy
from datetime import datetime, timezone
import importlib.util
import itertools
import json
import os
from pathlib import Path
import sqlite3
import statistics
import time

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('gap',ROOT/'scripts/study-uncovered-moods.py')
gap=importlib.util.module_from_spec(spec);spec.loader.exec_module(gap);r=gap.r

WORDS='dark eerie unnerving creepy haunted unsettling ominous dreadful uncanny unnatural eldritch sinister disturbing foreboding ghostly spooky spectral chilling disquieting morbid macabre otherworldly strange absurd fearful tense suspenseful apprehensive claustrophobic menacing'.split()


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta)
        mood=next(m for m in vocab['moods'] if m['id']=='grim')
        entries=[dict(zip(['id','name','hash','path'],row)) for row in conn.execute("SELECT id,name,content_hash,file_path FROM library_entries WHERE entry_type='music' ORDER BY name,id")]
        byhash={e['hash']:e for e in entries};targets=sorted(byhash)
        stored={eid:(auto,user) for eid,auto,user in conn.execute("SELECT library_entry_id,auto_score,user_level FROM library_entry_moods WHERE mood='grim'")}
        baseline_request,skipped,_=r.export.build_request(conn,meta,'grim','compare',[],[],[mood['prompts']],focus='')
        health=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/health',None,15)
        oldfamilies=json.loads((ROOT/'scripts/horror-experiment-sets.json').read_text(encoding='utf-8'))
        core={'Horror literal','Ominous','Dread','Eerie','Unsettling','Sinister','Terrifying','Uncanny','Macabre','Nightmarish','Creepy','Unnerving','Haunted','Spooky','Disturbing','Disquieting','Eldritch'}
        whole=[line for f in oldfamilies if f['name'] in core for line in f['sets']]
        pool=r.export.dedupe(mood['prompts']+whole+WORDS+[word+' music' for word in WORDS]+[word+' instrumental music' for word in WORDS]+[
            f'{x}, {y} instrumental music' for x,y in itertools.combinations(WORDS,2)])
        assert len(pool)<=1000
        report={'capturedAt':datetime.now(timezone.utc).isoformat(),'model':health['model'],'vocabulary':vocab,'entries':entries,
            'originalFlags':[dict(f,song=byhash[f['hash']]['name']) for f in baseline_request['flags']],
            'skippedFlags':skipped,'pool':pool,'searches':[],'comparisons':[],'calls':[]}
        def save():a.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        def post(endpoint,body,timeout=180):
            start=time.monotonic();answer=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],endpoint,body,timeout)
            report['calls'].append({'endpoint':endpoint,'seconds':round(time.monotonic()-start,3),'mode':body.get('mode'),'groups':len(body.get('mood_prompts',{}))})
            return answer
        checks=post('/tune',baseline_request)
        report['baseline']=checks['baseline'];save()
        for name,mode,extra,multiplier,kyne_boost in [
            ('App suggest defaults','suggest',[],1,1),
            ('Expanded semantic templates','arrange',pool,1,1),
            ('Expanded templates plus built-ins','suggest',pool,1,1),
            ('Expanded flag-first search','arrange',pool,20,1),
            ('Expanded flag-first; Kyne priority','arrange',pool,20,3),
        ]:
            request,_,_=r.export.build_request(conn,meta,'grim',mode,extra,[],[],focus='')
            for f in request['flags']:
                f['weight']*=multiplier
                if "Kyne's Peace" in byhash[f['hash']]['name']:f['weight']*=kyne_boost
            print('SEARCH',name,'pool',len(extra),flush=True)
            result=post('/tune',request)
            report['searches'].append({'name':name,'mode':mode,'extraUnits':len(extra),'searchWeightMultiplier':multiplier,'kyneMultiplier':kyne_boost,'answer':result})
            print('PROPOSALS',len(result['proposals']),[(sum(f['holds'] for f in c['flags']),c['lines']) for c in result['proposals']],flush=True);save()
        # Validate every distinct result, with original flag weights, no focus,
        # original current prompts and no changes to the app's reference mapping.
        proposals=[];seen=set()
        for search in report['searches']:
            for result in search['answer']['proposals']:
                key=json.dumps(result['lines'])
                if key not in seen:
                    seen.add(key);proposals.append({'search':search['name'],'lines':result['lines']})
        for offset in range(0,len(proposals),8):
            batch=proposals[offset:offset+8];request=deepcopy(baseline_request);request['candidates']=[c['lines'] for c in batch]
            result=post('/tune',request)
            before={f['hash']:f for f in result['baseline']['flags']}
            for candidate,checked in zip(batch,result['proposals']):
                report['comparisons'].append({**candidate,'result':checked,'held':sum(f['holds'] for f in checked['flags']),
                    'newBreaks':[byhash[f['hash']]['name'] for f in checked['flags'] if before[f['hash']]['holds'] and not f['holds']],
                    'flagRows':[dict(f,song=byhash[f['hash']]['name']) for f in checked['flags']]})
        if proposals:
            scored=post('/score',{'targets':targets,'mood_prompts':{r.spread.letter_id(i):p['lines'] for i,p in enumerate(proposals)},'energy_prompts':vocab['energy']})
            ref=scored['mood_reference'];report['reference']={'set':ref['set'],'tracks':ref['tracks']}
            for i,candidate in enumerate(proposals):
                key=r.spread.letter_id(i);stat=ref['stats'][key]
                candidate['scores']={h:r.spread.calibrated(song['moods'][key],stat) for h,song in scored['mood'].items()}
                candidate['meanDrift']=statistics.mean(abs(candidate['scores'][e['hash']]-stored[e['id']][0]) for e in entries)
                candidate['newHigh']=[e['name'] for e in entries if stored[e['id']][0]<.75<=candidate['scores'][e['hash']]]
                candidate['lostHigh']=[e['name'] for e in entries if candidate['scores'][e['hash']]<.75<=stored[e['id']][0]]
            report['scoredProposals']=proposals
        report['summary']={'tracks':len(entries),'pool':len(pool),'baselineHeld':sum(f['holds'] for f in report['baseline']['flags']),
            'flags':len(baseline_request['flags']),'distinctProposals':len(proposals),'bestHeld':max((c['held'] for c in report['comparisons']),default=0)}
        report['completedAt']=datetime.now(timezone.utc).isoformat();save();print('SUMMARY',report['summary'],flush=True)
        for c in sorted(report['comparisons'],key=lambda c:(len(c['newBreaks']),-c['held'],c['result']['loss'])):
            print('CHECKED',c['search'],'held',c['held'],'new breaks',c['newBreaks'],'loss',c['result']['loss'],'lines',c['lines'],flush=True)
            for f in c['flagRows']:print(' ',f['song'],round(f['after'],3),f['holds'],flush=True)


if __name__=='__main__':main()
