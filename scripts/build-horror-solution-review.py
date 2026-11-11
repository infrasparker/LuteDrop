"""Pack real scorer comparisons into the existing literal review fragment."""
import json
from pathlib import Path
import re
import sys
import numpy as np
from scipy.stats import spearmanr

base=Path(sys.argv[1])
def read(name):return json.loads((base/name).read_text(encoding='utf-8'))
snapshot=read('audio-geometry-snapshot.json');hs=snapshot['hashes'];current=next(m for m in snapshot['moods'] if m['id']=='grim')['scores'];before=np.array([current[h] for h in hs])
latest=read('ten-label-case/audio-geometry-snapshot.json');flags=next(m for m in latest['moods'] if m['id']=='grim')['flags']
original=read('personal-semantic-calibration-study.json');personal=read('ten-label-case/personal-semantic-calibration-study.json')
live=read('ten-label-case/personal-fit-live-validation.json')
data={'entries':[{k:e[k] for k in ['name','album']} for e in snapshot['entries']],'current':before.tolist(),'flags':[],'methods':[]}
for f in flags:
    label=('≥ '+str(f['min'])) if f['max'] is None else ('≤ '+str(f['max'])) if f['min'] is None else str(f['min'])+'–'+str(f['max'])
    name=next(e['name'] for e in snapshot['entries'] if e['hash']==f['hash'])
    if name=='Temple of Woedica':label='Stretch / Understandable (uncertain)'
    if name=='Dawn':label='No (middling; hopeful)'
    data['flags'].append(dict(f,index=hs.index(f['hash']),label=label))
def add(id,name,scores,meaning,held,definition):
    after=np.array([scores[h] for h in hs])
    overlap=[{'name':m['name'],'before':float(spearmanr(before,[m['scores'][h] for h in hs]).statistic),'after':float(spearmanr(after,[m['scores'][h] for h in hs]).statistic)} for m in snapshot['moods'] if m['id']!='grim']
    data['methods'].append({'id':id,'name':name,'scores':after.tolist(),'meaning':meaning,'held':held,'drift':float(np.abs(after-before).mean()),'definition':definition,'overlap':overlap,
        'strongSources':len({e['album'] for e,s in zip(snapshot['entries'],after) if s>=.75})})
add('current','Current Horror',current,'Live vocabulary · reference-calibrated rarity','Baseline; no fitted model',[['Prompts',' · '.join(next(m for m in snapshot['moods'] if m['id']=='grim')['prompts'])],['Scale','Original frozen reference mapping']])
p=personal['full']
add('personal-ten','Personal fit with latest feedback — experimental',{e['hash']:e['personalFit'] for e in live['scores']},'Experimental · 10 judgments used in training · personal fit, not reference percentile','5 / 10',
    [['Positive evidence',p['positive']],['Contrasting evidence',p['negative']],['Contrast weight',str(p['strength'])],['Scale','Learned, strictly increasing; slope 0.1–3; endpoints 0 and 1'],['Feedback','Eight existing flags + Dawn No + Temple uncertain Stretch/Understandable'],['Ordering','Forlorn above Temple; no exact Temple value asserted'],['Live verification','235 songs rescored independently; 10 / 10 known bands met'],['Generalization','5 / 10 when each song and its ordering comparisons are withheld'],['Deployment','Separate read-only scorer; live app vocabulary unchanged']])
p=original['full']
add('personal-eight','Earlier personal fit — before your two checks',p['scores'],'Frozen eight-label experiment · Temple was scored before your reply','4 / 8',
    [['Positive evidence',p['positive']],['Contrasting evidence',p['negative']],['Contrast weight',str(p['strength'])],['External check','Temple 0.916; Dawn 0.463. Temple judgment subsequently clarified as uncertain.'],['Training','Original eight flags only; later feedback excluded']])
learned=next(m for m in read('learned-mood-direction-study.json')['models'] if m['dimensions']==129 and m['regularization']==.1)
add('direction','Learned embedding direction — experimental',learned['scores'],'Experimental · original eight flags fitted directly in the embedding space','3 / 8',
    [['Features','128 reference principal components plus the original prompt direction'],['Scale','Original reference calibration; variance constrained to one'],['Training','8 / 8 original flags met; 0.024 mean library movement'],['Robustness','Held-out-song and separate-window tests do not support broad generalization']])
hybrid=next(m for m in read('hybrid-acoustic-case/audio-geometry-study.json')['moods'] if m['id']=='grim')
add('hybrid','Acoustic features + MuLan feedback — experimental',hybrid['scores'],'Experimental · signal features plus semantic embeddings; original eight flags fitted','4 / 8',
    [['Features','88 tonality, pitch-interval, spectral and dynamics measurements, plus MuLan'],['Model','Local interval correction selected with nested held-out checks'],['Training','8 / 8 original flags met'],['Interpretation','Personal correction; not a reference percentile']])
clap=next(m for m in read('clap-mood-study.json')['moods'] if m['id']=='grim')
add('clap','Independent CLAP music model — current words',clap['scores'],'Different frozen model · same 235 songs and same 300 reference recordings','Not fitted',
    [['Model','LAION larger_clap_music'],['Prompts','Exact current Horror vocabulary'],['Sampling','Same eight 10-second windows where available'],['Result','2 / 8 original flags met; worsened the other current mood flags too'],['Deployment','Rejected as a replacement']])
path=base/'horror-solution-review.html'
source=path.read_text(encoding='utf-8')
payload=json.dumps(data,ensure_ascii=False,separators=(',',':')).replace('<','\\u003c')
source=re.sub(r'/\*DATA_START\*/.*?/\*DATA_END\*/',lambda _: '/*DATA_START*/'+payload+'/*DATA_END*/',source,flags=re.S)
path.write_text(source,encoding='utf-8')
assert path.stat().st_size<1_000_000
print('Packed',len(data['methods']),'methods;',len(data['entries']),'songs;',path.stat().st_size,'bytes')
