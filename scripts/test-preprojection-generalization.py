"""Choose hidden audio features using historical tags only; test excluded judgments.

No current judgment, title, album or artist is used as an input feature. Album
identifiers are used solely to keep validation folds apart. The historical
feature-selection AUC is not reported as independent test performance.
"""
import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np
from scipy.special import expit
from sklearn.linear_model import LogisticRegression
from sklearn.metrics import roc_auc_score

spec=importlib.util.spec_from_file_location('history',Path(__file__).with_name('test-historical-horror-generalization.py'))
history=importlib.util.module_from_spec(spec);spec.loader.exec_module(history)
unit=history.unit


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'ten-label-case/audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    old=json.loads((a.directory/'historical-user-tags.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'audio-geometry.npz') as z:current=unit(z['means'].astype(float))
    with np.load(a.directory/'preprojection-features.npz') as z:
        means,stds= z['means'].astype(float),z['stds'].astype(float)
        fresh=unit(z['finals'].astype(float));hs=z['hashes'].tolist();layers=z['layers'].tolist()
    if hs!=d['hashes']:raise ValueError('Incomplete hidden feature export')
    candidates={'production-512':current,'fresh-512':fresh}
    for j,layer in enumerate(layers):
        candidates[f'layer-{layer}-mean-1024']=unit(means[:,j])
    candidates['last-mean-and-variation-2048']=unit(np.column_stack((unit(means[:,-1]),unit(stds[:,-1]))))
    candidates['all-layer-means-4096']=unit(np.concatenate([unit(means[:,j]) for j in range(len(layers))],axis=1))
    candidates['all-layer-means-and-variation-8192']=unit(np.concatenate([unit(feature[:,j]) for feature in [means,stds] for j in range(len(layers))],axis=1))
    parity=np.clip(np.sum(current*fresh,axis=1),-1,1)
    byhash={e['hash']:e for e in d['entries']};mood=next(m for m in d['moods'] if m['id']=='grim');flags=mood['flags'];excluded={f['hash'] for f in flags}
    report={'protocol':__doc__,'cacheAudit':{'minCosine':float(parity.min()),'medianCosine':float(np.median(parity)),
        'largestChanges':[dict(song=d['entries'][i]['name'],cosine=float(parity[i])) for i in np.argsort(parity)[:12]]},'models':[]}
    positive={'horror','haunting','ominous'};negative={'peaceful','heroic','majestic','lively','mischievous','tavern'}
    for policy in ['all-other-user-tagged-music','conservative-tagged-negatives']:
        entries=[e for e in old['entries'] if e['hash'] not in excluded and (policy=='all-other-user-tagged-music' or positive&set(e['userTags']) or negative&set(e['userTags']))]
        ix=np.array([hs.index(e['hash']) for e in entries]);labels=np.array([int(bool(positive&set(e['userTags']))) for e in entries]);groups=np.array([e['album'] for e in entries])
        for name,X in candidates.items():
            oof=np.zeros(len(entries))
            for album in set(groups):
                train=groups!=album;test=~train
                model=LogisticRegression(C=1.,class_weight='balanced',max_iter=2000,random_state=0).fit(X[ix[train]],labels[train])
                oof[test]=model.decision_function(X[ix[test]])
            cal=history.platt(oof,labels)
            model=LogisticRegression(C=1.,class_weight='balanced',max_iter=2000,random_state=0).fit(X[ix],labels)
            prediction=expit(cal[0]*model.decision_function(X)+cal[1])
            item={'features':name,'negativePolicy':policy,'historicalSelectionAUC':float(roc_auc_score(labels,oof)),
                'trainingSongs':len(entries),'trainingPositives':int(labels.sum()),'excludedHashes':sorted(excluded),
                'scores':dict(zip(hs,prediction.tolist())),'test':[]}
            for flag in flags:
                h=flag['hash'];v=float(prediction[hs.index(h)])
                item['test'].append(dict(song=byhash[h]['name'],score=v,min=flag['min'],max=flag['max'],
                    bandHolds=bool((flag['min'] is None or v>=flag['min']) and (flag['max'] is None or v<=flag['max']))))
            report['models'].append(item)
            print('HIDDEN',policy,name,'selection AUC',round(item['historicalSelectionAUC'],3),'test bands',sum(f['bandHolds'] for f in item['test']),'/10',flush=True)
            print([(f['song'],round(f['score'],3)) for f in item['test']],flush=True)
            (a.directory/'preprojection-generalization-study.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    report['selectedByHistory']=[max([m for m in report['models'] if m['negativePolicy']==p],key=lambda m:(m['historicalSelectionAUC'],-len(m['features'])))['features'] for p in ['all-other-user-tagged-music','conservative-tagged-negatives']]
    (a.directory/'preprojection-generalization-study.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print('CACHE PARITY',report['cacheAudit'],flush=True)
    print('HISTORY SELECTED',report['selectedByHistory'],flush=True)


if __name__=='__main__':main()
