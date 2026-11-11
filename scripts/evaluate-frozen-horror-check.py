"""Evaluate the frozen scorer against later human feedback, without refitting."""
import hashlib
import json
from pathlib import Path
import sys

base=Path(sys.argv[1]);path=base/'frozen-two-evidence-horror.json'
blob=path.read_text(encoding='utf-8').encode('utf-8');frozen=json.loads(blob)
assert hashlib.sha256(blob).hexdigest()=='2f2287895495efd91ea3cf641172a47eabde8baf0704aa88f459f8470a60bb44'
snapshot=json.loads((base/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
byname={e['name']:e for e in snapshot['entries']};baseline=next(m for m in snapshot['moods'] if m['id']=='grim')['scores']
labels=[{'song':'Wind from the Depths','min':.75,'max':None,'label':'Understandable or higher'},
        {'song':'Burial Isle','min':.75,'max':.9,'label':'Firmly Understandable; lower than Forlorn'},
        {'song':'Far Horizons','min':None,'max':.5,'label':'No'},
        {'song':'The Cove Battle','min':.75,'max':None,'label':'Understandable or higher'}]
test=[]
for label in labels:
    h=byname[label['song']]['hash'];v=frozen['scores'][h];before=baseline[h]
    def holds(score):return (label['min'] is None or score>=label['min']) and (label['max'] is None or score<=label['max'])
    test.append(dict(label,score=v,current=before,bandHolds=holds(v),currentBandHolds=holds(before),
                     categoryHolds=bool((v<=.5) if label['min'] is None else (v>=.75))))
forlorn=frozen['scores'][byname['Forlorn Cliffs']['hash']];burial=frozen['scores'][byname['Burial Isle']['hash']];kyne=frozen['scores'][byname["Kyne's Peace"]['hash']]
report={'frozenModelCanonicalLfSha256':hashlib.sha256(blob).hexdigest(),'refittedAfterFeedback':False,'results':test,
    'summary':{'coarseCategoryHeld':sum(t['categoryHolds'] for t in test),'gradedBandsHeld':sum(t['bandHolds'] for t in test),'testSongs':len(test)},
    'newOrderings':[{'higher':'Forlorn Cliffs','lower':'Burial Isle','holds':forlorn>burial,'difference':forlorn-burial}]+[
        {'higher':name,'lower':"Kyne's Peace",'holds':frozen['scores'][byname[name]['hash']]>kyne} for name in ['Wind from the Depths','Burial Isle','The Cove Battle']],
    'limitations':['Four test songs cannot establish population-wide reliability.','All ten earlier judgments helped fit the scale.','Historical labels are proxy supervision.','Band judgments describe personal fit, not a calibrated probability.']}
(base/'frozen-horror-independent-evaluation.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
print(report['summary'])
for row in test:print(row['song'],'current',round(row['current'],3),'frozen',round(row['score'],3),'grade holds',row['bandHolds'])
print('RELATIVE',report['newOrderings'])
