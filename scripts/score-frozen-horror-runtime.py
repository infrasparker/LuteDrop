"""Apply a frozen audio scorer to cached features, without training or an LLM.

The same runtime accepts newly extracted tracks; it does not use song identity
as a scoring input. Scores from the two components are retained as a diagnostic
of disagreement, not a calibrated uncertainty estimate.
"""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np
from scipy.special import expit


def unit(X):return X/np.maximum(np.linalg.norm(X,axis=1,keepdims=True),1e-12)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    model=json.loads((a.directory/'horror-pilot-runtime.json').read_text(encoding='utf-8'))
    weightfile=a.directory/'horror-pilot-weights.npz'
    if hashlib.sha256(weightfile.read_bytes()).hexdigest()!=model['weightsSha256']:raise ValueError('Weight file changed')
    with np.load(weightfile,allow_pickle=False) as z:weights={k:z[k].copy() for k in z.files}
    with np.load(a.directory/'preprojection-features.npz',allow_pickle=False) as z:
        H=z['means'].astype(float);layers=z['layers'].tolist();F=unit(z['finals'].astype(float));hashes=z['hashes'].tolist()
    def predict(spec,X):return expit(spec['calibration'][0]*(X@weights[spec['coefficient']]+spec['intercept'])+spec['calibration'][1])
    scores=[]
    for head in model['heads']:
        values=[predict(member,unit(H[:,layers.index(member['layer'])])) for member in head['members']]
        scores.append(np.mean(values,axis=0))
    grade=weights[model['gradedCoefficient']]
    graded=np.clip(np.column_stack([np.ones(len(H))]+scores)@grade,0,1)
    style=predict(model['style'],unit(H[:,layers.index(model['style']['layer'])]));unnerving=predict(model['unnerving'],F)
    raw=np.sqrt(style*unnerving)
    coarse=np.interp(raw,model['coarseMapping']['input'],model['coarseMapping']['output'])
    final=model['gradedWeight']*graded+(1-model['gradedWeight'])*coarse
    rows=[{'hash':h,'fit':float(v),'graded':float(g),'coarse':float(c),'componentDisagreement':float(abs(g-c))} for h,v,g,c in zip(hashes,final,graded,coarse)]
    a.out.write_text(json.dumps({'model':model['sourceFrozenSha256'],'meaning':model['meaning'],'scores':rows},indent=2)+'\n',encoding='utf-8')
    print('Frozen inference complete:',len(rows),'tracks; no training, no app writes.')


if __name__=='__main__':main()
