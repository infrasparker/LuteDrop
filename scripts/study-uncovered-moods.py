"""Read-only, gap-first vocabulary study using the live analyzer.

Freeze the current DB, re-score its exact vocabulary, then test independent
descriptions and semantically restricted edits across the entire music library.
Coverage uses effective scores (manual overrides retained); flags use automatic
scores, just as the app's tuner does. Results are hypotheses, not listening labels.
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

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('refine', ROOT/'scripts/refine-mood-sets.py')
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)


def holds(value, flag):
    return (flag['min'] is None or value >= flag['min']-1e-9) and (flag['max'] is None or value <= flag['max']+1e-9)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--extra', type=Path)
    p.add_argument('--edits', type=Path)
    a = p.parse_args()
    db = Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro', uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn)
        meta = r.export.meta(conn)
        vocab = r.export.vocabulary(meta)
        entries = [dict(zip(['id','name','hash','path'], row)) for row in conn.execute("SELECT id,name,content_hash,file_path FROM library_entries WHERE entry_type='music' ORDER BY name,id")]
        for e in entries: e['album'] = r.export.album_of(e.pop('path'))
        tags = dict(conn.execute("SELECT et.library_entry_id,t.name FROM library_entry_tags et JOIN tags t ON t.id=et.tag_id WHERE t.name LIKE 'ost: %'"))
        for e in entries: e['source'] = tags.get(e['id'], e['album'])
        targets = sorted({e['hash'] for e in entries if e['hash']})
        byhash = {e['hash']:e for e in entries}
        stored = {(eid,mid):{'auto':auto,'user':user} for eid,mid,auto,user in conn.execute('SELECT library_entry_id,mood,auto_score,user_level FROM library_entry_moods')}
        requests = {m['id']:r.export.build_request(conn,meta,m['id'],'compare',[],[],[],focus='')[0] for m in vocab['moods']}
        health = r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/health',None,15)
        report = {'capturedAt':datetime.now(timezone.utc).isoformat(),'model':health['model'],'vocabulary':vocab,'entries':entries,'calls':[]}
        def save(): a.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        def post(endpoint, body):
            start=time.monotonic()
            answer=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],endpoint,body,120)
            report['calls'].append({'endpoint':endpoint,'groups':len(body.get('mood_prompts',{})),'seconds':round(time.monotonic()-start,2)})
            return answer
        def score(variants):
            for offset in range(0,len(variants),24):
                batch=variants[offset:offset+24]
                answer=post('/score',{'targets':targets,'mood_prompts':{r.spread.letter_id(i):v['lines'] for i,v in enumerate(batch)},'energy_prompts':vocab['energy']})
                ref=answer['mood_reference']
                signature={'set':ref['set'],'tracks':ref['tracks']}
                if report.get('reference',signature)!=signature: raise ValueError('Reference changed')
                report['reference']=signature
                for i,v in enumerate(batch):
                    key=r.spread.letter_id(i)
                    v['scores']={h:r.spread.calibrated(song['moods'][key],ref['stats'][key]) for h,song in answer['mood'].items()}
                    if set(v['scores'])!=set(targets): raise ValueError('Missing analyzer scores')
                print(f"Scored {min(offset+24,len(variants))}/{len(variants)} sets",flush=True)
        current=[{'id':m['id'],'name':m['name'],'lines':m['prompts']} for m in vocab['moods']]
        score(current)
        current_byid={m['id']:m for m in current}
        def effective(mid,h,auto):
            override=stored.get((byhash[h]['id'],mid),{}).get('user')
            return auto if override is None else override
        baseline={h:max(effective(m['id'],h,m['scores'][h]) for m in current) for h in targets}
        weak=[h for h in targets if baseline[h]<.75]
        report['current']=current
        report['manualOverrides']=[{'entryId':eid,'moodId':mid,'level':v['user']} for (eid,mid),v in stored.items() if v['user'] is not None and mid in current_byid]
        report['weak']=[{**byhash[h],'best':baseline[h],'moods':sorted([{'id':m['id'],'name':m['name'],'score':effective(m['id'],h,m['scores'][h])} for m in current],key=lambda x:-x['score'])} for h in weak]
        report['storedDifferences']=[{'song':e['name'],'mood':m['name'],'stored':s['auto'],'rescored':m['scores'][e['hash']]} for e in entries for m in current if (s:=stored.get((e['id'],m['id']))) and s['auto'] is not None and abs(s['auto']-m['scores'][e['hash']])>1e-6]
        print('BASELINE',len(entries),'music;',len(weak),'uncovered;',len(report['storedDifferences']),'stored score differences',flush=True)
        for e in report['weak']: print(e['name'],round(e['best'],3),e['album'],flush=True)
        save()
        candidates=[q for f in ['qualifier-set-alternatives.json','battle-identity-sets.json'] for q in json.loads((ROOT/'scripts'/f).read_text(encoding='utf-8'))]
        if a.extra: candidates+=json.loads(a.extra.read_text(encoding='utf-8'))
        variants=[]
        for q in candidates:
            for i,line in enumerate(q['sets']): variants.append({'family':q['name'],'kind':'form','form':i,'lines':[line]})
            variants.append({'family':q['name'],'kind':'blend','lines':q['sets']})
        score(variants)
        report['candidateVariants']=variants
        summaries=[]
        for q in candidates:
            forms=[v['scores'] for v in variants if v['family']==q['name'] and v['kind']=='form']
            blend=next(v['scores'] for v in variants if v['family']==q['name'] and v['kind']=='blend')
            support=[h for h in targets if blend[h]>=.75 and sum(s[h]>=.75 for s in forms)>=2 and min(s[h] for s in forms)>=.5]
            overlaps=[{'name':m['name'],'rho':r.spread.comparison.correlation([blend[h] for h in targets],[m['scores'][h] for h in targets])} for m in current]
            summaries.append({**q,'support':len(support),'strictSupport':sum(min(s[h] for s in forms)>=.75 and blend[h]>=.75 for h in targets),
                'sources':dict(Counter(byhash[h]['source'] for h in support)),'albums':dict(Counter(byhash[h]['album'] for h in support)),
                'agreement':statistics.mean(r.spread.comparison.correlation([x[h] for h in targets],[y[h] for h in targets]) for x,y in itertools.combinations(forms,2)),
                'closest':max(overlaps,key=lambda x:x['rho']),'overlaps':overlaps,
                'gains':[byhash[h]['name'] for h in weak if h in support],
                'weakScores':[{'song':byhash[h]['name'],'score':blend[h],'forms':[s[h] for s in forms],'robust':h in support} for h in weak],
                'top':[{'song':byhash[h]['name'],'score':blend[h]} for h in sorted(targets,key=lambda h:-blend[h])[:12]]})
        report['candidates']=summaries
        save()
        # Existing-slot changes stay within the slot's ordinary meaning. Test
        # additions as complete blends, not max(current,new) cherry-picking.
        related={'peaceful':['Reflective','Pastoral','Tender'], 'sorrowful':['Melancholic','Mournful','Yearning'],
            'wondrous':['Solemn','Sacred','Majestic','Regal','Ceremonial'], 'joyful':['Jovial','Lively','Rowdy'],
            'quirky':['Comical','Swaggering'], 'whimsical':['Playful','Comical','Enchanted'],
            'mysterious':['Uncanny','Ethereal','Sneaky'], 'grim':['Ominous','Menacing','Tense','Haunting'],
            'heroic':['Resolute','Majestic','Dramatic'], 'battle':['Aggressive','Relentless','Frenzied']}
        alternatives=json.loads((ROOT/'scripts/mood-set-alternatives.json').read_text(encoding='utf-8'))
        byname={q['name']:q for q in candidates}
        edits=[]
        for m in current:
            choices=[('Ablation: '+label,lines) for label,lines in r.ablations(m['lines'])]
            choices += [('Alternative '+str(i+1),lines) for i,lines in enumerate(alternatives.get(m['id'],[]))]
            for name in related.get(m['id'],[]):
                for i,line in enumerate(byname[name]['sets']): choices.append((f'Append {name} formulation {i+1}',m['lines']+[line]))
            seen={json.dumps(m['lines'])}
            for label,lines in choices:
                sig=json.dumps(lines)
                if sig not in seen:
                    seen.add(sig);edits.append({'moodId':m['id'],'name':m['name'],'label':label,'lines':lines})
        if a.edits:
            for edit in json.loads(a.edits.read_text(encoding='utf-8')):
                m=current_byid[edit['moodId']]
                lines=edit['lines']
                if edit.get('mode')=='append': lines=m['lines']+lines
                elif edit.get('mode')=='replace_line':
                    lines=m['lines'][:edit['index']]+lines+m['lines'][edit['index']+1:]
                edits.append({'moodId':m['id'],'name':m['name'],'label':edit['label'],'lines':lines})
        score(edits)
        report['edits']=edits
        allflags=[]
        for m in current:
            mid=m['id'];flags=requests[mid]['flags'];base=m['scores']
            for f in flags:
                allflags.append({**f,'moodId':mid,'mood':m['name'],'song':byhash.get(f['hash'],{}).get('name',f['hash']),'baselineHeld':holds(base[f['hash']],f)})
            others={h:max(effective(x['id'],h,x['scores'][h]) for x in current if x['id']!=mid) for h in targets}
            for v in (e for e in edits if e['moodId']==mid):
                vals=v['scores']
                v['gains']=[byhash[h]['name'] for h in weak if effective(mid,h,vals[h])>=.75]
                v['lostCoverage']=[byhash[h]['name'] for h in targets if baseline[h]>=.75 and max(others[h],effective(mid,h,vals[h]))<.75]
                v['newFlagBreaks']=[byhash[f['hash']]['name'] for f in flags if holds(base[f['hash']],f) and not holds(vals[f['hash']],f)]
                v['held']=sum(holds(vals[f['hash']],f) for f in flags)
                v['flagCount']=len(flags)
                v['drift']=statistics.mean(abs(vals[h]-base[h]) for h in targets)
                v['newHigh']=[byhash[h]['name'] for h in targets if base[h]<.75<=vals[h]]
                v['lostHigh']=[byhash[h]['name'] for h in targets if vals[h]<.75<=base[h]]
                v['rho']=r.spread.comparison.correlation([vals[h] for h in targets],[base[h] for h in targets])
                v['topMovers']=[{'song':byhash[h]['name'],'before':base[h],'after':vals[h]} for h in sorted(targets,key=lambda h:-abs(vals[h]-base[h]))[:10]]
                v['weakScores']=[{'song':byhash[h]['name'],'before':base[h],'after':vals[h]} for h in weak]
        report['flags']=allflags
        # Independently verify promising screened edits with the app's real tuner.
        report['verified']=[]
        for m in current:
            options=sorted([v for v in edits if v['moodId']==m['id'] and v['gains'] and not v['newFlagBreaks'] and not v['lostCoverage']],key=lambda v:(-len(v['gains']),v['drift']))[:4]
            if not options: continue
            request=dict(requests[m['id']]);request['candidates']=[v['lines'] for v in options]
            answer=post('/tune',request)
            for v,result in zip(options,answer['proposals']):
                report['verified'].append({'moodId':m['id'],'label':v['label'],'baseline':answer['baseline'],'result':result})
        report['summary']={'music':len(entries),'weak':len(weak),'flags':len(allflags),'baselineHeld':sum(f['baselineHeld'] for f in allflags),
            'candidateFamilies':len(candidates),'candidateSets':len(variants),'editsTested':len(edits),'verifiedEdits':len(report['verified'])}
        report['completedAt']=datetime.now(timezone.utc).isoformat();save()
        print('SUMMARY',json.dumps(report['summary']),flush=True)
        for q in sorted(summaries,key=lambda q:(-len(q['gains']),q['closest']['rho'])):
            if q['gains']: print('NEW',q['name'],'gains',q['gains'],'support',q['support'],'overlap',q['closest'],flush=True)
        for v in sorted(edits,key=lambda v:(len(v['newFlagBreaks']),len(v['lostCoverage']),-len(v['gains']),v['drift'])):
            if v['gains'] and not v['newFlagBreaks'] and not v['lostCoverage']: print('EDIT',v['name'],v['label'],'gains',v['gains'],'drift',round(v['drift'],3),flush=True)


if __name__=='__main__': main()
