"""Read-only whole-library Wistful/Sorrowful distinction and literal-word probes.

Uses frozen reference calibration, current settings, tracked flags and cached
audio embeddings. Prompt agreement and score disagreement are not listening
labels. Reports all tracks, not only tracks currently below a threshold.
"""
import argparse
from collections import Counter
from contextlib import closing
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
spec=importlib.util.spec_from_file_location('gap_study',ROOT/'scripts/study-uncovered-moods.py')
gap=importlib.util.module_from_spec(spec);spec.loader.exec_module(gap)
r=gap.r


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--out',type=Path,required=True);a=parser.parse_args()
    families=json.loads((ROOT/'scripts/wistful-experiment-sets.json').read_text(encoding='utf-8'))
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta)
        health=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/health',None,15)
        entries=[dict(zip(['id','name','hash','path'],row)) for row in conn.execute("SELECT id,name,content_hash,file_path FROM library_entries WHERE entry_type='music' ORDER BY name,id")]
        tags=dict(conn.execute("SELECT et.library_entry_id,t.name FROM library_entry_tags et JOIN tags t ON t.id=et.tag_id WHERE t.name LIKE 'ost: %'"))
        for e in entries:
            e['album']=r.export.album_of(e.pop('path'));e['source']=tags.get(e['id'],e['album'])
        byhash={e['hash']:e for e in entries};targets=sorted(byhash)
        stored={(eid,mid):{'auto':auto,'user':user} for eid,mid,auto,user in conn.execute('SELECT library_entry_id,mood,auto_score,user_level FROM library_entry_moods')}
        variants=[{'key':'current:'+m['id'],'name':m['name'],'kind':'current','lines':m['prompts'],'moodId':m['id']} for m in vocab['moods']]
        for f in families:
            variants += [{'key':f['name']+':'+str(i),'name':f['name'],'kind':'form','lines':[line]} for i,line in enumerate(f['sets'])]
            variants.append({'key':f['name']+':blend','name':f['name'],'kind':'blend','lines':f['sets']})
        # Bare language probes keep adjective behavior separate from acoustic
        # and narrative qualifiers added to a longer synonym set.
        words=['wistful','sorrowful','melancholic','bittersweet','somber','nostalgic','dinky','quaint','pensive','mysterious','whimsical']
        for word in words:
            for suffix in ['', ' music']:
                variants.append({'key':'probe:'+word+suffix,'name':word,'kind':'probe','lines':[word+suffix]})
        mysterious=next((m for m in vocab['moods'] if m['id']=='mysterious'),None)
        if mysterious:
            for i,line in enumerate(mysterious['prompts']):
                variants.append({'key':'mystery:line:'+str(i),'name':'Mysterious line '+str(i+1),'kind':'mystery-control','lines':[line]})
                if len(mysterious['prompts'])>1:
                    variants.append({'key':'mystery:remove:'+str(i),'name':'Mysterious without line '+str(i+1),'kind':'mystery-control','lines':mysterious['prompts'][:i]+mysterious['prompts'][i+1:]})
        calls=[];reference=None
        def post(endpoint,body):
            start=time.monotonic();answer=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],endpoint,body,120)
            calls.append({'endpoint':endpoint,'groups':len(body.get('mood_prompts',{})),'seconds':round(time.monotonic()-start,3)})
            return answer
        for offset in range(0,len(variants),24):
            batch=variants[offset:offset+24]
            answer=post('/score',{'targets':targets,'mood_prompts':{r.spread.letter_id(i):v['lines'] for i,v in enumerate(batch)},'energy_prompts':vocab['energy']})
            ref=answer['mood_reference'];sig={'set':ref['set'],'tracks':ref['tracks']}
            if reference is not None and sig!=reference:raise ValueError('Reference changed during study')
            reference=sig
            for i,v in enumerate(batch):
                key=r.spread.letter_id(i);v['scores']={h:r.spread.calibrated(song['moods'][key],ref['stats'][key]) for h,song in answer['mood'].items()}
                if set(v['scores'])!=set(targets):raise ValueError('Missing scores')
            print('SCORED',min(offset+24,len(variants)),'/',len(variants),flush=True)
        current={v['moodId']:v for v in variants if v['kind']=='current'}
        if 'sorrowful' not in current: raise ValueError('Current Sorrowful mood no longer exists')
        sorrow=current['sorrowful']['scores']
        baseline={h:max(stored.get((byhash[h]['id'],mid),{}).get('user') if stored.get((byhash[h]['id'],mid),{}).get('user') is not None else v['scores'][h] for mid,v in current.items()) for h in targets}
        def rho(x,y,hs=targets): return r.spread.comparison.correlation([x[h] for h in hs],[y[h] for h in hs])
        def robust(blend,forms,h):return blend[h]>=.75 and sum(f[h]>=.75 for f in forms)>=2 and min(f[h] for f in forms)>=.5
        def row(h,score,forms):return {**byhash[h],'score':score[h],'sorrowful':sorrow[h],'currentBest':baseline[h],'forms':[f[h] for f in forms],'robust':robust(score,forms,h)}
        summaries=[]
        for f in families:
            score=next(v['scores'] for v in variants if v['name']==f['name'] and v['kind']=='blend')
            forms=[v['scores'] for v in variants if v['name']==f['name'] and v['kind']=='form']
            high={h for h in targets if score[h]>=.75};sad={h for h in targets if sorrow[h]>=.75};support={h for h in high if robust(score,forms,h)}
            holdouts=[]
            for source in sorted({e['source'] for e in entries}):
                hs=[h for h in targets if byhash[h]['source']!=source]
                holdouts.append({'removed':source,'remaining':len(hs),'rho':rho(score,sorrow,hs)})
            witcher=[h for h in targets if byhash[h]['source']=='ost: witcher'];others=[h for h in targets if h not in witcher]
            summaries.append({**f,'high':len(high),'support':len(support),'sources':dict(Counter(byhash[h]['source'] for h in support)),
                'strict':sum(score[h]>=.75 and min(s[h] for s in forms)>=.75 for h in targets),
                'agreement':statistics.mean(rho(x,y) for x,y in itertools.combinations(forms,2)),
                'sorrowRho':rho(score,sorrow),'sorrowHigh':len(sad),'bothHigh':len(high&sad),'onlyCandidateHigh':len(high-sad),'onlySorrowHigh':len(sad-high),
                'highJaccard':len(high&sad)/max(1,len(high|sad)),
                'robustNotSorrowHigh':len(support-sad),'robustSorrowBelowHalf':sum(sorrow[h]<.5 for h in support),
                'witcher':{'tracks':len(witcher),'support':sum(h in support for h in witcher),'rho':rho(score,sorrow,witcher)},
                'otherSources':{'tracks':len(others),'support':sum(h in support for h in others),'rho':rho(score,sorrow,others)},
                'holdouts':holdouts,'closestCurrent':sorted([{'name':v['name'],'rho':rho(score,v['scores'])} for v in current.values()],key=lambda x:-x['rho'])[:3],
                'thresholds':[{'cutoff':cut,'candidate':sum(score[h]>=cut for h in targets),'sorrowful':sum(sorrow[h]>=cut for h in targets),'both':sum(score[h]>=cut and sorrow[h]>=cut for h in targets)} for cut in [.65,.75,.85]],
                'tracks':[row(h,score,forms) for h in sorted(targets,key=lambda h:-score[h])]})
        # Does a narrower Sorrowful set actually respect the user's existing
        # constraints? Compare rather than simply assuming a conceptual split.
        request,skipped,_=r.export.build_request(conn,meta,'sorrowful','compare',[],[],[],focus='')
        replacement_names=['Sorrowful literal','Grieving','Somber','Wistful','Wistful longing']
        appended=[current['sorrowful']['lines']+[line] for line in next(f for f in families if f['name']=='Wistful')['sets']]
        request['candidates']=[next(f for f in families if f['name']==name)['sets'] for name in replacement_names]+appended
        tuned=post('/tune',request)
        checks=[]
        baseflags={f['hash']:f for f in tuned['baseline']['flags']}
        for label,result in zip(replacement_names+['Append Wistful form '+str(i+1) for i in range(3)],tuned['proposals']):
            checks.append({'label':label,'lines':result['lines'],'held':sum(f['holds'] for f in result['flags']),
                'newBreaks':[byhash[f['hash']]['name'] for f in result['flags'] if baseflags[f['hash']]['holds'] and not f['holds']],
                'flags':[{**f,'song':byhash[f['hash']]['name']} for f in result['flags']],'loss':result['loss']})
        second=dict(request)
        second_names=['Wistful literal definitions','Wistful without sadness words','CURRENT Sorrowful unchanged']
        second['candidates']=[next(f for f in families if f['name']==name)['sets'] for name in second_names[:2]]+[current['sorrowful']['lines']]
        independent=post('/tune',second)
        for label,result in zip(second_names,independent['proposals']):
            checks.append({'label':label,'lines':result['lines'],'held':sum(f['holds'] for f in result['flags']),
                'newBreaks':[byhash[f['hash']]['name'] for f in result['flags'] if baseflags[f['hash']]['holds'] and not f['holds']],
                'flags':[{**f,'song':byhash[f['hash']]['name']} for f in result['flags']],'loss':result['loss']})
        focus=[e for e in entries if any(part in e['name'] for part in ['Dear Snell','Fangle Forest','White Orchard','Roars Across'])]
        blends={v['name']:v['scores'] for v in variants if v['kind']=='blend'}
        pairs=[{'a':x,'b':y,'rho':rho(blends[x],blends[y]),
            'witcherRho':rho(blends[x],blends[y],[h for h in targets if byhash[h]['source']=='ost: witcher'])}
            for x,y in itertools.combinations(blends,2)]
        report={'capturedAt':datetime.now(timezone.utc).isoformat(),'model':health['model'],'reference':reference,'vocabulary':vocab,
            'entries':entries,'variants':variants,'families':summaries,'calls':calls,'focusIds':[e['id'] for e in focus],'familyPairs':pairs,
            'sorrowTune':{'baseline':tuned['baseline'],'checks':checks,'skipped':skipped},
            'storedDifferences':[{'song':e['name'],'mood':v['name']} for e in entries for mid,v in current.items() if (s:=stored.get((e['id'],mid))) and s['auto'] is not None and abs(s['auto']-v['scores'][e['hash']])>1e-6],
            'manualOverrides':[{'entryId':eid,'moodId':mid,'level':s['user']} for (eid,mid),s in stored.items() if s['user'] is not None],
            'summary':{'music':len(entries),'families':len(families),'scoredGroups':len(variants),'calls':len(calls)}}
        a.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        print('SUMMARY',report['summary'],flush=True)
        for f in summaries:
            print(f['name'],'support',f['support'],'rho',round(f['sorrowRho'],3),'only robust not sad',f['robustNotSorrowHigh'],'robust sad<.5',f['robustSorrowBelowHalf'],'Witcher',f['witcher'],flush=True)
        for c in checks:print('TUNE',c['label'],'held',c['held'],'breaks',c['newBreaks'],flush=True)


if __name__=='__main__':main()
