"""Bounded, read-only tests of semantic-reference anchoring over every mood."""
import json,sys,sqlite3,os
from pathlib import Path
import numpy as np
from scipy.optimize import minimize
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tagger'))
from tagger import personal,tune
BASE=Path(r'C:\Users\Infrasparker\.codex\visualizations\2026\11\08\01a1b8e6-30b2-7ea0-b750-7cc1eefbb98e')

def main():
    text={}
    for filename in ['text-geometry.npz','sparse-fit-text-vectors.npz']:
        z=np.load(BASE/filename,allow_pickle=False);text.update(zip(z['texts'].tolist(),z['vectors']))
    lib=np.load(BASE/'preprojection-features.npz',allow_pickle=False)
    ref=np.load(BASE/'preprojection-reference-features.npz',allow_pickle=False)
    active=np.load(BASE/'active-reference.npz',allow_pickle=False)
    V=np.asarray(active['vectors'],float);V/=np.linalg.norm(V,axis=1,keepdims=True)
    L=np.asarray(lib['finals'],float);L/=np.linalg.norm(L,axis=1,keepdims=True)
    evidence=personal.Evidence.load('tagger/tagger/reference');audio=evidence.features(lib['means']);refaudio=evidence.features(ref['means'])
    hashes=lib['hashes'].tolist();idx={h:i for i,h in enumerate(hashes)}
    report=json.loads((BASE/'directional-mood-upgrade-final.json').read_text(encoding='utf-8'))
    with sqlite3.connect((Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db').as_uri()+'?mode=ro',uri=True) as c: names=dict(c.execute("SELECT content_hash,name FROM library_entries WHERE entry_type='music'"))
    results=[]; datasets={}
    for record in report['reviews']:
        request=record['request'];old=next(m['profile'] for m in report['vocabulary']['moods'] if m['id']==record['mood'])
        axes=old['wordAxes'];P=np.array([text[a['prompts'][0]]/np.linalg.norm(text[a['prompts'][0]]) for a in axes]).T
        mean=np.array([a['mean'] for a in axes]);sd=np.array([a['sd'] for a in axes])
        assert np.max(abs((V@P).mean(axis=0)-mean))<1e-6
        features=np.column_stack([audio,(L@P-mean)/sd]);anchors=np.column_stack([refaudio,(V@P-mean)/sd])
        X=np.column_stack([np.ones(len(features)),features]);R=np.column_stack([np.ones(len(anchors)),anchors])
        signs=[-1 if s.startswith('-') else 1 for s in request['current_lines']];scale=np.pi*1.8/4;base=np.r_[-.674*scale,np.zeros(19),[scale*s/signs.count(s) for s in signs]]
        bounds=[(-8,8)]*20+[(0,8) if s>0 else (-8,0) for s in signs]
        flags=[tune.FlagTarget(idx[f['hash']],f.get('min'),f.get('max'),1.,True,'mid',target=f.get('target'),polarity=f.get('polarity'),confidence=f.get('confidence',1),pivot=f.get('pivot',.5),feedback_version=5) for f in request['flags']]
        A=X[[f.index for f in flags]];weight=np.array([f.confidence/4 for f in flags],float)
        datasets[record['mood']]=dict(features=features,anchors=anchors,flags=flags,signs=signs,base=base,request=request)
        for strength in [.03,.1,.3]:
            def fun(w):
                loss,grad=personal.directional_losses(A@w,flags);delta=w-base;diff=R@delta
                return float(weight@loss+strength*np.mean(diff**2)+.0001*np.sum(delta**2)),A.T@(weight*grad)+2*strength*R.T@diff/len(R)+.0002*delta
            opt=minimize(fun,base,jac=True,bounds=bounds,method='L-BFGS-B',options={'maxiter':2000,'ftol':1e-12,'gtol':1e-8});assert opt.success,opt.message
            score=personal.smooth_score(X@opt.x);prior=personal.smooth_score(X@base);selected=[idx[h] for h in request['targets']]
            row=dict(mood=request['name'],anchor=strength,bias=round(float(opt.x[0]),3),above9=int(sum(score[selected]>=.9)),below1=int(sum(score[selected]<=.1)),priorAbove9=int(sum(prior[selected]>=.9)),meanChange=round(float(np.mean(abs(score[selected]-prior[selected]))),4),examples=[dict(hash=request['flags'][i]['hash'],score=round(float(score[f.index]),4),direction=f.polarity,confidence=f.confidence) for i,f in enumerate(flags)])
            row['top']=[(names.get(hashes[i],hashes[i]),round(float(score[i]),4)) for i in sorted(selected,key=lambda i:-score[i])[:10]]
            results.append(row)
            print(json.dumps({k:v for k,v in row.items() if k not in ['examples','top']}),flush=True)
    (BASE/'reference-anchoring-sum-study.json').write_text(json.dumps(results),encoding='utf-8')
    # Pure research cache: feature columns and ordering are explicit, no secrets.
    np.savez_compressed(BASE/'reference-anchoring-features.npz',hashes=np.array(hashes),refaudio=refaudio,**{f'{mid}_features':data['features'] for mid,data in datasets.items()},**{f'{mid}_anchors':data['anchors'] for mid,data in datasets.items()})

if __name__=='__main__':main()
