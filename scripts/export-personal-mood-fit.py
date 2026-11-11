"""Export the tested semantic scorer as a portable, explicitly experimental profile."""
import json
from pathlib import Path
import sys
import numpy as np

directory=Path(sys.argv[1])
study=json.loads((directory/'personal-semantic-calibration-study.json').read_text(encoding='utf-8'))
snapshot=json.loads((directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
model=study['full']
with np.load(directory/'audio-geometry.npz') as z:R=z['reference'].astype(float)
R/=np.linalg.norm(R,axis=1,keepdims=True)
with np.load(directory/'text-geometry.npz') as z:texts=z['texts'].tolist();T=z['vectors'].astype(float)
T/=np.linalg.norm(T,axis=1,keepdims=True)
vector=T[texts.index(model['positive'])]-model['strength']*T[texts.index(model['negative'])]
reference=R@vector
mood=next(m for m in snapshot['moods'] if m['id']=='grim')
profile={'schema':'lutedrop.personal-mood-fit/v1','name':'Horror — experimental personal fit','moodId':'grim',
    'model':snapshot['model'],'positive':[model['positive']],'negative':[model['negative']],'negativeWeight':model['strength'],
    'reference':{'set':snapshot['reference']['set'],'tracks':snapshot['reference']['tracks'],'mean':float(reference.mean()),'sd':float(reference.std())},
    'referenceMapping':mood['calibration'],'fitMapping':{'input':model['knots'],'output':model['values'],'minSlope':study['minSlope'],'maxSlope':study['maxSlope']},
    'meaning':'Personal mood fit learned from explicit flags; not a reference percentile or probability.',
    'evidence':{'trainingHeld':sum(f['holds'] for f in model['flags']),'flags':len(model['flags']),
        'outerLeaveOneOutHeld':sum(f['holds'] for f in study['outerLeaveOneOut']),'outerLeaveOneOutTotal':len(study['outerLeaveOneOut']),
        'meanLibraryDrift':model['meanDrift'],'songs':len(snapshot['hashes']),'status':'experimental'},
    'trainingFlags':mood['flags']}
profile['orderingPreferences']=study.get('orderingPreferences',[])
profile['uncertainFeedback']=snapshot.get('externalLabelsNowTraining',[])
(directory/'horror-personal-fit-profile.json').write_text(json.dumps(profile,indent=2)+'\n',encoding='utf-8')
print('Exported profile:',profile['positive'],'minus',profile['negativeWeight'],profile['negative'],profile['evidence'])
