"""Read-only whole-library test of Forlorn descriptions and prompt formatting."""
import argparse
from contextlib import closing
from datetime import datetime,timezone
import importlib.util
import json
import os
from pathlib import Path
import sqlite3

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('gap',ROOT/'scripts/study-uncovered-moods.py');g=importlib.util.module_from_spec(spec);spec.loader.exec_module(g);r=g.r


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    families=json.loads((ROOT/'scripts/forlorn-ominous-sets.json').read_text(encoding='utf-8'))
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live,closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta)
        es=[dict(zip(['id','name','hash','path'],row)) for row in conn.execute("SELECT id,name,content_hash,file_path FROM library_entries WHERE entry_type='music' ORDER BY name")]
        for e in es:e['album']=r.export.album_of(e.pop('path'))
        tags=dict(conn.execute("SELECT et.library_entry_id,t.name FROM library_entry_tags et JOIN tags t ON t.id=et.tag_id WHERE t.name LIKE 'ost: %'"))
        for e in es:e['source']=tags.get(e['id'],e['album'])
        hs=[e['hash'] for e in es];byh={e['hash']:e for e in es};req,_,_=r.export.build_request(conn,meta,'grim','compare',[],[],[next(m['prompts'] for m in vocab['moods'] if m['id']=='grim')],focus='')
        def post(ep,b):return r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],ep,b,180)
        baseline=post('/tune',req)['baseline'];held={f['hash']:f['holds'] for f in baseline['flags']}
        variants=[{'name':'CURRENT '+m['name'],'family':'CURRENT','format':'current','lines':m['prompts']} for m in vocab['moods']]
        for f in families:
            variants += [{'name':f['name']+' / formulation '+str(i+1),'family':f['name'],'format':'single','lines':[line]} for i,line in enumerate(f['sets'])]
            variants.append({'name':f['name']+' / separate lines','family':f['name'],'format':'lines','lines':f['sets']})
            variants.append({'name':f['name']+' / single comma paragraph','family':f['name'],'format':'comma','lines':[', '.join(f['sets'])]})
        for text in ['ominous, desolate, cavernous','foreboding, cold, hollow','brooding, spectral, ancient','abyssal, desolate, monolithic, brooding, cavernous, forgotten, spectral, grim, solemn']:
            variants.append({'name':text+' / commas','family':'Exact-token punctuation control','format':'comma','lines':[text]})
            variants.append({'name':text+' / newlines','family':'Exact-token punctuation control','format':'lines','lines':text.split(', ')})
        for phrase in ['echoes trapped beneath miles of cold stone','the vertigo of gazing into an ancient void','wind whistling through hollow stalactites','footsteps swallowed by eternal night','the weight of a mountain pressing down on silence']:
            variants.append({'name':phrase,'family':'User imagery','format':'single','lines':[phrase]})
        reference=None
        for off in range(0,len(variants),24):
            batch=variants[off:off+24];ans=post('/score',{'targets':hs,'mood_prompts':{r.spread.letter_id(i):v['lines'] for i,v in enumerate(batch)},'energy_prompts':vocab['energy']})
            ref=ans['mood_reference'];sig={'set':ref['set'],'tracks':ref['tracks']}
            if reference is not None and reference!=sig:raise ValueError('Reference changed')
            reference=sig
            for i,v in enumerate(batch):
                key=r.spread.letter_id(i);stat=ref['stats'][key];v['scores']={h:r.spread.calibrated(ans['mood'][h]['moods'][key],stat) for h in hs}
                v['flags']=[dict(f,song=byh[f['hash']]['name'],score=v['scores'][f['hash']],held=g.holds(v['scores'][f['hash']],f)) for f in req['flags']]
                v['held']=sum(f['held'] for f in v['flags']);v['newBreaks']=[f['song'] for f in v['flags'] if held[f['hash']] and not f['held']]
            print('SCORED',min(off+24,len(variants)),len(variants),flush=True)
        focus={name:next(e['hash'] for e in es if name in e['name']) for name in ['Forlorn Cliffs','Kyne','Lead Your Fights','Dear Snell']}
        candidates=[v for v in variants if v['family']!='CURRENT']
        best=sorted(candidates,key=lambda v:(len(v['newBreaks']),-v['held'],v['scores'][focus['Kyne']]))[:4]
        others=sorted(candidates,key=lambda v:-(v['scores'][focus['Forlorn Cliffs']]-v['scores'][focus['Kyne']]))[:4]
        selected=[]
        for v in best+others:
            if v not in selected:selected.append(v)
        checked=post('/tune',dict(req,candidates=[v['lines'] for v in selected]))['proposals']
        out={'capturedAt':datetime.now(timezone.utc).isoformat(),'vocabulary':vocab,'reference':reference,'entries':es,'focus':focus,'flags':req['flags'],'baseline':baseline,'variants':variants,'validated':[{'name':v['name'],'result':c} for v,c in zip(selected,checked)]}
        a.out.write_text(json.dumps(out,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        for v in sorted(candidates,key=lambda v:-v['scores'][focus['Forlorn Cliffs']])[:12]:print('FORLORN',v['name'],[round(v['scores'][h],3) for h in focus.values()],v['held'],v['newBreaks'],flush=True)
        for v in sorted(candidates,key=lambda v:-(v['scores'][focus['Forlorn Cliffs']]-v['scores'][focus['Kyne']]))[:10]:print('SEPARATION',v['name'],[round(v['scores'][h],3) for h in focus.values()],v['held'],v['newBreaks'],flush=True)


if __name__=='__main__':main()
