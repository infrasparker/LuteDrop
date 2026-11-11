"""Read-only positive-minus-negative text experiment, with fixed reference bars.

This scoring form is NOT supported for ordinary moods in the current app.
Uses the same exact reference-moment reconstruction as exhaustive-horror-fit.
Negative text is subtracted numerically, never phrased as a natural-language 'not'.
"""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import itertools
import json
import os
from pathlib import Path
import sqlite3

import numpy as np

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('gap',ROOT/'scripts/study-uncovered-moods.py');gap=importlib.util.module_from_spec(spec);spec.loader.exec_module(gap);r=gap.r

POS=['eerie choir music','haunted horror soundtrack','haunted music suggesting the lingering presence of ghosts',
    'horror, haunted house, terror, fear, scary, dark, an ominous presence, a horror movie',
    'secrets in the dark','a lurking supernatural threat','eldritch, bleak instrumental music',
    'infernal, malevolent, ominous choir music','creepy, eerie, unnerving, disturbing music',
    'haunting voices in a desolate and abandoned place']
NEG=['peaceful, serene, tranquil, comforting music','gentle, peaceful flute music',
    'heroic, triumphant, victorious music','confident, brave, triumphant battle music',
    'calm, reassuring, pastoral instrumental music','tender, intimate, inspiring']


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db';units=POS+NEG;n=len(units)
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live,closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta);mood=next(m for m in vocab['moods'] if m['id']=='grim')
        req,_,_=r.export.build_request(conn,meta,'grim','compare',[],[],[mood['prompts']],focus='');flags=req['flags']
        es=[dict(zip(['id','name','hash'],row)) for row in conn.execute("SELECT id,name,content_hash FROM library_entries WHERE entry_type='music' ORDER BY content_hash")];hs=[e['hash'] for e in es];byh={e['hash']:e for e in es}
        def post(ep,b):return r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],ep,b,120)
        baseline=post('/tune',req)['baseline'];oldheld=np.array([f['holds'] for f in baseline['flags']]);byflag={f['hash']:f for f in baseline['flags']}
        oldheld=np.array([byflag[f['hash']]['holds'] for f in flags]);idx=[hs.index(f['hash']) for f in flags]
        ans=post('/score',{'targets':hs,'mood_prompts':{r.spread.letter_id(i):[s] for i,s in enumerate(units)},'energy_prompts':vocab['energy']})
        stats=[ans['mood_reference']['stats'][r.spread.letter_id(i)] for i in range(n)];mu=np.array([s['mean'] for s in stats]);sd=np.array([s['sd'] for s in stats]);U=np.array([[ans['mood'][h]['moods'][r.spread.letter_id(i)] for h in hs] for i in range(n)])
        C=np.diag(sd**2);pairs=list(itertools.combinations(range(n),2))
        for offset in range(0,len(pairs),24):
            batch=pairs[offset:offset+24];ans=post('/score',{'targets':[hs[0]],'mood_prompts':{r.spread.letter_id(i):[units[x],units[y]] for i,(x,y) in enumerate(batch)},'energy_prompts':vocab['energy']})
            for i,(x,y) in enumerate(batch):C[x,y]=C[y,x]=2*ans['mood_reference']['stats'][r.spread.letter_id(i)]['sd']**2-(sd[x]**2+sd[y]**2)/2
        high=np.array([np.nan if f['max'] is None else f['max'] for f in flags]);low=np.array([np.nan if f['min'] is None else f['min'] for f in flags]);kyne=next(i for i,f in enumerate(flags) if "Kyne's Peace" in byh[f['hash']]['name']);lead=next(i for i,f in enumerate(flags) if 'Lead Your' in byh[f['hash']]['name'])
        candidates=[];best={};count=0
        for nparts in [1,2,3]:
            for pp in itertools.combinations(range(len(POS)),nparts):
                for nnparts in [1,2]:
                    for nn in itertools.combinations(range(len(POS),n),nnparts):
                        for strength in [.125,.25,.5,1.,2.,4.]:
                            w=np.zeros(n);w[list(pp)]=1/len(pp);w[list(nn)]=-strength/len(nn)
                            variance=w@C@w
                            if variance<=1e-14:continue
                            z=(w@U-w@mu)/np.sqrt(variance);b=1/(1+np.exp(np.clip(-req['calibration']['slope']*(z-req['calibration']['midpoint_z']),-700,700)));fv=b[idx]
                            miss=np.nan_to_num(np.maximum(0,low-fv))+np.nan_to_num(np.maximum(0,fv-high));held=miss<1e-8;new=int((oldheld&~held).sum());total=int(held.sum());count+=1
                            item={'positive':[units[i] for i in pp],'negative':[units[i] for i in nn],'strength':strength,'held':total,'newBreaks':new,
                                'flags':[dict(f,song=byh[f['hash']]['name'],score=float(v),held=bool(ok)) for f,v,ok in zip(flags,fv,held)],'scores':dict(zip(hs,b.tolist()))}
                            key=(new,-total,float(miss.sum()))
                            if 'coverage' not in best or key<tuple(best['coverage']['key']):best['coverage']=dict(item,key=list(key))
                            if held[kyne] and held[lead] and new==0:
                                key=(-total,float(miss.sum()),float(fv[kyne]))
                                if 'negatives' not in best or key<tuple(best['negatives']['key']):best['negatives']=dict(item,key=list(key))
        out={'capturedAt':datetime.now(timezone.utc).isoformat(),'vocabulary':vocab,'entries':es,'originalFlags':flags,'baseline':baseline,'units':units,'examined':count,'best':best,
            'method':'mean(positive text cosines) minus strength*mean(negative text cosines); exact reference mean/variance; original logistic mapping',
            'limitations':['Unsupported app scoring form; no app data changed.','Exploratory selection on the same eight labels; no held-out semantic validation.']}
        a.out.write_text(json.dumps(out,ensure_ascii=False,indent=2)+'\n',encoding='utf-8');print('EXAMINED',count)
        for key,c in best.items():print(key,c['held'],c['newBreaks'],c['positive'],c['negative'],c['strength']);print([(f['song'],round(f['score'],3),f['held']) for f in c['flags']])


if __name__=='__main__':main()
