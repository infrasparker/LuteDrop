"""Reconstruct the production fit and isolate regularization, without app writes."""
import inspect
import json
import os
from pathlib import Path
import sqlite3
import sys
import urllib.request
import numpy as np

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tagger'))
from tagger import personal, tune

BASE=Path(r'C:\Users\Infrasparker\.codex\visualizations\2026\11\08\01a1b8e6-30b2-7ea0-b750-7cc1eefbb98e')


def main():
    original=json.loads((BASE/'horror-strength-ratings-check.json').read_text(encoding='utf-8'))
    variants=json.loads((BASE/'horror-strength-word-diagnosis.json').read_text(encoding='utf-8'))
    request=original['request']; hashes=request['targets']; index={h:i for i,h in enumerate(hashes)}
    with sqlite3.connect((Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db').as_uri()+'?mode=ro',uri=True) as conn:
        meta=dict(conn.execute('SELECT key,value FROM meta'))
        names=dict(conn.execute("SELECT content_hash,name FROM library_entries WHERE entry_type='music'"))
    profiles=[request['active_profile'],*(variants[k]['result']['proposals'][0]['profile'] for k in ['current','safety_contrast'])]
    phrases=list(dict.fromkeys(a['prompts'][0] for p in profiles for a in p['wordAxes']))
    body=dict(targets=hashes,mood_prompts={chr(97+i):[text] for i,text in enumerate(phrases)})
    req=urllib.request.Request(meta['autotag_url'].rstrip('/')+'/score',data=json.dumps(body).encode(),headers={'Content-Type':'application/json','X-LuteDrop-Token':meta['autotag_token']})
    with urllib.request.urlopen(req,timeout=120) as response: result=json.load(response)
    raw={text:np.array([result['mood'][h]['moods'][chr(97+i)] for h in hashes]) for i,text in enumerate(phrases)}
    z=np.load(BASE/'preprojection-features.npz',allow_pickle=False)
    hidx={str(h):i for i,h in enumerate(z['hashes'])}
    evidence=personal.Evidence.load(Path('tagger/tagger/reference'))
    audio=evidence.features(z['means'][[hidx[h] for h in hashes]])
    flags=[tune.FlagTarget(index[f['hash']],None,None,1.,True,'mid',target=f['target']) for f in request['flags']]
    wanted=['bloodletting','forlorn','kyne','dawn','burial','lead your']
    summary=[]
    for label,profile in zip(['saved','current','safety_contrast'],profiles):
        axes=profile['wordAxes']; Z=np.column_stack([(raw[a['prompts'][0]]-a['mean'])/a['sd'] for a in axes])
        scores=personal.profile_scores(personal.Profile(**profile),evidence,audio,np.column_stack([raw[a['prompts'][0]] for a in axes]))
        if label != 'saved':
            actual=variants[label]['result']['proposals'][0]['flags']
            error=max(abs(scores[index[f['hash']]]-f['after']) for f in actual)
            assert error < 1e-5, f'Cached audio differs: {error}'
        for i,h in enumerate(hashes):
            if 'bloodletting' in names.get(h,'').lower():
                contrib={'bias':profile['bias'],**{head['label']:float(audio[i,j]*profile['audioWeights'][j]) for j,head in enumerate(evidence.metadata['heads'])},**{a['prompts'][0]:float(Z[i,j]*a['weight']) for j,a in enumerate(axes)}}
                print(label,'Bloodletting',round(scores[i],3),'largest contributions',sorted(contrib.items(),key=lambda kv:-abs(kv[1]))[:7],flush=True)
        if label=='saved': continue
        features=np.column_stack([audio,Z]); signs=[1 if a['weight']>=0 else -1 for a in axes]
        # Use the exact production objective; replace only its regularization grid.
        for strength in [1e-5,1e-4,.001,.01,.1]:
            src=inspect.getsource(personal.fit_general).replace('grid = [.00001, .0001, .001, .01, .1]',f'grid = [{strength}]')
            scope=dict(personal.__dict__);exec(src,scope); diagnostics={}
            weights,values,_=scope['fit_general'](features,flags,signs,audio.shape[1],diagnostics=diagnostics)
            row=dict(description=label,regularization=strength,mae=float(np.mean([abs(values[f.index]-f.target) for f in flags])),validation=diagnostics['validation_mae'],productionCoefficientDifference=float(np.max(abs(weights-np.r_[profile['bias'],profile['audioWeights'],[a['weight'] for a in axes]]))),key={names[h]:round(float(values[i]),3) for i,h in enumerate(hashes) if any(t in names.get(h,'').lower() for t in wanted)})
            summary.append(row);print(json.dumps(row),flush=True)
    (BASE/'horror-strength-regularization-check.json').write_text(json.dumps(summary),encoding='utf-8')


if __name__=='__main__':main()
