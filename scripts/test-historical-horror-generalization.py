"""Fresh test of the historical supervised trajectory on entirely excluded songs.

Uses only pre-migration source=user tags, never automatic tags. The ten recent
judgments are withheld from *all* training and calibration. C=1 balanced logistic
regression is fixed from the original Nov 5 implementation, not chosen on these
test songs. Album-blocked nested calibration evaluates historical transfer.
"""
import argparse
import json
from pathlib import Path
import sys

import numpy as np
from scipy.optimize import minimize
from scipy.special import expit,logit
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import roc_auc_score,brier_score_loss


def unit(X):return X/np.maximum(np.linalg.norm(X,axis=1,keepdims=True),1e-12)


def platt(scores,y):
    # Smooth a calibration fit on out-of-album logits, not in-sample predictions.
    def objective(v):
        pred=expit(v[0]*scores+v[1]);eps=1e-12
        loss=-np.mean(y*np.log(pred+eps)+(1-y)*np.log(1-pred+eps))+.001*v[0]**2
        grad=np.array([np.mean((pred-y)*scores)+.002*v[0],np.mean(pred-y)])
        return loss,grad
    res=minimize(objective,[1.,0.],jac=True,bounds=[(0,None),(None,None)],method='L-BFGS-B')
    if not res.success:raise RuntimeError(res.message)
    return res.x


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'ten-label-case/audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    old=json.loads((a.directory/'historical-user-tags.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:X=unit(z['means'].astype(float))
    with np.load(a.directory/'clap-library.npz') as z:CLAP=unit(z['means'].astype(float))
    hs=d['hashes'];byhash={e['hash']:e for e in d['entries']};horror=next(m for m in d['moods'] if m['id']=='grim');testflags=horror['flags'];excluded={f['hash'] for f in testflags}
    positive={'horror','haunting','ominous'}
    negative={'peaceful','heroic','majestic','lively','mischievous','tavern'}
    result={'source':old['source'],'provenance':old['provenance'],'excludedTestHashes':sorted(excluded),
            'primaryProtocol':'Fixed historical C=1 balanced MuLan logistic model; all ten recent songs excluded from training and calibration.','models':[]}
    for variant in ['historical-one-vs-rest','conservative-tagged-negatives']:
        history=[e for e in old['entries'] if e['hash'] not in excluded and (variant=='historical-one-vs-rest' or positive&set(e['userTags']) or negative&set(e['userTags']))]
        indices=np.array([hs.index(e['hash']) for e in history]);y=np.array([int(bool(positive&set(e['userTags']))) for e in history]);albums=np.array([e['album'] for e in history])
        for feature_name,Z in [('MuLan',X),('MuLan+CLAP',unit(np.column_stack((X,CLAP))))]:
            features=Z[indices]
            def trained(keep):
                return LogisticRegression(C=1.,max_iter=3000,class_weight='balanced',random_state=0).fit(features[keep],y[keep])
            def crossfit(keep):
                scores=np.zeros(len(keep))
                for album in sorted(set(albums[keep])):
                    train=keep[albums[keep]!=album];held=np.flatnonzero(albums[keep]==album)
                    if len(set(y[train]))<2:raise ValueError('Album holdout loses a class')
                    scores[held]=trained(train).decision_function(features[keep[held]])
                return scores
            keep=np.arange(len(history));oof=crossfit(keep)
            # Outer album predictions use a calibration fitted only on inner
            # album holdouts of the remaining albums.
            nested=np.zeros(len(history))
            for album in sorted(set(albums)):
                train=keep[albums!=album];held=keep[albums==album]
                inner=crossfit(train);cal=platt(inner,y[train]);raw=trained(train).decision_function(features[held])
                nested[held]=expit(cal[0]*raw+cal[1])
            calibrator=platt(oof,y);model=trained(keep);raw=model.decision_function(Z)
            pred=expit(calibrator[0]*raw+calibrator[1])
            rows=[]
            for f in testflags:
                h=f['hash'];v=float(pred[hs.index(h)])
                rows.append({'song':byhash[h]['name'],'score':v,'min':f['min'],'max':f['max'],
                    'withinCurrentBand':bool((f['min'] is None or v>=f['min']) and (f['max'] is None or v<=f['max']))})
            item={'negativePolicy':variant,'features':feature_name,'trainSongs':len(history),'trainPositives':int(y.sum()),'trainAlbums':len(set(albums)),
                'historicalBlockedAUC':float(roc_auc_score(y,oof)),'historicalNestedCalibratedAUC':float(roc_auc_score(y,nested)),
                'historicalNestedBrier':float(brier_score_loss(y,nested)),'testSongs':rows,'calibration':calibrator.tolist(),
                'scores':dict(zip(hs,pred.tolist())),'training':[{**e,'positive':bool(label)} for e,label in zip(history,y)]}
            result['models'].append(item)
            print('HISTORICAL',variant,feature_name,'train',len(history),'positive',int(y.sum()),'album AUC',round(item['historicalBlockedAUC'],3),'nested AUC',round(item['historicalNestedCalibratedAUC'],3),'Brier',round(item['historicalNestedBrier'],3),flush=True)
            print('EXCLUDED SONGS',[(r['song'],round(r['score'],3),r['withinCurrentBand']) for r in rows],flush=True)
            (a.directory/'historical-horror-generalization.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')


if __name__=='__main__':main()
