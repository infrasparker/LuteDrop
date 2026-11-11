"""Read-only production check: same-wording feedback updates and frozen inference.

Uses the configured analyzer and current saved vocabulary, flags and overrides.
Never changes the library, flags, wording, or accepted models. Credentials are
read in memory and are never included in the output report.
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import urllib.request


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mood',action='append',help='Mood id; repeat, or omit for all')
    parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args()
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with sqlite3.connect(db.as_uri()+'?mode=ro',uri=True) as conn:
        meta=dict(conn.execute('SELECT key,value FROM meta'))
        entries=dict(conn.execute("SELECT id,content_hash FROM library_entries WHERE entry_type='music'"))
        names=dict(conn.execute("SELECT content_hash,name FROM library_entries WHERE entry_type='music'"))
        rows=conn.execute('SELECT library_entry_id,mood,kind,band,bound,bar_at_flag FROM mood_flags ORDER BY library_entry_id,mood').fetchall()
        levels=conn.execute('SELECT library_entry_id,mood,user_level FROM library_entry_moods WHERE user_level IS NOT NULL').fetchall()
    vocabulary=json.loads(meta['mood_vocabulary'])
    prefs=json.loads(meta.get('mood_tune_prefs','{}'))
    targets=sorted({h for h in entries.values() if h})

    def post(path,body):
        request=urllib.request.Request(meta['autotag_url'].rstrip('/')+path,
            data=json.dumps(body).encode(),headers={'Content-Type':'application/json','X-LuteDrop-Token':meta['autotag_token']})
        with urllib.request.urlopen(request,timeout=300) as response:return json.load(response)

    report=[]
    for mood in vocabulary['moods']:
        mid=mood['id']
        if args.mood and mid not in args.mood:continue
        old=mood.get('profile')
        if not old or old['version']<6:continue
        flags=[];flagged=set()
        for entry,ident,kind,band,bound,at in rows:
            if ident!=mid or not entries.get(entry):continue
            lo={'definitely':.9,'understandable':.75,'stretch':.5,'no':None}.get(band)
            hi={'definitely':None,'understandable':.9,'stretch':.75,'no':.5}.get(band)
            polarity=None;pivot=.5
            if kind in ('higher','lower'):
                if at is None:continue
                polarity=1 if kind=='higher' else -1;pivot=at
                lo,hi=(min(1,at+.1),None) if kind=='higher' else (None,max(0,at-.1))
            else:
                if bound=='at_least':hi=None
                elif bound=='at_most':lo=None
                if lo is None and hi is None:continue
                positive=band in ('definitely','understandable')
                if not ((positive and bound=='at_most')or(not positive and bound=='at_least')):
                    polarity=1 if positive else -1
            flags.append(dict(hash=entries[entry],min=lo,max=hi,polarity=polarity,pivot=pivot,
                confidence=4 if band in ('definitely','no')else 1,side='mid'))
            flagged.add(entry)
        for entry,ident,level in levels:
            if ident==mid and entry not in flagged and entries.get(entry):
                flags.append(dict(hash=entries[entry],target=level,confidence=4,side='mid'))
        pairs=[[entries[a],entries[b]]for a,b in prefs.get(mid,{}).get('comparisons',[])if entries.get(a)and entries.get(b)]
        request=dict(mood_id=mid,name=mood['name'],scoring_version=8,current_lines=mood['prompts'],
            extra_words=mood['prompts'],active_profile=old,mode='learn',flags=flags,targets=targets,
            comparisons=pairs,calibration=dict(source='reference',midpoint_z=.674,slope=1.8))
        result=post('/tune',request)['proposals'][0];new=result['profile']
        assert all(new[k]==old.get(k) for k in ('bias','audioWeights','wordAxes','local','method'))
        assert new['wordVersion']==old.get('wordVersion',old['version'])
        scored=post('/score',dict(targets=targets,mood_prompts={'saved':mood['prompts'],'preview':mood['prompts']},profiles={'saved':old,'preview':new}))['mood']
        parity=max((abs(scored[f['hash']]['learned']['preview']-f['after'])for f in result['flags']),default=0)
        assert parity<1e-10,parity
        changes=[dict(hash=h,name=names.get(h,h),before=x['learned']['saved'],after=x['learned']['preview'])for h,x in scored.items()]
        changes.sort(key=lambda x:abs(x['after']-x['before']),reverse=True)
        report.append(dict(mood=mid,profile=new,parity=parity,scores=changes))
        notable={x['name']:[round(x['before'],3),round(x['after'],3)]for x in changes if any(w in x['name'].lower()for w in ('lead your','hushed','twilight','forlorn','bloodletting'))}
        print(mid,len(flags),'examples; changes > .1:',sum(abs(x['after']-x['before'])>.1 for x in changes),notable,flush=True)
    args.out.parent.mkdir(parents=True,exist_ok=True)
    args.out.write_text(json.dumps(report),encoding='utf-8')


if __name__=='__main__':main()
