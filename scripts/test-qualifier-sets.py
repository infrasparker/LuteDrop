"""Score alternative qualifier word SETS against a frozen current-set study.

python scripts/test-qualifier-sets.py --baseline <current-set-tests.json> --out <qualifiers.json>

The live analyzer scores three complete formulations per qualifier, plus the
exact three-line combination the app would save. All per-form bars, a median
diagnostic and pairwise rank agreements are retained. Read-only app access.
"""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import itertools
import json
import os
from pathlib import Path
import sqlite3
import statistics
import time


ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('refinement',ROOT/'scripts/refine-mood-sets.py')
refinement=importlib.util.module_from_spec(spec)
spec.loader.exec_module(refinement)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline',type=Path,required=True)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--sets',type=Path,default=ROOT/'scripts/qualifier-set-alternatives.json')
    parser.add_argument('--db',type=Path,default=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db')
    args=parser.parse_args()
    baseline=json.loads(args.baseline.read_text(encoding='utf-8'))
    qualifiers=json.loads(args.sets.read_text(encoding='utf-8'))
    with closing(sqlite3.connect(args.db.resolve().as_uri()+'?mode=ro',uri=True)) as conn:
        meta=refinement.export.meta(conn)
    entries=baseline['entries'];targets=sorted({e['hash'] for e in entries if e['hash']})
    current={m['name']:next(v['scores'] for v in m['variants'] if v['label']=='CURRENT') for m in baseline['moods']}
    current_max={h:max(s[h] for s in current.values()) for h in targets}
    variants=[v for q in qualifiers for v in [
        *({'qualifier':q['name'],'prompts':[text],'kind':'form'} for text in q['sets']),
        {'qualifier':q['name'],'prompts':q['sets'],'kind':'blend'}]]
    calls=[]
    for offset in range(0,len(variants),24):
        batch=variants[offset:offset+24]
        body={'targets':targets,'mood_prompts':{refinement.spread.letter_id(i):v['prompts'] for i,v in enumerate(batch)},'energy_prompts':baseline['vocabulary']['energy']}
        started=time.monotonic()
        response=refinement.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/score',body,120)
        ref=response['mood_reference']
        if ref['set']!=baseline['reference']['set']:
            raise RuntimeError('Reference set differs from the baseline')
        calls.append({'endpoint':'/score','groups':len(batch),'seconds':round(time.monotonic()-started,3)})
        for i,v in enumerate(batch):
            key=refinement.spread.letter_id(i);stat=ref['stats'][key]
            v['scores']={h:refinement.spread.calibrated(r['moods'][key],stat) for h,r in response['mood'].items()}
        print(f'/score: {min(offset+24,len(variants))}/{len(variants)} complete qualifier sets',flush=True)
    for q in qualifiers:
        forms=[v for v in variants if v['qualifier']==q['name'] and v['kind']=='form']
        q['forms']=forms
        q['scores']=next(v['scores'] for v in variants if v['qualifier']==q['name'] and v['kind']=='blend')
        q['consensus']={h:statistics.median(v['scores'][h] for v in forms) for h in targets}
        q['agreement']=statistics.mean(refinement.spread.comparison.correlation([a['scores'][h] for h in targets],[b['scores'][h] for h in targets]) for a,b in itertools.combinations(forms,2))
        q['currentOverlap']=sorted([{'name':name,'correlation':refinement.spread.comparison.correlation([q['scores'][h] for h in targets],[s[h] for h in targets])} for name,s in current.items()],key=lambda x:-x['correlation'])[:3]
        q['newIdentity']=[{'id':e['id'],'name':e['name'],'currentBest':current_max[e['hash']],'score':q['scores'][e['hash']], 'range':[min(v['scores'][e['hash']] for v in forms),max(v['scores'][e['hash']] for v in forms)]} for e in entries if current_max[e['hash']]<.75 and q['scores'][e['hash']]>=.75]
        q['top']=sorted([{'id':e['id'],'name':e['name'],'album':e['album'],'score':q['scores'][e['hash']]} for e in entries],key=lambda e:-e['score'])[:15]
    report={'capturedAt':datetime.now(timezone.utc).isoformat(),'baselineCapturedAt':baseline['capturedAt'],'model':baseline['model'],'reference':baseline['reference'],'entries':entries,'vocabulary':baseline['vocabulary'],'current':current,'currentMax':current_max,'qualifiers':qualifiers,'calls':calls,'variantCount':len(variants)}
    args.out.write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    for q in sorted(qualifiers,key=lambda q:(-len(q['newIdentity']),-q['agreement'])):
        print(f"{q['name']:14} agreement {q['agreement']:.2f} | weak tracks reached {len(q['newIdentity'])} | overlap {q['currentOverlap'][0]['name']} {q['currentOverlap'][0]['correlation']:.2f}")
        print('  '+str([(e['name'],round(e['currentBest'],2),round(e['score'],2)) for e in q['newIdentity']]))
        print('  TOP: '+ ' / '.join(e['name'] for e in q['top'][:5]))


if __name__=='__main__':
    main()
