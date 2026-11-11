"""Derive joint coverage and overlap from the live gap-first study (no rescoring)."""
import argparse
import importlib.util
import itertools
import json
from pathlib import Path
import statistics

spec=importlib.util.spec_from_file_location('study',Path(__file__).with_name('study-uncovered-moods.py'))
study=importlib.util.module_from_spec(spec);spec.loader.exec_module(study)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('report',type=Path);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    d=json.loads(a.report.read_text(encoding='utf-8'));hashes=[e['hash'] for e in d['entries']];byhash={e['hash']:e for e in d['entries']}
    base={m['id']:m['scores'] for m in d['current']}
    overrides={(v['entryId'],v['moodId']):v['level'] for v in d.get('manualOverrides',[])}
    def effective(scores,mid,h): return overrides.get((byhash[h]['id'],mid),scores[mid][h])
    selected=[('mysterious','Replace instrument-heavy line with enigmatic atmosphere'),('whimsical','Append impish synonym'),
        ('wondrous','Add devotional to existing reverent line'),('heroic','Ablation: Remove subword: victorious'),('grim','Append Ominous formulation 1')]
    edits=[next(e for e in d['edits'] if (e['moodId'],e['label'])==key) for key in selected]
    plans=[]
    for name,versions in [('Current',[]),('First two refinements',edits[:2]),('Five-edit review bundle',edits)]:
        scores={**base,**{e['moodId']:e['scores'] for e in versions}}
        best={h:max(effective(scores,mid,h) for mid in scores) for h in hashes}
        before={h:max(effective(base,mid,h) for mid in base) for h in hashes}
        lost=[byhash[h]['name'] for h in hashes if before[h]>=.75 and best[h]<.75]
        breaks=[f['song']+' / '+f['mood'] for f in d['flags'] if f['baselineHeld'] and not study.holds(scores[f['moodId']][f['hash']],f)]
        pairs=[study.r.spread.comparison.correlation([x[h] for h in hashes],[y[h] for h in hashes]) for x,y in itertools.combinations(scores.values(),2)]
        plans.append({'name':name,'edits':[e['label'] for e in versions],'covered':sum(v>=.75 for v in best.values()),'lostCoverage':lost,'newFlagBreaks':breaks,
            'flagsHeld':sum(study.holds(scores[f['moodId']][f['hash']],f) for f in d['flags']),
            'meanPositiveRho':statistics.mean(max(0,v) for v in pairs),'maxPositiveRho':max(pairs),
            'gapTracks':[{'song':w['name'],'before':w['best'],'after':best[w['hash']],
                'bestMood':next(m['name'] for m in d['current'] if effective(scores,m['id'],w['hash'])==best[w['hash']])} for w in d['weak']]})
    # Whole-source removal diagnoses album/series concentration; it is not a
    # held-out human-label test and must not be described as one.
    for q in d['candidates']:
        variants=[v for v in d['candidateVariants'] if v['family']==q['name']]
        blend=next(v['scores'] for v in variants if v['kind']=='blend')
        current=next(m['scores'] for m in d['current'] if m['name']==q['closest']['name'])
        corrs=[]
        for source in set(e['source'] for e in d['entries']):
            hs=[e['hash'] for e in d['entries'] if e['source']!=source]
            corrs.append(study.r.spread.comparison.correlation([blend[h] for h in hs],[current[h] for h in hs]))
        q['sourceRemovalRhoRange']=[min(corrs),max(corrs)]
        q['supportAfterLargestSourceRemoved']=q['support']-max(q['sources'].values(),default=0)
    result={'capturedAt':d['capturedAt'],'model':d['model'],'reference':d['reference'],'summary':d['summary'],
        'current':[{'id':m['id'],'name':m['name'],'lines':m['lines']} for m in d['current']],
        'weak':d['weak'],'reviewEdits':[{k:v for k,v in e.items() if k!='scores'} for e in edits],
        'candidates':d['candidates'],'plans':plans,
        'rules':{'cutoff':.75,'candidateSupport':'combined >= .75; at least two of three formulations >= .75; third >= .50',
            'limitations':['Prompt agreement is not semantic ground truth.','84 families explored; apparent winners need listening review.',
                'No Heroic flags exist; passing zero constraints provides no evidence of correct Heroic classification.',
                'Existing unmet Serene flag remains; no app settings changed.']}}
    a.out.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    for plan in plans: print(json.dumps(plan,ensure_ascii=False))


if __name__=='__main__':main()
