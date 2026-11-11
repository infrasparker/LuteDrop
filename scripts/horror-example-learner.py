"""Read-only deterministic audio-feedback prototype, with nested leave-one-out.

Fit a regularized correction to the existing text score using single-seed audio
cosines from /similar. Labels are interval centers from existing flags. This is
an exploratory personalized score, not the app's reference-calibrated percentile.
No prompts, flags, calibration or app models are changed.
"""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import sqlite3

import numpy as np

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('gap',ROOT/'scripts/study-uncovered-moods.py');gap=importlib.util.module_from_spec(spec);spec.loader.exec_module(gap);r=gap.r


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta);mood=next(m for m in vocab['moods'] if m['id']=='grim')
        request,_,_=r.export.build_request(conn,meta,'grim','compare',[],[],[mood['prompts']],focus='');flags=request['flags']
        entries=[dict(zip(['id','name','hash'],row)) for row in conn.execute("SELECT id,name,content_hash FROM library_entries WHERE entry_type='music' ORDER BY content_hash")];hashes=[e['hash'] for e in entries]
        idx=[hashes.index(f['hash']) for f in flags];n=len(flags)
        def post(endpoint,body):return r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],endpoint,body,120)
        scored=post('/score',{'targets':hashes,'mood_prompts':{'current':mood['prompts']},'energy_prompts':vocab['energy']})
        stat=scored['mood_reference']['stats']['current'];before=np.array([r.spread.calibrated(scored['mood'][h]['moods']['current'],stat) for h in hashes]);prior=before[idx]
        S=np.empty((len(hashes),n))
        for j,f in enumerate(flags):
            result=post('/similar',{'query':[f['hash']],'candidates':hashes,'limit':500});values={x['hash']:x['score'] for x in result['results']}
            # A unit vector's cosine with itself is exactly one. /similar omits seeds.
            values[f['hash']]=1.0
            if set(values)!=set(hashes):raise ValueError('Missing audio similarity values')
            S[:,j]=[values[h] for h in hashes]
        K=(S[idx]+S[idx].T)/2
        y=np.array([((0 if f['min'] is None else f['min'])+(1 if f['max'] is None else f['max']))/2 for f in flags]);delta=y-prior
        grid=[.001,.01,.1,1.,10.]
        def predict(train,test,lam):
            alpha=np.linalg.solve(K[np.ix_(train,train)]+lam*np.eye(len(train)),delta[train])
            return np.clip(prior[test]+K[np.ix_(test,train)]@alpha,0,1)
        def miss(scores,positions):return sum(max(0,(flags[i]['min'] or 0)-float(s))+max(0,float(s)-(1 if flags[i]['max'] is None else flags[i]['max'])) for i,s in zip(positions,scores))
        def choose(train):
            errors=[]
            for lam in grid:
                error=0
                for held in train:
                    rest=[i for i in train if i!=held];error+=miss(predict(rest,[held],lam),[held])
                errors.append((error,-lam,lam))
            return min(errors)[2]
        loo=[]
        for held in range(n):
            train=[i for i in range(n) if i!=held];lam=choose(train);value=float(predict(train,[held],lam)[0])
            loo.append({'song':entries[idx[held]]['name'],'predicted':value,'held':gap.holds(value,flags[held]),'lambda':lam})
        lam=choose(list(range(n)));alpha=np.linalg.solve(K+lam*np.eye(n),delta);after=np.clip(before+S@alpha,0,1)
        trainrows=[{'song':entries[i]['name'],'before':float(before[i]),'after':float(after[i]),'held':gap.holds(float(after[i]),f),'targetMin':f['min'],'targetMax':f['max'],'fittedTarget':float(t)} for f,i,t in zip(flags,idx,y)]
        out={'capturedAt':datetime.now(timezone.utc).isoformat(),'vocabulary':vocab,'originalFlags':flags,'entries':entries,'lambda':lam,'lambdaGrid':grid,
            'training':trainrows,'nestedLeaveOneOut':loo,'summary':{'flags':n,'baselineHeld':sum(gap.holds(float(before[i]),f) for f,i in zip(flags,idx)),'trainingHeld':sum(t['held'] for t in trainrows),'nestedLooHeld':sum(t['held'] for t in loo),'meanDrift':float(np.abs(after-before).mean())},
            'top':[dict(entries[i],before=float(before[i]),after=float(after[i])) for i in np.argsort(-after)[:20]],
            'limitations':['Only eight labeled examples.','Nested leave-one-out tests the correction with a fixed existing text prior.','Training fit is not generalization evidence.','Personalized correction scores are not reference percentiles.','No app settings changed.']}
        a.out.write_text(json.dumps(out,ensure_ascii=False,indent=2)+'\n',encoding='utf-8');print(out['summary']);print('TRAIN',trainrows);print('NESTED LOO',loo)


if __name__=='__main__':main()
