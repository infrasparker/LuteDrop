"""Read-only deterministic narrative/choral phrase probes for Horror flags."""
import argparse
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import sqlite3

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('gap',ROOT/'scripts/study-uncovered-moods.py');gap=importlib.util.module_from_spec(spec);spec.loader.exec_module(gap);r=gap.r


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    adjectives='inhuman unholy demonic evil malevolent ghostly spectral forbidden dreadful sinister unsettling eerie haunted desolate'.split()
    templates=['a {w} presence','music suggesting a {w} presence','a lurking {w} threat','music suggesting a lurking {w} threat',
        '{w}, unsettling, ominous horror music','{w} choir music','{w} choral horror soundtrack','{w} ambient horror soundtrack']
    texts=[t.format(w=w) for w in adjectives for t in templates]
    texts+=['music suggesting a lurking supernatural threat','an eerie atmosphere of haunted isolation and supernatural dread',
        'a ghostly and desolate choral horror soundtrack','haunted, ghostly, desolate choir music',
        'haunting voices in a desolate and abandoned place','a haunting choir evoking supernatural dread']
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live,closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta);mood=next(m for m in vocab['moods'] if m['id']=='grim')
        req,_,_=r.export.build_request(conn,meta,'grim','compare',[],[],[mood['prompts']],focus='');flags=req['flags']
        es=[dict(zip(['id','name','hash'],row)) for row in conn.execute("SELECT id,name,content_hash FROM library_entries WHERE entry_type='music' ORDER BY name")];hs=[e['hash'] for e in es];byh={e['hash']:e for e in es}
        def post(ep,b):return r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],ep,b,120)
        baseline=post('/tune',req)['baseline'];baseheld={f['hash']:f['holds'] for f in baseline['flags']}
        variants=[]
        for offset in range(0,len(texts),24):
            batch=texts[offset:offset+24];ans=post('/score',{'targets':hs,'mood_prompts':{r.spread.letter_id(i):[line] for i,line in enumerate(batch)},'energy_prompts':vocab['energy']})
            for i,line in enumerate(batch):
                key=r.spread.letter_id(i);stat=ans['mood_reference']['stats'][key];scores={h:r.spread.calibrated(ans['mood'][h]['moods'][key],stat) for h in hs}
                rows=[dict(f,song=byh[f['hash']]['name'],score=scores[f['hash']],held=gap.holds(scores[f['hash']],f)) for f in flags]
                variants.append({'lines':[line],'scores':scores,'flags':rows,'held':sum(f['held'] for f in rows),'newBreaks':[f['song'] for f in rows if baseheld[f['hash']] and not f['held']]})
        kyne=next(f['hash'] for f in flags if "Kyne's Peace" in byh[f['hash']]['name']);lead=next(f['hash'] for f in flags if 'Lead Your' in byh[f['hash']]['name'])
        ranked=sorted(variants,key=lambda v:(len(v['newBreaks']),v['scores'][kyne]>.5,-v['held'],v['scores'][kyne]))
        selected=ranked[:8];validated=post('/tune',dict(req,candidates=[v['lines'] for v in selected]))['proposals']
        out={'vocabulary':vocab,'entries':es,'originalFlags':flags,'baseline':baseline,'variants':variants,'selected':[dict(v,result=res) for v,res in zip(selected,validated)]}
        a.out.write_text(json.dumps(out,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        for v in ranked[:15]:print(v['lines'],v['held'],v['newBreaks'],'Kyne',round(v['scores'][kyne],3),'Lead',round(v['scores'][lead],3),flush=True)
        print('TOP FORLORN')
        fh=next(e['hash'] for e in es if e['name']=='Forlorn Cliffs')
        for v in sorted(variants,key=lambda v:-v['scores'][fh])[:8]:print(v['lines'],round(v['scores'][fh],3),'Kyne',round(v['scores'][kyne],3),'Lead',round(v['scores'][lead],3),flush=True)


if __name__=='__main__':main()
