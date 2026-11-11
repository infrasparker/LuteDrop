"""Freeze an externally testable graded pilot; never overwrite after feedback.

The personal graded model uses eight independently learned emotion inputs.
A single mixture weight retains the earlier coarse scorer where compatible
with all fourteen training judgments. No next-check song participates in either
component's supervised training, the personal grades, or mixture selection.
"""
import argparse
from datetime import datetime,timezone
import hashlib
import json
from pathlib import Path

import numpy as np


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    data=json.loads((a.directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    graded=json.loads((a.directory/'graded-audio-evidence-study.json').read_text(encoding='utf-8'))
    coarse=json.loads((a.directory/'frozen-two-evidence-horror.json').read_text(encoding='utf-8'))
    history=json.loads((a.directory/'historical-user-tags.json').read_text(encoding='utf-8'))
    evidence=json.loads((a.directory/'publisher-audio-evidence.json').read_text(encoding='utf-8'))
    hs=data['hashes'];byname={e['name']:e for e in data['entries']}
    blind=['09 Alexius','21 The Inquisition Marches','23 Arkemyr',next(n for n in byname if 'Periki' in n)]
    # Extra assurance: these new-album test tracks were not in the historical
    # dataset at all, not merely absent from its positive set.
    old_ids={e['id'] for e in history['entries']}
    if any(byname[name]['id'] in old_ids for name in blind):raise ValueError('A proposed blind song has historical labels')
    g=np.array([graded['scores'][h] for h in hs]);c=np.array([coarse['scores'][h] for h in hs]);delta=g-c
    lower,upper=0.,1.
    def interval(intercept,slope,lo,hi):
        nonlocal lower,upper
        if abs(slope)<1e-12:
            if (lo is not None and intercept<lo-1e-7) or (hi is not None and intercept>hi+1e-7):raise ValueError('Mixture cannot meet a training constraint')
            return
        if lo is not None:
            edge=(lo-intercept)/slope
            if slope>0:lower=max(lower,edge)
            else:upper=min(upper,edge)
        if hi is not None:
            edge=(hi-intercept)/slope
            if slope>0:upper=min(upper,edge)
            else:lower=max(lower,edge)
    for f in graded['flags']:
        i=f['index'];interval(c[i],delta[i],f['min'],f['max'])
    fi=hs.index(byname['Forlorn Cliffs']['hash'])
    for name in ['Temple of Woedica','Burial Isle']:
        i=hs.index(byname[name]['hash']);interval(c[fi]-c[i],delta[fi]-delta[i],1e-6,None)
    if lower>upper+1e-7:raise ValueError('Mixture has no feasible weight')
    alpha=float(np.clip(.5,max(0,lower),min(1,upper)))
    scores=c+alpha*delta
    report={'frozenAt':datetime.now(timezone.utc).isoformat(),'protocol':__doc__,'gradedWeight':alpha,'coarseWeight':1-alpha,
        'personalJudgmentsUsed':14,'blindSongs':blind,'scores':dict(zip(hs,scores.tolist())),
        'gradedComponent':{'features':graded['features'],'coefficients':graded['coefficients'],'sourceModels':evidence},
        'coarseComponent':coarse,'knownJudgments':[],'testProvenance':'All four next-check songs absent from historical tags, publisher recordings and personal training judgments.',
        'limitations':['This candidate was developed after the first blind check; that check is now training feedback.','A four-song external check is not population-wide reliability evidence.']}
    for f in graded['flags']:
        v=float(scores[f['index']]);name=next(e['name'] for e in data['entries'] if e['hash']==f['hash'])
        report['knownJudgments'].append({'song':name,'score':v,'min':f['min'],'max':f['max'],'holds':bool((f['min'] is None or v>=f['min']-1e-7) and (f['max'] is None or v<=f['max']+1e-7))})
    path=a.directory/'frozen-graded-horror-pilot.json'
    if path.exists():raise ValueError('Do not overwrite a frozen pilot')
    path.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8',newline='\n')
    print('FROZEN SHA256',hashlib.sha256(path.read_bytes()).hexdigest())
    print('WEIGHT graded',alpha,'coarse',1-alpha)
    print('KNOWN',[(r['song'],round(r['score'],3),r['holds']) for r in report['knownJudgments']])
    print('BLIND PREDICTIONS',[(name,round(report['scores'][byname[name]['hash']],3)) for name in blind])


if __name__=='__main__':main()
