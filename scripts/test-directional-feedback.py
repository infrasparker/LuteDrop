"""Read-only directional-feedback experiment on actual cached analyzer evidence."""
import json, os, sqlite3, sys, urllib.request
from pathlib import Path
import numpy as np
from scipy.optimize import minimize
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tagger'))
from tagger import personal
BASE=Path(r'C:\Users\Infrasparker\.codex\visualizations\2026\11\08\01a1b8e6-30b2-7ea0-b750-7cc1eefbb98e')

def main():
    request=json.loads((BASE/'horror-strength-ratings-check.json').read_text(encoding='utf-8'))['request']
    hashes=request['targets']; idx={h:i for i,h in enumerate(hashes)}
    with sqlite3.connect((Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db').as_uri()+'?mode=ro',uri=True) as c:
        meta=dict(c.execute('SELECT key,value FROM meta'))
        names=dict(c.execute("SELECT content_hash,name FROM library_entries WHERE entry_type='music'"))
        stored=c.execute('SELECT e.content_hash,f.kind,f.band,f.bound,f.bar_at_flag FROM mood_flags f JOIN library_entries e ON e.id=f.library_entry_id WHERE f.mood=?',(request['mood_id'],)).fetchall()
    profile=request['active_profile']; axes=profile['wordAxes']; n=19
    req=urllib.request.Request(meta['autotag_url'].rstrip('/')+'/score',data=json.dumps(dict(targets=hashes,mood_prompts={chr(97+i):a['prompts'] for i,a in enumerate(axes)})).encode(),headers={'Content-Type':'application/json','X-LuteDrop-Token':meta['autotag_token']})
    with urllib.request.urlopen(req,timeout=120) as response: result=json.load(response)
    raw=np.array([[result['mood'][h]['moods'][chr(97+j)] for j in range(len(axes))] for h in hashes])
    z=np.load(BASE/'preprojection-features.npz',allow_pickle=False); hidx={str(h):i for i,h in enumerate(z['hashes'])}
    audio=personal.Evidence.load('tagger/tagger/reference').features(z['means'][[hidx[h] for h in hashes]])
    features=np.column_stack([audio,(raw-[a['mean'] for a in axes])/[a['sd'] for a in axes]])
    np.savez_compressed(BASE/'horror-directional-features.npz',features=features,hashes=hashes)
    X=np.column_stack([np.ones(len(features)),features]); signs=[-1 if line.startswith('-') else 1 for line in request['current_lines']]
    scale=np.pi*1.8/4; base=np.r_[-.674*scale,np.zeros(n),[scale*s/signs.count(s) for s in signs]]
    bounds=[(-8,8)]*(n+1)+[(0,8) if s>0 else (-8,0) for s in signs]
    rows=np.array([idx[h] for h,*_ in stored]); y=np.array([1 if band in ('definitely','understandable') else 0 for h,kind,band,bound,at in stored]); confidence=np.array([4 if band in ('definitely','no') else 1 for h,kind,band,bound,at in stored])
    # Historical "at most Understandable" is an upper limit, not negative membership.
    upper=np.array([.9 if band=='understandable' and bound=='at_most' else np.nan for h,kind,band,bound,at in stored])
    def fit(keep,reg):
        A=X[rows[keep]]; Y=y[keep]; W=confidence[keep]/confidence[keep].sum(); U=upper[keep]
        def fun(w):
            latent=A@w; p=personal.smooth_score(latent); der=1/(np.pi*(1+latent**2)); p=np.clip(p,1e-10,1-1e-10)
            ce=-Y*np.log(p)-(1-Y)*np.log1p(-p); g=(p-Y)/(p*(1-p))*der
            # Respect the original ceiling without turning it into a point target.
            mask=np.isfinite(U); ce[mask]=np.maximum(0,p[mask]-U[mask])**2;g[mask]=2*np.maximum(0,p[mask]-U[mask])*der[mask]
            delta=w-base; penalty=np.r_[.1,np.ones(len(w)-1)]
            return float(W@ce+reg*np.sum(penalty*delta**2)), A.T@(W*g)+2*reg*penalty*delta
        opt=minimize(fun,base,jac=True,bounds=bounds,method='L-BFGS-B',options={'maxiter':2000,'ftol':1e-12,'gtol':1e-8})
        assert opt.success,opt.message
        return opt.x,personal.smooth_score(X@opt.x)
    out=[]
    for reg in [1e-5,1e-4,.001,.01,.1]:
        weights,scores=fit(np.arange(len(rows)),reg)
        held=[]
        for i in range(len(rows)):
            _,pred=fit(np.array([j for j in range(len(rows)) if i!=j]),reg); p=pred[rows[i]]
            held.append(max(0,p-upper[i])**2 if np.isfinite(upper[i]) else -y[i]*np.log(p)-(1-y[i])*np.log1p(-p))
        r=dict(reg=reg,validation=float(np.average(held,weights=confidence)),key={names[h]:round(float(scores[i]),3) for i,h in enumerate(hashes) if any(t in names.get(h,'').lower() for t in ['bloodletting','forlorn','kyne','dawn','burial','lead your','dank bodies'])},strong=[round(float(scores[rows[i]]),3) for i in range(len(rows)) if confidence[i]==4],weights=weights.tolist())
        out.append(r);print(json.dumps({k:v for k,v in r.items() if k!='weights'}),flush=True)
    (BASE/'horror-directional-experiment.json').write_text(json.dumps(out),encoding='utf-8')
if __name__=='__main__':main()
