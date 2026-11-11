"""Freeze a compact two-evidence scorer before receiving a new blind check.

Horror style: layer 9 hidden audio features, historical user tags, fixed C=1.
Unnerving: publisher labels on entirely separate reference recordings, C=1.
Combine equally by geometric mean; learn only a monotone personal score scale
from the ten existing judgments. Four next-check songs are absent from all
library training. Their answers must not be used to revise this frozen scorer.
"""
import argparse
from datetime import datetime,timezone
import hashlib
import importlib.util
import json
from pathlib import Path

import numpy as np
from scipy.special import expit
from sklearn.linear_model import LogisticRegression
from sklearn.model_selection import StratifiedGroupKFold

def module(name,file):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(file));m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
history=module('history','test-historical-horror-generalization.py')
personal=module('personal','study-personal-semantic-calibration.py')
unit=history.unit


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'ten-label-case/audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    old=json.loads((a.directory/'historical-user-tags.json').read_text(encoding='utf-8'))
    ref=json.loads((Path(__file__).resolve().parents[1]/'tagger/tagger/reference/manifest.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:X,R=unit(z['means'].astype(float)),unit(z['reference'].astype(float))
    with np.load(a.directory/'preprojection-features.npz') as z:H=unit(z['means'][:,2].astype(float));assert z['layers'][2]==9
    hs=d['hashes'];byname={e['name']:e for e in d['entries']};mood=next(m for m in d['moods'] if m['id']=='grim');flags=[dict(f,index=hs.index(f['hash'])) for f in mood['flags']]
    blind=['Wind from the Depths','Burial Isle','Far Horizons','The Cove Battle']
    excluded={f['hash'] for f in flags}|{byname[name]['hash'] for name in blind}
    positive={'horror','haunting','ominous'};negative={'peaceful','heroic','majestic','lively','mischievous','tavern'}
    entries=[e for e in old['entries'] if e['hash'] not in excluded and (positive&set(e['userTags']) or negative&set(e['userTags']))]
    ix=np.array([hs.index(e['hash']) for e in entries]);labels=np.array([int(bool(positive&set(e['userTags']))) for e in entries]);groups=np.array([e['album'] for e in entries])
    oof=np.zeros(len(entries))
    def estimator():return LogisticRegression(C=1.,class_weight='balanced',max_iter=2500,random_state=0)
    for album in set(groups):
        keep=groups!=album;test=~keep
        clf=estimator().fit(H[ix[keep]],labels[keep]);oof[test]=clf.decision_function(H[ix[test]])
    hcal=history.platt(oof,labels);hmodel=estimator().fit(H[ix],labels)
    pstyle=expit(hcal[0]*hmodel.decision_function(H)+hcal[1])
    y=np.array([int('Unnerving' in e['feel']) for e in ref]);groups=np.array([e['title'].split()[0].lower() for e in ref]);oof=np.zeros(len(ref))
    for train,test in StratifiedGroupKFold(5,shuffle=True,random_state=104729).split(R,y,groups):
        clf=estimator().fit(R[train],y[train]);oof[test]=clf.decision_function(R[test])
    ucal=history.platt(oof,y);umodel=estimator().fit(R,y)
    punnerving=expit(ucal[0]*umodel.decision_function(X)+ucal[1])
    base=np.sqrt(pstyle*punnerving)
    prior=np.array([mood['scores'][h] for h in hs])
    preferences=[dict(pref,higherIndex=hs.index(pref['higher']),lowerIndex=hs.index(pref['lower'])) for pref in d.get('orderingPreferences',[])]
    fit=personal.curve(base,prior,flags,list(range(len(flags))),preferences=preferences)
    if fit is None:raise ValueError('Fixed two-evidence scorer cannot satisfy the personal bands with the bounded scale')
    after,knots,values=fit
    names={e['hash']:e['name'] for e in d['entries']}
    def holds(value,f):return bool((f['min'] is None or value>=f['min']-1e-7) and (f['max'] is None or value<=f['max']+1e-7))
    report={'frozenAt':datetime.now(timezone.utc).isoformat(),'architecture':__doc__,'trainingSongs':len(entries),'trainingPositives':int(labels.sum()),
        'blindSongs':blind,'allExcludedHashes':sorted(excluded),'libraryHead':{'layer':9,'coefficient':hmodel.coef_[0].tolist(),'intercept':float(hmodel.intercept_[0]),'calibration':hcal.tolist()},
        'referenceHead':{'label':'Unnerving','coefficient':umodel.coef_[0].tolist(),'intercept':float(umodel.intercept_[0]),'calibration':ucal.tolist()},
        'personalScale':{'input':knots.tolist(),'output':values.tolist(),'minimumSlope':.1,'maximumSlope':3.},
        'scores':dict(zip(hs,after.tolist())),'evidenceScores':[dict(song=names[h],style=float(s),unnerving=float(u),combined=float(b),personal=float(v)) for h,s,u,b,v in zip(hs,pstyle,punnerving,base,after)],
        'knownJudgments':[dict(song=names[f['hash']],score=float(after[f['index']]),min=f['min'],max=f['max'],holds=holds(after[f['index']],f)) for f in flags],
        'calibrationOnlyLeaveOneOut':[],'limitations':['Historical tags are weak proxy labels, not graded Horror judgments.','Ten personal judgments fit the scale; they are not an external test.','The blind predictions were frozen before new answers.']}
    for held in range(len(flags)):
        train=[i for i in range(len(flags)) if i!=held]
        f=personal.curve(base,prior,flags,train,preferences=preferences)
        if f is None:continue
        v=float(f[0][flags[held]['index']])
        report['calibrationOnlyLeaveOneOut'].append(dict(song=names[flags[held]['hash']],score=v,holds=holds(v,flags[held])))
    payload=json.dumps(report,indent=2)+'\n'
    profile=a.directory/'frozen-two-evidence-horror.json'
    if profile.exists():raise ValueError('Frozen scorer already exists; do not overwrite it after asking for labels')
    profile.write_text(payload,encoding='utf-8')
    digest=hashlib.sha256(payload.encode()).hexdigest()
    print('FROZEN SHA256',digest)
    print('TRAINED HISTORICAL',len(entries),'positive',int(labels.sum()))
    print('KNOWN',[(r['song'],round(r['score'],3),r['holds']) for r in report['knownJudgments']])
    print('SCALE LOO',[(r['song'],round(r['score'],3),r['holds']) for r in report['calibrationOnlyLeaveOneOut']])
    print('BLIND PREDICTIONS',[(e['song'],round(e['personal'],3)) for e in report['evidenceScores'] if e['song'] in blind])


if __name__=='__main__':main()
