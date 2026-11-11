"""Compare an independent frozen music model against exact current flags.

Same 235 songs, same sampled audio windows, same 300 reference recordings and
same numeric bar mapping. No automatic adoption or changes to app settings.
"""
import argparse
import json
from pathlib import Path
import sys

import numpy as np
from scipy.special import expit

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tagger'))
from tagger import tune


def unit(x):return x/np.maximum(np.linalg.norm(x,axis=1,keepdims=True),1e-12)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args()
    d=json.loads((a.directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    with np.load(a.directory/'clap-library.npz') as z:
        X=unit(z['means'].astype(float));hs=z['hashes'].tolist();texts=z['texts'].tolist();T=unit(z['vectors'].astype(float))
    with np.load(a.directory/'clap-reference.npz') as z:R=unit(z['reference'].astype(float))
    if hs!=d['hashes'] or len(R)!=300:raise ValueError('Incomplete or mismatched audio')
    names={e['hash']:e['name'] for e in d['entries']}
    report={'model':'laion/larger_clap_music','songs':len(X),'reference':len(R),'moods':[]}
    for mood in d['moods']:
        w=T[[texts.index(t) for t in mood['prompts']]].mean(0);ref=R@w;raw=X@w
        m=tune.Mapping(**mood['calibration']);before=tune.bars(raw,ref,m)
        flags=[tune.FlagTarget(hs.index(f['hash']),f['min'],f['max'],f['weight'],f['fresh'],f['side'],f.get('focus',False)) for f in mood['flags']]
        broken=set(tune.broken(before,flags))
        item={'id':mood['id'],'name':mood['name'],'held':len(flags)-len(broken),'flags':len(flags),'scores':dict(zip(hs,before.tolist())),
            'flagScores':[dict(song=names[hs[f.index]],score=float(before[f.index]),min=f.lo,max=f.hi,holds=i not in broken) for i,f in enumerate(flags)],
            'priorHeld':sum((f['min'] is None or mood['scores'][f['hash']]>=f['min']) and (f['max'] is None or mood['scores'][f['hash']]<=f['max']) for f in mood['flags'])}
        report['moods'].append(item)
        print('CLAP CURRENT',mood['name'],item['held'],'/',len(flags),'MuLan',item['priorHeld'],flush=True)
        if mood['id']=='grim':
            print('CURRENT FLAGS',item['flagScores'],flush=True)
            # No new generated captions: same fixed text dictionary as MuLan.
            U=X@T.T;UR=R@T.T
            current=T[[texts.index(t) for t in mood['prompts']]].mean(0)
            U=np.vstack([U.T,X@current]);UR=np.vstack([UR.T,R@current]);units=texts+['CURRENT HORROR']
            found=tune.beam_search(U,UR,units,flags,m,seeds=[tuple(texts.index(t) for t in mood['prompts'])],before=before)
            item['tuned']=[]
            for c in found:
                ix=[units.index(t) for t in c.lines];values=tune.bars(U[ix].mean(0),UR[ix].mean(0),m);bad=set(tune.broken(values,flags))
                item['tuned'].append({'lines':list(c.lines),'held':len(flags)-len(bad),'loss':c.loss,'meanDrift':float(np.abs(values-before).mean()),'scores':dict(zip(hs,values.tolist())),
                    'flagScores':[dict(song=names[hs[f.index]],score=float(values[f.index]),holds=i not in bad) for i,f in enumerate(flags)]})
            print('CLAP TUNED',item['tuned'][0]['held'],item['tuned'][0]['lines'],item['tuned'][0]['flagScores'],flush=True)
    (a.directory/'clap-mood-study.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    case=a.directory/'clap-case';case.mkdir(exist_ok=True)
    np.savez_compressed(case/'audio-geometry.npz',means=X,reference=R)
    np.savez_compressed(case/'text-geometry.npz',texts=np.array(texts),vectors=T)
    (case/'audio-geometry-snapshot.json').write_text(json.dumps(d)+'\n',encoding='utf-8')


if __name__=='__main__':main()
