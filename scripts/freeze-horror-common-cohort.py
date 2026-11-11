"""Second frozen experiment: use a common historical validation cohort.

R1's conservative negatives omitted untagged dark/melancholic material. Its
high historical AUC therefore compared an easier task. This version restores
the original one-vs-rest protocol and uses layer 3, selected on that common
cohort. The first blind check is now training feedback, never counted again
as independent validation. Four new songs remain completely excluded.
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

def module(name,file):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(file));m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
history=module('history','test-historical-horror-generalization.py')
personal=module('personal','study-personal-semantic-calibration.py')
unit=history.unit


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'ten-label-case/audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    old=json.loads((a.directory/'historical-user-tags.json').read_text(encoding='utf-8'))
    prior_model=json.loads((a.directory/'frozen-two-evidence-horror.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:X=unit(z['means'].astype(float))
    with np.load(a.directory/'preprojection-features.npz') as z:H=unit(z['means'][:,0].astype(float));assert z['layers'][0]==3
    hs=d['hashes'];byname={e['name']:e for e in d['entries']};mood=next(m for m in d['moods'] if m['id']=='grim')
    additions=[('Wind from the Depths',.75,None,'high'),('Burial Isle',.75,.9,'mid'),('Far Horizons',None,.5,'low'),('The Cove Battle',.75,None,'high')]
    flags=[dict(f,index=hs.index(f['hash'])) for f in mood['flags']]
    for name,lo,hi,side in additions:flags.append({'hash':byname[name]['hash'],'index':hs.index(byname[name]['hash']),'min':lo,'max':hi,'side':side,'weight':1.,'fresh':True,'focus':False})
    blind=['Blood Run','Twilight','From Past to Present','13 If Their Bones Sleep']
    excluded={f['hash'] for f in flags}|{byname[name]['hash'] for name in blind}
    positive={'horror','haunting','ominous'}
    entries=[e for e in old['entries'] if e['hash'] not in excluded]
    ix=np.array([hs.index(e['hash']) for e in entries]);labels=np.array([int(bool(positive&set(e['userTags']))) for e in entries]);groups=np.array([e['album'] for e in entries]);oof=np.zeros(len(entries))
    def estimator():return LogisticRegression(C=1.,class_weight='balanced',max_iter=2500,random_state=0)
    for album in set(groups):
        keep=groups!=album;test=~keep;clf=estimator().fit(H[ix[keep]],labels[keep]);oof[test]=clf.decision_function(H[ix[test]])
    hcal=history.platt(oof,labels);hmodel=estimator().fit(H[ix],labels);pstyle=expit(hcal[0]*hmodel.decision_function(H)+hcal[1])
    rh=prior_model['referenceHead'];punnerving=expit(rh['calibration'][0]*(X@np.array(rh['coefficient'])+rh['intercept'])+rh['calibration'][1])
    base=np.sqrt(pstyle*punnerving);prior=np.array([mood['scores'][h] for h in hs])
    comparisons=list(d.get('orderingPreferences',[]))+[{'higher':byname['Forlorn Cliffs']['hash'],'lower':byname['Burial Isle']['hash'],'trainingMargin':.03}]
    preferences=[dict(pref,higherIndex=hs.index(pref['higher']),lowerIndex=hs.index(pref['lower'])) for pref in comparisons]
    fit=personal.curve(base,prior,flags,list(range(len(flags))),preferences=preferences)
    if fit is None:
        diagnostics=[dict(song=next(e['name'] for e in d['entries'] if e['hash']==f['hash']),style=float(pstyle[f['index']]),unnerving=float(punnerving[f['index']]),combined=float(base[f['index']]),min=f['min'],max=f['max']) for f in flags]
        (a.directory/'common-cohort-fit-failure.json').write_text(json.dumps(diagnostics,indent=2)+'\n',encoding='utf-8')
        print('FAILED REPRESENTATION',diagnostics,flush=True)
        raise ValueError('The prespecified representation cannot satisfy all fourteen bands with the bounded scale')
    after,knots,values=fit;names={e['hash']:e['name'] for e in d['entries']}
    def holds(v,f):return bool((f['min'] is None or v>=f['min']-1e-7) and (f['max'] is None or v<=f['max']+1e-7))
    report={'frozenAt':datetime.now(timezone.utc).isoformat(),'protocol':__doc__,'trainingSongs':len(entries),'trainingPositives':int(labels.sum()),'blindSongs':blind,
        'excludedHashes':sorted(excluded),'libraryHead':{'layer':3,'coefficient':hmodel.coef_[0].tolist(),'intercept':float(hmodel.intercept_[0]),'calibration':hcal.tolist()},
        'referenceHead':rh,'personalScale':{'input':knots.tolist(),'output':values.tolist(),'minSlope':.1,'maxSlope':3.},
        'scores':dict(zip(hs,after.tolist())),'knownJudgments':[dict(song=names[f['hash']],score=float(after[f['index']]),min=f['min'],max=f['max'],holds=holds(after[f['index']],f)) for f in flags],
        'calibrationLeaveOneOut':[]}
    for held in range(len(flags)):
        f=personal.curve(base,prior,flags,[i for i in range(len(flags)) if i!=held],preferences=preferences)
        if f is None:continue
        v=float(f[0][flags[held]['index']]);report['calibrationLeaveOneOut'].append(dict(song=names[flags[held]['hash']],score=v,holds=holds(v,flags[held])))
    path=a.directory/'frozen-horror-common-cohort.json'
    if path.exists():raise ValueError('Do not overwrite a frozen model')
    payload=json.dumps(report,indent=2)+'\n';path.write_text(payload,encoding='utf-8',newline='\n')
    print('FROZEN SHA256',hashlib.sha256(path.read_bytes()).hexdigest())
    print('HISTORICAL',len(entries),'positive',int(labels.sum()))
    print('KNOWN',[(r['song'],round(r['score'],3),r['holds']) for r in report['knownJudgments']])
    print('SCALE LOO',sum(r['holds'] for r in report['calibrationLeaveOneOut']),len(report['calibrationLeaveOneOut']))
    print('NEXT CHECK PREDICTIONS',[(name,round(report['scores'][byname[name]['hash']],3)) for name in blind])


if __name__=='__main__':main()
