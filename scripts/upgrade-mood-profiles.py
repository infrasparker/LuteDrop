"""Prepare or apply reviewed v2 profiles for every mood; back up before writes.

Preparation reads the live DB and analyzer. Apply uses only the prepared file,
rejects changed source data, and preserves manual levels, flags and mixer data.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
from urllib.error import HTTPError


def fingerprint(conn):
    data = [conn.execute("SELECT value FROM meta WHERE key='mood_vocabulary'").fetchone()[0],
            conn.execute('SELECT * FROM mood_flags ORDER BY library_entry_id,mood').fetchall(),
            conn.execute('SELECT library_entry_id,mood,user_level FROM library_entry_moods WHERE user_level IS NOT NULL ORDER BY library_entry_id,mood').fetchall()]
    return hashlib.sha256(json.dumps(data).encode()).hexdigest()


def main():
    p = argparse.ArgumentParser(description=__doc__); p.add_argument('report', type=Path); p.add_argument('--apply', action='store_true')
    a = p.parse_args(); db = Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    conn = sqlite3.connect(db if a.apply else db.resolve().as_uri()+'?mode=ro', uri=not a.apply)
    if a.apply:
        report = json.loads(a.report.read_text(encoding='utf-8'))
        if fingerprint(conn) != report['fingerprint']: raise ValueError('Source preferences or vocabulary changed; prepare again')
        backup = db.parent/'backups'/f"before-general-moods-{datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S')}.db"
        backup.parent.mkdir(exist_ok=True)
        with sqlite3.connect(backup) as target: conn.backup(target)
        with conn:
            conn.execute("UPDATE meta SET value=? WHERE key='mood_vocabulary'", (json.dumps(report['vocabulary']),))
            for mood in report['vocabulary']['moods']:
                sig = '\n'.join(mood['prompts']+['@learned:'+mood['profile']['id']])
                conn.execute('UPDATE mood_flags SET lines_sig=? WHERE mood=?', (sig, mood['id']))
            for h, values in report['scores'].items():
                for entry_id, in conn.execute('SELECT id FROM library_entries WHERE content_hash=? AND entry_type=\'music\'', (h,)).fetchall():
                    for mood, score in values.items():
                        conn.execute('INSERT INTO library_entry_moods(library_entry_id,mood,auto_score) VALUES (?,?,?) ON CONFLICT(library_entry_id,mood) DO UPDATE SET auto_score=excluded.auto_score', (entry_id,mood,score))
        print('Applied', len(report['vocabulary']['moods']), 'profiles. Backup:', backup)
        return
    spec = importlib.util.spec_from_file_location('export', Path(__file__).resolve().parents[1]/'ui-preview/export_fixtures.py')
    export = importlib.util.module_from_spec(spec); spec.loader.exec_module(export)
    meta = export.meta(conn); url = meta['autotag_url'].rstrip('/'); token = meta['autotag_token']
    vocabulary = json.loads(conn.execute("SELECT value FROM meta WHERE key='mood_vocabulary'").fetchone()[0])
    targets = [row[0] for row in conn.execute("SELECT DISTINCT content_hash FROM library_entries WHERE entry_type='music' AND content_hash IS NOT NULL ORDER BY content_hash")]
    prepared = []; source = fingerprint(conn)
    for mood in vocabulary['moods']:
        rows = conn.execute('SELECT e.content_hash,f.kind,f.band,f.bound,f.bar_at_flag FROM mood_flags f JOIN library_entries e ON e.id=f.library_entry_id WHERE f.mood=? ORDER BY e.content_hash', (mood['id'],)).fetchall()
        flags = []; flagged = set()
        for h, kind, band, bound, at in rows:
            if not h: continue
            lo, hi = {'definitely':(.9,None), 'understandable':(.75,.9), 'stretch':(.5,.75), 'no':(None,.5)}.get(band,(None,None))
            if kind == 'higher' and at is not None: lo,hi=min(1.,at+.1),None
            elif kind == 'lower' and at is not None: lo,hi=None,max(0.,at-.1)
            elif bound == 'at_least': hi=None
            elif bound == 'at_most': lo=None
            if lo is None and hi is None: continue
            flags.append(dict(hash=h,min=lo,max=hi,weight=1.,fresh=True,side='high' if lo is not None else 'low')); flagged.add(h)
        for h, level in conn.execute('SELECT e.content_hash,m.user_level FROM library_entry_moods m JOIN library_entries e ON e.id=m.library_entry_id WHERE m.mood=? AND m.user_level IS NOT NULL', (mood['id'],)):
            if not h or h in flagged: continue
            lo,hi=(.9,None) if level>=1 else (.75,None) if level>=.75 else (None,.75) if level>=.5 else (None,.5)
            flags.append(dict(hash=h,min=lo,max=hi,weight=1.,fresh=True,side='high' if lo is not None else 'low')); flagged.add(h)
        description = mood['prompts']
        if mood.get('profile',{}).get('version') == 1:
            # Replace the dormant legacy prompt pool with the actual semantic
            # evidence from the accepted profile, so editable wording is honest.
            description = [('' if axis['weight'] >= 0 else '- ')+', '.join(axis['prompts']) for axis in mood['profile']['wordAxes'] if abs(axis['weight']) > .0001]
        body = dict(mood_id=mood['id'],name=mood['name'],current_lines=mood['prompts'],extra_words=description,
                    mode='learn',targets=targets,flags=flags,calibration=dict(source='reference',midpoint_z=.674,slope=1.8),active_profile=mood.get('profile'))
        try: result = export.post(url,token,'/tune',body,300)
        except HTTPError as error:
            raise ValueError(f"{mood['name']}: {error.read().decode('utf-8')}") from None
        draft = result['proposals'][0]
        mood['prompts'] = description; mood['profile'] = draft['profile']
        prepared.append(dict(mood=mood['id'], name=mood['name'], examples=len(flags), review=draft))
        print(mood['name'], len(flags), 'examples;', sum(f['holds'] for f in draft['flags']), 'within preferences;', flush=True)
    response = export.post(url,token,'/score',dict(targets=targets,mood_prompts={m['id']:m['prompts'] for m in vocabulary['moods']},profiles={m['id']:m['profile'] for m in vocabulary['moods']}),300)
    scores = {h: response['mood'][h]['learned'] for h in targets}
    if any(not 0 < value < 1 for values in scores.values() for value in values.values()): raise ValueError('A smooth score reached a flat endpoint')
    a.report.write_text(json.dumps(dict(fingerprint=source,vocabulary=vocabulary,reviews=prepared,scores=scores),indent=2)+'\n',encoding='utf-8')
    print('Prepared',len(prepared),'moods for',len(scores),'recordings; no data changed')


if __name__ == '__main__': main()
