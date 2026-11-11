"""Audit portable inference, whole-library spread and cross-mood overlap."""
import json
from pathlib import Path
import sys
import numpy as np
from scipy.stats import spearmanr

base=Path(sys.argv[1])
data=json.loads((base/'audio-geometry-snapshot.json').read_text(encoding='utf-8'));hs=data['hashes'];names={e['hash']:e['name'] for e in data['entries']}
frozen=json.loads((base/'frozen-graded-horror-pilot.json').read_text(encoding='utf-8'))
runtime=json.loads((base/'frozen-pilot-runtime-verification.json').read_text(encoding='utf-8'))
runtime_scores={e['hash']:e['fit'] for e in runtime['scores']}
errors=sorted([(abs(runtime_scores[h]-frozen['scores'][h]),h) for h in hs],reverse=True)
# A previous audit found one genuinely stale final-vector cache entry. Fresh
# inference uses current audio; report the difference, never conceal it.
parity=[dict(song=names[h],absoluteDifference=float(error)) for error,h in errors[:5]]
after=np.array([frozen['scores'][h] for h in hs]);before=np.array([next(m for m in data['moods'] if m['id']=='grim')['scores'][h] for h in hs])
counts={str(threshold):int((after>=threshold).sum()) for threshold in [.5,.75,.9,.95]}
sources=len({e['album'] for e,v in zip(data['entries'],after) if v>=.75})
overlap=[{'mood':m['name'],'currentHorror':float(spearmanr(before,[m['scores'][h] for h in hs]).statistic),'pilot':float(spearmanr(after,[m['scores'][h] for h in hs]).statistic)} for m in data['moods'] if m['id']!='grim']
result={'songs':len(hs),'inferenceParity':parity,'secondLargestInferenceDifference':errors[1][0],
    'thresholdCounts':counts,'strongAlbums':sources,'saturated':int(((after<.05)|(after>.95)).sum()),'meanChange':float(np.abs(after-before).mean()),'overlap':overlap,
    'biggestChanges':[{'song':names[hs[i]],'current':float(before[i]),'pilot':float(after[i])} for i in np.argsort(-np.abs(after-before))[:20]]}
(base/'frozen-pilot-audit.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
print('PARITY',parity)
print('SPREAD',counts,'albums',sources,'near endpoints',result['saturated'],'mean change',result['meanChange'])
print('OVERLAP',overlap)
print('CHANGES',result['biggestChanges'][:8])
