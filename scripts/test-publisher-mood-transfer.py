"""Independent emotion supervision from the reference recordings' publisher tags.

Primary task defined before inspection: Eerie OR Unnerving. Dark alone and
Suspenseful alone are not assumed to mean Horror. Current library songs and
their flags never participate in fitting or calibration.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re

import numpy as np
from scipy.special import expit
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import roc_auc_score,brier_score_loss
from sklearn.model_selection import StratifiedGroupKFold

spec=importlib.util.spec_from_file_location('history',Path(__file__).with_name('test-historical-horror-generalization.py'))
history=importlib.util.module_from_spec(spec);spec.loader.exec_module(history)
unit=history.unit


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'ten-label-case/audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    ref=json.loads((Path(__file__).resolve().parents[1]/'tagger/tagger/reference/manifest.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:X,R=unit(z['means'].astype(float)),unit(z['reference'].astype(float))
    hs=d['hashes'];byhash={e['hash']:e for e in d['entries']};flags=next(m for m in d['moods'] if m['id']=='grim')['flags']
    # Keep related title families (e.g. SCP variants) together. The recording's
    # title is not a predictor and none of the game-library audio is in training.
    groups=np.array([re.sub('[^a-z]','',e['title'].split()[0].lower()) for e in ref])
    report={'primary':'Eerie OR Unnerving','source':'Publisher feel tags in the shipped 300-recording manifest','libraryUsedInTraining':False,'models':[]}
    for name,positive in [('eerie-or-unnerving',{'Eerie','Unnerving'}),('eerie',{'Eerie'}),('unnerving',{'Unnerving'}),('suspenseful',{'Suspenseful'})]:
        y=np.array([int(bool(positive&set(e['feel']))) for e in ref])
        splitter=StratifiedGroupKFold(5,shuffle=True,random_state=104729)
        oof=np.zeros(len(R));raw_oof=np.zeros(len(R))
        for train,test in splitter.split(R,y,groups):
            inner_oof=np.zeros(len(train))
            inner=StratifiedGroupKFold(4,shuffle=True,random_state=104729)
            for it,iv in inner.split(R[train],y[train],groups[train]):
                estimator=LogisticRegression(C=1.,max_iter=2500,class_weight='balanced',random_state=0).fit(R[train[it]],y[train[it]])
                inner_oof[iv]=estimator.decision_function(R[train[iv]])
            cal=history.platt(inner_oof,y[train])
            estimator=LogisticRegression(C=1.,max_iter=2500,class_weight='balanced',random_state=0).fit(R[train],y[train])
            raw=estimator.decision_function(R[test]);oof[test]=expit(cal[0]*raw+cal[1]);raw_oof[test]=raw
        cal=history.platt(raw_oof,y)
        model=LogisticRegression(C=1.,max_iter=2500,class_weight='balanced',random_state=0).fit(R,y)
        pred=expit(cal[0]*model.decision_function(X)+cal[1])
        item={'name':name,'positives':int(y.sum()),'referenceNestedAUC':float(roc_auc_score(y,oof)),'referenceNestedBrier':float(brier_score_loss(y,oof)),
            'calibration':cal.tolist(),'scores':dict(zip(hs,pred.tolist())),'test':[]}
        for f in flags:
            value=float(pred[hs.index(f['hash'])])
            item['test'].append(dict(song=byhash[f['hash']]['name'],score=value,min=f['min'],max=f['max'],
                bandHolds=bool((f['min'] is None or value>=f['min']) and (f['max'] is None or value<=f['max']))))
        report['models'].append(item)
        print('PUBLISHER',name,'positives',int(y.sum()),'nested reference AUC',round(item['referenceNestedAUC'],3),'Brier',round(item['referenceNestedBrier'],3),flush=True)
        print('INDEPENDENT GAME SONGS',[(r['song'],round(r['score'],3),r['bandHolds']) for r in item['test']],flush=True)
    (a.directory/'publisher-mood-transfer-study.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')


if __name__=='__main__':main()
