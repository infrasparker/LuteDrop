"""Export frozen weights to a compact, reusable scorer; no fitting or app writes."""
import hashlib
import json
from pathlib import Path
import sys
import numpy as np

base=Path(sys.argv[1]);path=base/'frozen-graded-horror-pilot.json';raw=path.read_bytes()
digest=hashlib.sha256(raw).hexdigest()
assert digest=='28016d98aaa57e94a0ec1ac270cbd301dcd8f6427795fb28ca43ee33433a1d36'
frozen=json.loads(raw);arrays={};heads=[]
for i,head in enumerate(frozen['gradedComponent']['sourceModels']['heads']):
    members=[]
    for j,model in enumerate(head['models']):
        key=f'emotion_{i}_{j}';arrays[key]=np.array(model['coefficient'],dtype=np.float64)
        members.append({'layer':model['layer'],'coefficient':key,'intercept':model['intercept'],'calibration':model['calibration']})
    heads.append({'label':head['label'],'members':members})
coarse=frozen['coarseComponent'];style=coarse['libraryHead'];unnerving=coarse['referenceHead']
arrays['style']=np.array(style['coefficient']);arrays['unnerving']=np.array(unnerving['coefficient'])
arrays['grade']=np.array(frozen['gradedComponent']['coefficients'])
np.savez_compressed(base/'horror-pilot-weights.npz',**arrays)
runtime={'schema':'lutedrop.experimental-audio-fit/v1','sourceFrozenSha256':digest,'model':'OpenMuQ/MuQ-MuLan-large',
    'sampleRate':24000,'windowSeconds':10,'maxWindows':8,'pooling':'Mean frames and windows, then normalize each hidden-layer mean. Final 512: normalize windows, mean, normalize.',
    'meaning':'Personal Horror fit; neither a probability nor a reference percentile.','status':'experimental',
    'heads':heads,'gradedCoefficient':'grade','gradedWeight':frozen['gradedWeight'],
    'style':dict(style,coefficient='style'),'unnerving':dict(unnerving,coefficient='unnerving'),'coarseMapping':coarse['personalScale'],
    'evidence':{'personalTrainingJudgments':14,'historicalSourceTrainingSongs':coarse['trainingSongs'],'referenceSourceRecordings':300,
        'firstFrozenCheck':'3/4 graded bands; Burial overstated. First-check feedback is now training data.',
        'secondFrozenCheck':'Not used in training; awaiting human judgments.'},
    'weightsSha256':hashlib.sha256((base/'horror-pilot-weights.npz').read_bytes()).hexdigest()}
(base/'horror-pilot-runtime.json').write_text(json.dumps(runtime,indent=2)+'\n',encoding='utf-8')
print('Exported compact runtime; frozen model unchanged.',(base/'horror-pilot-weights.npz').stat().st_size,'weight bytes')
