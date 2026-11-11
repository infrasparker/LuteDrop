"""Freeze the first external check, then make a separate ten-label training case."""
import json
from pathlib import Path
import shutil
import sys

base=Path(sys.argv[1])
snapshot=json.loads((base/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
byname={e['name']:e for e in snapshot['entries']}
live=json.loads((base/'personal-fit-live-validation.json').read_text(encoding='utf-8'))
pred={e['name']:e['personalFit'] for e in live['scores']}
labels=[{'song':'Temple of Woedica','min':.5,'max':.9,'side':'mid','qualifier':'uncertain: Stretch or Understandable; no exact threshold asserted'},
        {'song':'Dawn','min':None,'max':.5,'side':'low','qualifier':'middling No, hopeful despite mystery'}]
check=[dict(label,score=pred[label['song']],holds=bool((label['min'] is None or pred[label['song']]>=label['min']) and pred[label['song']]<=label['max'])) for label in labels]
(base/'refined-external-validation.json').write_text(json.dumps({'model':'Frozen original eight-label personal fit','results':check,'note':'Temple label revised by user; initial check remains in first-external-validation.json.'},indent=2)+'\n',encoding='utf-8')
for label in check:print('EXTERNAL CHECK',label)
mood=next(m for m in snapshot['moods'] if m['id']=='grim')
for label in labels:
    mood['flags'].append({'hash':byname[label['song']]['hash'],'min':label['min'],'max':label['max'],'weight':1.,'fresh':True,'side':label['side'],'focus':False})
snapshot['externalLabelsNowTraining']=labels
snapshot['orderingPreferences']=[{'higher':byname['Forlorn Cliffs']['hash'],'lower':byname['Temple of Woedica']['hash'],
    'source':'Forlorn Definitely versus Temple uncertain Stretch/Understandable','trainingMargin':.03}]
directory=base/'ten-label-case';directory.mkdir(exist_ok=True)
(directory/'audio-geometry-snapshot.json').write_text(json.dumps(snapshot,indent=2)+'\n',encoding='utf-8')
for file in ['audio-geometry.npz','text-geometry.npz']:
    shutil.copyfile(base/file,directory/file)
