"""Fixed four-layer audio ensemble for independently annotated emotion evidence.

Every head learns only from the 300 publisher-tagged reference recordings.
No library tracks or personal flags enter these fits. Equal averaging over
layers 3,6,9,12 avoids selecting a layer using the user's evaluation songs.
"""
import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np
from scipy.special import expit
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import roc_auc_score
from sklearn.model_selection import StratifiedGroupKFold

spec=importlib.util.spec_from_file_location('history',Path(__file__).with_name('test-historical-horror-generalization.py'))
history=importlib.util.module_from_spec(spec);spec.loader.exec_module(history)
unit=history.unit


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    snapshot=json.loads((a.directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    manifest=json.loads((Path(__file__).resolve().parents[1]/'tagger/tagger/reference/manifest.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'preprojection-features.npz') as z:means=z['means'].astype(float);layers=z['layers'].tolist()
    with np.load(a.directory/'preprojection-reference-features.npz') as z:refmeans=z['means'].astype(float)
    groups=np.array([e['title'].split()[0].lower() for e in manifest])
    report={'protocol':__doc__,'sourceRecordings':300,'source':'Publisher feel tags; not personal Horror grades.','heads':[]}
    for label in ['Eerie','Unnerving','Suspenseful','Dark','Somber','Calming','Relaxed','Uplifting']:
        target=np.array([int(label in e['feel']) for e in manifest]);members=[];predictions=[];oofmembers=[]
        for j,layer in enumerate(layers):
            X,R=unit(means[:,j]),unit(refmeans[:,j]);oof=np.zeros(len(R))
            for train,test in StratifiedGroupKFold(5,shuffle=True,random_state=104729).split(R,target,groups):
                clf=LogisticRegression(C=1.,class_weight='balanced',max_iter=2500,random_state=0).fit(R[train],target[train])
                oof[test]=clf.decision_function(R[test])
            cal=history.platt(oof,target)
            clf=LogisticRegression(C=1.,class_weight='balanced',max_iter=2500,random_state=0).fit(R,target)
            pred=expit(cal[0]*clf.decision_function(X)+cal[1]);predictions.append(pred);oofmembers.append(expit(cal[0]*oof+cal[1]))
            members.append({'layer':layer,'coefficient':clf.coef_[0].tolist(),'intercept':float(clf.intercept_[0]),'calibration':cal.tolist()})
        pooled=np.mean(predictions,axis=0);auc=float(roc_auc_score(target,np.mean(oofmembers,axis=0)))
        report['heads'].append({'label':label,'sourcePositives':int(target.sum()),'sourceCrossfitRankingAUC':auc,'models':members,'scores':dict(zip(snapshot['hashes'],pooled.tolist()))})
        print('AUDIO EVIDENCE',label,'positives',int(target.sum()),'source crossfit rank AUC',round(auc,3),flush=True)
    (a.directory/'publisher-audio-evidence.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')


if __name__=='__main__':main()
