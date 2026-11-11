"""Pack actual Horror flag experiments into an inline comparison."""
import argparse
import json
import math
from pathlib import Path
import numpy as np


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('directory',type=Path);a=p.parse_args();base=a.directory
    def read(name):return json.loads((base/name).read_text(encoding='utf-8'))
    plain=read('horror-exhaustive-context-fit.json');grave=read('horror-graveyard-fit.json');cal=read('horror-personal-calibration-fit.json');search=read('horror-flag-fit.json');learner=read('horror-example-learner.json')
    assert [(f['hash'],f['min'],f['max']) for f in plain['flags']]==[(f['hash'],f['min'],f['max']) for f in grave['flags']]
    before=plain['baseline']['scores'];flags=plain['flags'];entries=plain['entries'];trials=[]
    def trial(name,lines,scores,mapping,status):
        held=[(f['min'] is None or scores[f['hash']]>=f['min']-1e-8) and (f['max'] is None or scores[f['hash']]<=f['max']+1e-8) for f in flags]
        prev=[(f['min'] is None or before[f['hash']]>=f['min']-1e-8) and (f['max'] is None or before[f['hash']]<=f['max']+1e-8) for f in flags]
        trials.append({'name':name,'lines':lines,'scores':scores,'mapping':mapping,'status':status,'held':sum(held),'newBreaks':sum(p and not h for p,h in zip(prev,held))})
    current=next(m for m in plain['vocabulary']['moods'] if m['id']=='grim')['prompts']
    trial('Current Horror',current,before,'Frozen reference','Current vocabulary; unchanged.')
    c=next(c for c in plain['validated'] if c['source']=='Kyne and Lead respected')
    trial('Closest words-only draft — incomplete',c['lines'],c['scores'],'Frozen reference','Incomplete: Kyne barely below .50; Forlorn Cliffs and Dank Bodies miss their .90 targets.')
    c=next(c for c in grave['validated'] if c['source']=='Flag coverage')
    trial('Graveyard-expanded search — incomplete',c['lines'],c['scores'],'Frozen reference','Incomplete: this search still fails your Kyne constraint. Not applied.')
    c=cal['personalCalibration']
    trial('8/8 via steep personal calibration — rejected',c['lines'],c['personalScores'],'Personal mapping; not reference bars','Rejected diagnostic: most library scores collapse near zero or one. Not a words-only fix.')
    U=np.array(grave['numericBasis']['raw']);mu=np.array(grave['numericBasis']['referenceMean']);sd=np.sqrt(np.diag(np.array(grave['numericBasis']['referenceCovariance'])))
    bars=1/(1+np.exp(-math.log(3)/(1.2816-.674)*((U-mu[:,None])/sd[:,None]-.674)))
    positions=[next(i for i,e in enumerate(grave['entries']) if name in e['name']) for name in ['Forlorn Cliffs','Kyne','Lead Your Fights']]
    controls=[{'words':s['line'],'scores':[float(bars[j,i]) for i in positions]} for j,s in enumerate(grave['selected']) if s['line'] in ['ghostly choir music','funerary choir music','gothic choir music','gothic graveyard atmosphere','haunted choir music']]
    out={'model':plain['model'],'reference':plain['reference'],'capturedAt':grave['completedAt'],'entries':entries,'flags':flags,'baseline':before,'baselineHeld':plain['baseline']['held'],'trials':trials,'graveControls':controls,
        'experiments':[
            {'name':'Default app Suggest','scope':'Shipped vocabulary and phrases','outcome':str(max(sum(f['holds'] for f in p['flags']) for p in search['searches'][0]['answer']['proposals']))+'/8 flags'},
            {'name':'Expanded deterministic phrase bank','scope':str(search['summary']['pool'])+' fear-related units','outcome':str(search['summary']['bestHeld'])+'/8 flags'},
            {'name':'More complete context search','scope':str(plain['poolCount'])+' phrases; '+f"{plain['examined']:,}"+' subsets of 40 selected units','outcome':'6/8; Kyne .498, Lead .723'},
            {'name':'Graveyard-aware search','scope':str(grave['poolCount'])+' phrases; '+f"{grave['examined']:,}"+' subsets','outcome':'6/8; no complete two-head fit found'},
            {'name':'Personal calibration diagnostic','scope':'2 extra mapping parameters','outcome':'8/8; 207/235 scores near 0 or 1; rejected'},
            {'name':'Example-based correction prototype','scope':'Audio similarity; nested leave-one-out','outcome':str(learner['summary']['nestedLooHeld'])+'/8 held out; not validated for use'}]}
    file=base/'horror-flag-fit-review.html';text=file.read_text(encoding='utf-8');payload=json.dumps(out,ensure_ascii=False,separators=(',',':')).replace('</','<\\/')
    if '__HORROR_FLAG_FIT__' in text:text=text.replace('__HORROR_FLAG_FIT__',payload)
    else:
        start=text.index('>',text.index('application/json'))+1;end=text.index('</script>',start);text=text[:start]+payload+text[end:]
    file.write_text(text,encoding='utf-8');print('Fragment bytes',file.stat().st_size)


if __name__=='__main__':main()
