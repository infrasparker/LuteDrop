"""Prepare/apply the directional-feedback scoring migration; never alter words or flags."""
import argparse, copy, hashlib, json, os, sqlite3, urllib.request
from datetime import datetime, timezone
from pathlib import Path

DB=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
FLOOR={'definitely':.9,'understandable':.75,'stretch':.5,'no':None}
CEILING={'definitely':None,'understandable':.9,'stretch':.75,'no':.5}

def state(conn):
    return {
        'vocabulary':json.loads(conn.execute("SELECT value FROM meta WHERE key='mood_vocabulary'").fetchone()[0]),
        'flags':conn.execute('SELECT * FROM mood_flags ORDER BY library_entry_id,mood').fetchall(),
        'levels':conn.execute('SELECT library_entry_id,mood,user_level FROM library_entry_moods WHERE user_level IS NOT NULL ORDER BY library_entry_id,mood').fetchall(),
        'entries':conn.execute("SELECT id,content_hash FROM library_entries WHERE entry_type='music' ORDER BY id").fetchall(),
        'prefs':conn.execute("SELECT value FROM meta WHERE key='mood_tune_prefs'").fetchone(),
    }

def signature(value):return hashlib.sha256(json.dumps(value,sort_keys=True).encode()).hexdigest()

def post(meta,path,body):
    req=urllib.request.Request(meta['autotag_url'].rstrip('/')+path,data=json.dumps(body).encode(),headers={'Content-Type':'application/json','X-LuteDrop-Token':meta['autotag_token']})
    with urllib.request.urlopen(req,timeout=300) as response:return json.load(response)

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('report',type=Path);parser.add_argument('--apply',action='store_true');args=parser.parse_args()
    with sqlite3.connect(DB.as_uri()+'?mode=ro',uri=True) as conn:
        source=state(conn)
        meta=dict(conn.execute('SELECT key,value FROM meta'))
        names=dict(conn.execute("SELECT content_hash,name FROM library_entries WHERE entry_type='music'"))
        entries=dict(source['entries']); targets=sorted({h for h in entries.values() if h})
        levels={}
        for entry,mood,level in source['levels']:levels.setdefault(mood,[]).append((entry,level))
        rows=conn.execute('SELECT library_entry_id,mood,kind,band,bound,bar_at_flag FROM mood_flags ORDER BY library_entry_id,mood').fetchall()
    if args.apply:
        report=json.loads(args.report.read_text(encoding='utf-8'))
        if signature(source)!=report['sourceSignature']:raise ValueError('Words, feedback or library changed; prepare again')
        backup=DB.parent/'backups'/f"before-adaptive-mood-learning-{datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S')}.db"
        backup.parent.mkdir(exist_ok=True)
        with sqlite3.connect(DB) as conn:
            with sqlite3.connect(backup) as dest:conn.backup(dest)
            conn.execute('BEGIN IMMEDIATE')
            if signature(state(conn))!=report['sourceSignature']:raise ValueError('Source changed while applying')
            conn.execute("UPDATE meta SET value=? WHERE key='mood_vocabulary'",(json.dumps(report['vocabulary']),))
            for h,values in report['scores'].items():
                for entry,content_hash in source['entries']:
                    if content_hash!=h:continue
                    for mood,value in values.items():
                        conn.execute('INSERT INTO library_entry_moods(library_entry_id,mood,auto_score) VALUES (?,?,?) ON CONFLICT(library_entry_id,mood) DO UPDATE SET auto_score=excluded.auto_score',(entry,mood,value))
            updated=state(conn)
            assert all(updated[key]==source[key] for key in ['flags','levels','entries','prefs'])
            assert all(a['prompts']==b['prompts'] for a,b in zip(source['vocabulary']['moods'],updated['vocabulary']['moods']))
            conn.commit()
        print('Applied',len(report['vocabulary']['moods']),'directional profiles. Backup:',backup)
        return
    vocabulary=copy.deepcopy(source['vocabulary']);prefs=json.loads(source['prefs'][0]) if source['prefs'] else {}
    prepared=[]
    for mood in vocabulary['moods']:
        flags=[];flagged=set()
        for entry,mid,kind,band,bound,anchor in rows:
            if mid!=mood['id'] or not entries.get(entry):continue
            lo,hi=FLOOR.get(band),CEILING.get(band);polarity=None;pivot=.5
            if kind in ['higher','lower']:
                if anchor is None:continue
                polarity=1 if kind=='higher' else -1;pivot=anchor
                lo,hi=(min(1,anchor+.1),None) if kind=='higher' else (None,max(0,anchor-.1))
            else:
                if bound=='at_least':hi=None
                elif bound=='at_most':lo=None
                if lo is None and hi is None:continue
                positive=band in ['definitely','understandable']
                if not ((positive and bound=='at_most') or (not positive and bound=='at_least')):polarity=1 if positive else -1
            flags.append(dict(hash=entries[entry],min=lo,max=hi,polarity=polarity,confidence=4 if band in ['definitely','no'] else 1,pivot=pivot,side='mid',weight=1,fresh=True))
            flagged.add(entry)
        for entry,level in levels.get(mood['id'],[]):
            if entry not in flagged and entries.get(entry):flags.append(dict(hash=entries[entry],target=level,confidence=4,side='mid',weight=1,fresh=True))
        pairs=[[entries[a],entries[b]] for a,b in prefs.get(mood['id'],{}).get('comparisons',[]) if entries.get(a) and entries.get(b)]
        request=dict(mood_id=mood['id'],name=mood['name'],scoring_version=7,current_lines=mood['prompts'],extra_words=mood['prompts'],mode='learn',flags=flags,targets=targets,comparisons=pairs,calibration=dict(source='reference',midpoint_z=.674,slope=1.8),active_profile=mood.get('profile'))
        result=post(meta,'/tune',request);proposal=result['proposals'][0]
        assert proposal['profile']['version']==7
        assert proposal['lines']==mood['prompts']
        proposal['profile']['trainingSignature']=signature(request)
        mood['profile']=proposal['profile']
        prepared.append(dict(mood=mood['id'],request=request,result=result))
        key={names.get(f['hash'],''):round(f['after'],3) for f in proposal['flags'] if any(t in names.get(f['hash'],'').lower() for t in ['bloodletting','forlorn','kyne','dawn','dank bodies'])}
        print(mood['name'],len(flags),'examples',key,flush=True)
    scored=post(meta,'/score',dict(targets=targets,mood_prompts={m['id']:m['prompts'] for m in vocabulary['moods']},profiles={m['id']:m['profile'] for m in vocabulary['moods']}))
    scores={h:values['learned'] for h,values in scored['mood'].items()}
    maximum=max(abs(scores[f['hash']][record['mood']]-f['after']) for record in prepared for f in record['result']['proposals'][0]['flags'])
    assert maximum<1e-10,maximum
    assert len(scores)==len(targets)
    with sqlite3.connect(DB.as_uri()+'?mode=ro',uri=True) as conn:assert signature(state(conn))==signature(source),'Source changed during preparation'
    args.report.write_text(json.dumps(dict(sourceSignature=signature(source),vocabulary=vocabulary,scores=scores,reviews=prepared,maximumScoreError=maximum)),encoding='utf-8')
    print('Prepared',len(vocabulary['moods']),'moods,',len(scores),'recordings. Inference parity:',maximum)

if __name__=='__main__':main()
