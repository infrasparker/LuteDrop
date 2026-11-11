"""Read-only audit: threat semantics versus pace, energy, aggression and role.

Scores matched slow/fast prompts, alternative literal mood descriptions and
current-line controls against all music. Energy is the app's frozen calibrated
energy value, not measured BPM; combat role is a user label, not truth.
"""
import argparse
from collections import Counter
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import itertools
import json
import math
import os
from pathlib import Path
import sqlite3
import statistics
import time

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('gap_study',ROOT/'scripts/study-uncovered-moods.py')
gap=importlib.util.module_from_spec(spec);spec.loader.exec_module(gap);r=gap.r


def partial(a,b,c):
    if any(v is None for v in (a,b,c)):return None
    den=(1-b*b)*(1-c*c)
    return None if den<1e-12 else (a-b*c)/math.sqrt(den)


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--out',type=Path,required=True);a=parser.parse_args()
    families=json.loads((ROOT/'scripts/horror-experiment-sets.json').read_text(encoding='utf-8'))
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta)
        health=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/health',None,15)
        entries=[dict(zip(['id','name','hash','path','roleExplicit'],row)) for row in conn.execute("SELECT id,name,content_hash,file_path,role_user FROM library_entries WHERE entry_type='music' ORDER BY name,id")]
        tags=dict(conn.execute("SELECT et.library_entry_id,t.name FROM library_entry_tags et JOIN tags t ON t.id=et.tag_id WHERE t.name LIKE 'ost: %'"))
        stored={(eid,mid):{'auto':auto,'user':user} for eid,mid,auto,user in conn.execute('SELECT library_entry_id,mood,auto_score,user_level FROM library_entry_moods')}
        for e in entries:
            e['album']=r.export.album_of(e.pop('path'));e['source']=tags.get(e['id'],e['album']);e['role']='combat' if e['roleExplicit']=='combat' else 'nonCombat'
            en=stored.get((e['id'],'energy'),{});e['energy']=en.get('user') if en.get('user') is not None else en.get('auto')
            if e['energy'] is None:raise ValueError('Missing stored energy; do not impute a pace')
            e['energyBand']='low' if e['energy']<.375 else 'high' if e['energy']>.625 else 'mid'
        targets=sorted(e['hash'] for e in entries);byhash={e['hash']:e for e in entries}
        horror=next(m for m in vocab['moods'] if m['id']=='grim')
        variants=[{'key':'current:'+m['id'],'name':m['name'],'kind':'current','lines':m['prompts'],'moodId':m['id']} for m in vocab['moods']]
        for f in families:
            variants += [{'key':f['name']+':'+str(i),'name':f['name'],'kind':'form','lines':[line]} for i,line in enumerate(f['sets'])]
            variants.append({'key':f['name']+':blend','name':f['name'],'kind':'blend','lines':f['sets']})
        for word in ['horror','ominous','dread','eerie','menacing','sinister','terrifying','frantic','panic','absurd','battle','slow','fast']:
            for suffix in ['', ' music']:
                variants.append({'key':'probe:'+word+suffix,'name':word,'kind':'probe','lines':[word+suffix]})
        for i,line in enumerate(horror['prompts']):
            variants.append({'key':'horror:line:'+str(i),'name':'Horror line '+str(i+1),'kind':'control','lines':[line]})
            if len(horror['prompts'])>1: variants.append({'key':'horror:remove:'+str(i),'name':'Remove Horror line '+str(i+1),'kind':'edit','lines':horror['prompts'][:i]+horror['prompts'][i+1:]})
        for name in ['Ominous','Dread','Eerie','Menacing','Terrifying','Panic','Battle horror']:
            for i,line in enumerate(next(f['sets'] for f in families if f['name']==name)):
                variants.append({'key':'append:'+name+str(i),'name':'Append '+name+' form '+str(i+1),'kind':'edit','lines':horror['prompts']+[line]})
        calls=[];reference=None;energy_raw={}
        def post(endpoint,body):
            start=time.monotonic();result=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],endpoint,body,120)
            calls.append({'endpoint':endpoint,'groups':len(body.get('mood_prompts',{})),'seconds':round(time.monotonic()-start,3)})
            return result
        for offset in range(0,len(variants),24):
            batch=variants[offset:offset+24]
            response=post('/score',{'targets':targets,'mood_prompts':{r.spread.letter_id(i):v['lines'] for i,v in enumerate(batch)},'energy_prompts':vocab['energy']})
            ref=response['mood_reference'];sig={'set':ref['set'],'tracks':ref['tracks']}
            if reference is not None and sig!=reference:raise ValueError('Reference changed')
            reference=sig
            for i,v in enumerate(batch):
                key=r.spread.letter_id(i);v['scores']={h:r.spread.calibrated(song['moods'][key],ref['stats'][key]) for h,song in response['mood'].items()}
                if set(v['scores'])!=set(targets):raise ValueError('Incomplete analyzer response')
            energy_raw={h:s['energy'] for h,s in response['mood'].items()}
            print('SCORED',min(offset+24,len(variants)),'/',len(variants),flush=True)
        current={v['moodId']:v for v in variants if v['kind']=='current'}
        base=current['grim']['scores'];ferocious=current['battle']['scores'];energy={h:byhash[h]['energy'] for h in targets}
        def rho(x,y,hs=targets):return r.spread.comparison.correlation([x[h] for h in hs],[y[h] for h in hs])
        def robust(score,forms,h):return score[h]>=.75 and sum(s[h]>=.75 for s in forms)>=2 and min(s[h] for s in forms)>=.5
        def top_ids(scores,hs=targets,n=20):return set(sorted(hs,key=lambda h:-scores[h])[:n])
        # These are diagnostics for rank reproducibility. The first three
        # geometric combinations use the same product-based fit as the app.
        combos={'Horror high + Ferocious high':{h:math.sqrt(base[h]*ferocious[h]) for h in targets},
            'Horror high + Ferocious low':{h:math.sqrt(base[h]*(1-ferocious[h])) for h in targets},
            'Horror high':base,
            'Horror high + Energy high (diagnostic)':{h:math.sqrt(base[h]*energy[h]) for h in targets},
            'Horror high + Energy low (diagnostic)':{h:math.sqrt(base[h]*(1-energy[h])) for h in targets}}
        summaries=[]
        for f in families:
            scores=next(v['scores'] for v in variants if v['name']==f['name'] and v['kind']=='blend')
            forms=[v['scores'] for v in variants if v['name']==f['name'] and v['kind']=='form']
            support=[h for h in targets if robust(scores,forms,h)]
            overlaps=sorted([{'name':v['name'],'rho':rho(scores,v['scores'])} for v in current.values()],key=lambda x:-x['rho'])
            cands=[]
            for name,values in combos.items():cands.append({'name':name,'rho':rho(scores,values),'top20Shared':len(top_ids(scores)&top_ids(values))})
            summaries.append({**f,'support':len(support),'high':sum(s>=.75 for s in scores.values()),
                'strict':sum(scores[h]>=.75 and min(v[h] for v in forms)>=.75 for h in targets),
                'agreement':statistics.mean(rho(x,y) for x,y in itertools.combinations(forms,2)),
                'sources':dict(Counter(byhash[h]['source'] for h in support)),
                'energyBands':dict(Counter(byhash[h]['energyBand'] for h in support)),
                'roles':dict(Counter(byhash[h]['role'] for h in support)),
                'energyRho':rho(scores,energy),'horrorRho':rho(scores,base),'ferociousRho':rho(scores,ferocious),
                'horrorPartialEnergy':partial(rho(scores,base),rho(scores,energy),rho(base,energy)),
                'ferociousPartialEnergy':partial(rho(scores,ferocious),rho(scores,energy),rho(ferocious,energy)),
                'closestCurrent':overlaps[:3],'existingCombinations':sorted(cands,key=lambda x:-x['rho']),
                'robustHorrorBelowHalf':sum(base[h]<.5 for h in support),'robustFerociousBelowHalf':sum(ferocious[h]<.5 for h in support),
                'tracks':[{**byhash[h],'score':scores[h],'forms':[v[h] for v in forms],'robust':h in support,'horror':base[h],'ferocious':ferocious[h]} for h in sorted(targets,key=lambda h:-scores[h])]})
        request,skipped,_=r.export.build_request(conn,meta,'grim','compare',[],[],[],focus='')
        flags=request['flags'];flagrows=[{**f,'song':byhash[f['hash']]['name'],'current':base[f['hash']],'baselineHeld':gap.holds(base[f['hash']],f)} for f in flags]
        choices=[{'name':'Replace with '+f['name'],'lines':f['sets'],'scores':next(v['scores'] for v in variants if v['name']==f['name'] and v['kind']=='blend')} for f in families]
        choices += [v for v in variants if v['kind']=='edit']
        for c in choices:
            c['newFlagBreaks']=[byhash[f['hash']]['name'] for f in flags if gap.holds(base[f['hash']],f) and not gap.holds(c['scores'][f['hash']],f)]
            c['held']=sum(gap.holds(c['scores'][f['hash']],f) for f in flags)
            c['drift']=statistics.mean(abs(c['scores'][h]-base[h]) for h in targets)
        checkchoices=[{'name':'CURRENT Horror unchanged','lines':horror['prompts']}]+choices
        tuner=[]
        for offset in range(0,len(checkchoices),8):
            batch=checkchoices[offset:offset+8];body=dict(request);body['candidates']=[c['lines'] for c in batch]
            answer=post('/tune',body);baseline=answer['baseline'];baselineflags={f['hash']:f for f in baseline['flags']}
            for c,result in zip(batch,answer['proposals']):
                tuner.append({'name':c['name'],'lines':c['lines'],'held':sum(f['holds'] for f in result['flags']),
                    'newFlagBreaks':[byhash[f['hash']]['name'] for f in result['flags'] if baselineflags[f['hash']]['holds'] and not f['holds']],
                    'loss':result['loss'],'baselineLoss':baseline['loss'],
                    'flags':[{**f,'song':byhash[f['hash']]['name']} for f in result['flags']]})
        # A bounded semantic search may diagnose whether the current category
        # can meet its flags. Never accept a proposal or add new user labels.
        semantic_pool=r.export.dedupe(horror['prompts']+[
            word.strip() for f in families if f['name'] in ['Horror literal','Ominous','Dread','Eerie','Unsettling','Sinister','Terrifying','Uncanny','Macabre','Nightmarish','Creepy','Unnerving','Haunted','Spooky','Disturbing','Disquieting','Eldritch']
            for line in f['sets'] for word in line.split(',')])
        arrange,_,_=r.export.build_request(conn,meta,'grim','arrange',semantic_pool,[],[],focus='')
        arrange['other_moods']={}
        arranged=post('/tune',arrange)
        if arranged['proposals']:
            batch=[{'key':'arranged:'+str(i),'name':'Semantic arrangement '+str(i+1),'kind':'arranged','lines':result['lines']} for i,result in enumerate(arranged['proposals'])]
            response=post('/score',{'targets':targets,'mood_prompts':{r.spread.letter_id(i):v['lines'] for i,v in enumerate(batch)},'energy_prompts':vocab['energy']})
            if {'set':response['mood_reference']['set'],'tracks':response['mood_reference']['tracks']}!=reference:raise ValueError('Arrangement reference changed')
            for i,v in enumerate(batch):
                key=r.spread.letter_id(i);stat=response['mood_reference']['stats'][key]
                v['scores']={h:r.spread.calibrated(song['moods'][key],stat) for h,song in response['mood'].items()}
                v['drift']=statistics.mean(abs(v['scores'][h]-base[h]) for h in targets)
                v['newHigh']=[byhash[h]['name'] for h in targets if base[h]<.75<=v['scores'][h]]
                v['lostHigh']=[byhash[h]['name'] for h in targets if v['scores'][h]<.75<=base[h]]
                combo={h:math.sqrt(v['scores'][h]*ferocious[h]) for h in targets}
                battle=next(vv['scores'] for vv in variants if vv['name']=='Battle horror' and vv['kind']=='blend')
                v['battleComboRho']=rho(combo,battle)
            variants.extend(batch)
        for i,result in enumerate(arranged['proposals']):
            tuner.append({'name':'Semantic arrangement '+str(i+1),'lines':result['lines'],'held':sum(f['holds'] for f in result['flags']),
                'newFlagBreaks':[byhash[f['hash']]['name'] for f in result['flags'] if gap.holds(base[f['hash']],next(x for x in flags if x['hash']==f['hash'])) and not f['holds']],
                'loss':result['loss'],'baselineLoss':arranged['baseline']['loss'],
                'flags':[{**f,'song':byhash[f['hash']]['name']} for f in result['flags']]})
        blends={f['name']:next(v['scores'] for v in variants if v['name']==f['name'] and v['kind']=='blend') for f in families}
        pairs=[]
        for x,y in itertools.combinations(blends,2):
            sx,sy=blends[x],blends[y];hx={h for h in targets if sx[h]>=.75};hy={h for h in targets if sy[h]>=.75}
            pairs.append({'a':x,'b':y,'rho':rho(sx,sy),'partialEnergy':partial(rho(sx,sy),rho(sx,energy),rho(sy,energy)),
                'sharedHigh':len(hx&hy),'onlyAHigh':len(hx-hy),'onlyBHigh':len(hy-hx),
                'energyBandRhos':{band:rho(sx,sy,[h for h in targets if byhash[h]['energyBand']==band]) for band in ['low','mid','high']},
                'sourceRemovalRange':[min(rho(sx,sy,[h for h in targets if byhash[h]['source']!=source]) for source in {e['source'] for e in entries}),
                    max(rho(sx,sy,[h for h in targets if byhash[h]['source']!=source]) for source in {e['source'] for e in entries})]})
        report={'capturedAt':datetime.now(timezone.utc).isoformat(),'model':health['model'],'reference':reference,'vocabulary':vocab,
            'entries':entries,'variants':variants,'families':summaries,'familyPairs':pairs,'calls':calls,'flags':flagrows,'tuner':tuner,'skippedFlags':skipped,'arrangement':arranged,
            'existingQueries':{name:[{**byhash[h],'score':scores[h],'horror':base[h],'ferocious':ferocious[h]} for h in sorted(targets,key=lambda h:-scores[h])[:20]] for name,scores in combos.items()},
            'currentHorror':[{**byhash[h],'score':base[h],'ferocious':ferocious[h]} for h in sorted(targets,key=lambda h:-base[h])],
            'storedDifferences':[{'song':e['name'],'mood':v['name']} for e in entries for mid,v in current.items() if (s:=stored.get((e['id'],mid))) and s['auto'] is not None and abs(s['auto']-v['scores'][e['hash']])>1e-6],
            'manualOverrides':[{'entryId':eid,'moodId':mid,'level':s['user']} for (eid,mid),s in stored.items() if s['user'] is not None],
            'summary':{'music':len(entries),'families':len(families),'scoredGroups':len(variants),'tunerChecks':len(tuner),'flags':len(flags),'baselineHeld':sum(f['baselineHeld'] for f in flagrows)},
            'limitations':['Energy is not BPM.','Noncombat is the default role for unlabeled songs.','Partial rank correlation is descriptive, not causal.','Prompt agreement does not establish semantic ground truth.','Energy diagnostic combinations are not the current UI; the UI uses energy bands.']}
        a.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        print('SUMMARY',report['summary'],flush=True);print('CURRENT HORROR',horror['prompts'],flush=True)
        for f in summaries:print(f['name'],'support',f['support'],'agree',round(f['agreement'],2),'energy',round(f['energyRho'],2),'Horror',round(f['horrorRho'],2),'Ferocious',round(f['ferociousRho'],2),'bands',f['energyBands'],'roles',f['roles'],flush=True)
        for c in tuner:
            if not c['newFlagBreaks']:print('FLAGS SAFE',c['name'],c['held'],'loss',round(c['loss'],3),'baseline',round(c['baselineLoss'],3),flush=True)


if __name__=='__main__':main()
