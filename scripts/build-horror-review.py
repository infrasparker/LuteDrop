"""Pack the Horror audit into an existing inline review fragment."""
import argparse
import json
from pathlib import Path


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('report',type=Path);p.add_argument('fragment',type=Path);a=p.parse_args()
    d=json.loads(a.report.read_text(encoding='utf-8'))
    current={v['moodId']:v for v in d['variants'] if v['kind']=='current'}
    out={k:d[k] for k in ['capturedAt','model','reference','summary','flags']}
    out['currentWords']=current['grim']['lines']
    out['entries']=[{k:e[k] for k in ['id','name','source','energy','energyBand','role','roleExplicit']}|
        {'horror':current['grim']['scores'][e['hash']],'ferocious':current['battle']['scores'][e['hash']]} for e in d['entries']]
    out['families']=[]
    for f in d['families']:
        byid={t['id']:t for t in f['tracks']}
        values=[[round(byid[e['id']]['score'],6),*[round(s,6) for s in byid[e['id']]['forms']],byid[e['id']]['robust']] for e in d['entries']]
        out['families'].append({k:v for k,v in f.items() if k!='tracks'}|{'values':values})
    out['pairs']=[next(pair for pair in d['familyPairs'] if {pair['a'],pair['b']}=={x,y}) for x,y in [
        ('Slow horror','Fast horror'),('Ominous','Menacing'),('Battle horror','Eldritch'),('Dread','Panic')]]
    labels={'CURRENT Horror unchanged':'Current vocabulary — unchanged','Semantic arrangement 1':'Best searched refinement — not applied',
        'Remove Horror line 4':'Remove added ominous line — diagnostic','Replace with Eldritch':'Eldritch replacement — rejected',
        'Replace with Battle horror':'Battle horror replacement — rejected','Replace with Ominous':'Ominous replacement — diagnostic'}
    out['trials']=[]
    for name,label in labels.items():
        trial=next(t for t in d['tuner'] if t['name']==name)
        movement=next(({k:v[k] for k in ['drift','newHigh','lostHigh']} for v in d['variants'] if v['name']==name and v['kind']=='arranged'),None)
        out['trials'].append(trial|{'displayName':label,'movement':movement})
    text=a.fragment.read_text(encoding='utf-8');payload=json.dumps(out,ensure_ascii=False,separators=(',',':')).replace('</','<\\/')
    if '__HORROR_DATA__' in text:text=text.replace('__HORROR_DATA__',payload)
    else:
        start=text.index('>',text.index('application/json'))+1;end=text.index('</script>',start)
        text=text[:start]+payload+text[end:]
    a.fragment.write_text(text,encoding='utf-8');print('Fragment bytes',a.fragment.stat().st_size)


if __name__=='__main__':main()
