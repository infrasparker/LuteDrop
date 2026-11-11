"""Install the explicitly reviewed Horror pilot after the app has stopped.

Dry-run by default. Backs up SQLite before applying; preserves every other mood,
manual level, tag, track and scene. Only the earlier 17 spoken judgments are
recorded; the four latest comparison songs are not added to training.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import sqlite3
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--apply', action='store_true')
    a = parser.parse_args(); base = a.directory
    profile = json.loads((base / 'app-horror-profile.json').read_text(encoding='utf-8'))
    verified = json.loads((base / 'app-horror-profile-verification.json').read_text(encoding='utf-8'))
    study = json.loads((base / 'signed-prompts-vs-audio-17-directional.json').read_text(encoding='utf-8'))
    if verified['profileId'] != profile['id'] or verified['maximumError'] > 1e-6: raise ValueError('Verify the deployed scorer first')
    db = Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db'
    conn = sqlite3.connect(str(db) if a.apply else db.resolve().as_uri()+'?mode=ro', uri=not a.apply)
    vocabulary = json.loads(conn.execute("SELECT value FROM meta WHERE key='mood_vocabulary'").fetchone()[0])
    mood = next(m for m in vocabulary['moods'] if m['id']=='grim')
    entries = {}
    for entry_id, content_hash in conn.execute("SELECT id,content_hash FROM library_entries WHERE entry_type='music'"):
        entries.setdefault(content_hash, []).append(entry_id)
    planned = []
    for flag in study['developmentFlags']:
        lo, hi = flag['min'], flag['max']
        band, bound = ('definitely','at_least') if lo is not None and lo >= .9 else ('understandable','at_least') if lo is not None and lo >= .75 else ('no','in') if hi is not None and hi <= .5 else ('stretch','at_most') if hi is not None and hi <= .75 else ('understandable','at_most')
        ids = entries.get(flag['hash'], [])
        if not ids: raise ValueError(f"Missing training song: {flag['song']}")
        for entry_id in ids:
            existing = conn.execute("SELECT kind,band,bound FROM mood_flags WHERE library_entry_id=? AND mood='grim'",(entry_id,)).fetchone()
            wanted = ('band',band,bound)
            if existing and existing != wanted and not (flag['song']=='Lead Your Fights' and existing==('band','stretch','in')):
                raise ValueError(f"A newer manual judgment conflicts for {flag['song']}; preserve it and review the proposal")
            planned.append((entry_id,band,bound,existing))
    print('Pilot:', profile['trainingCount'], 'training examples;', sum(p[3] is None for p in planned), 'spoken judgments to record;',len(verified['scores']),'verified scores')
    if not a.apply:
        print('Dry run only; no writes.')
        return
    backup = db.parent/'backups'/f"before-learned-profile-{datetime.now(timezone.utc).strftime('%Y%m%d-%H%M%S')}.db"
    backup.parent.mkdir(parents=True,exist_ok=True)
    with sqlite3.connect(backup) as destination: conn.backup(destination)
    signature = '\n'.join(mood['prompts'] + ['@learned:'+profile['id']])
    now = int(time.time()*1000)
    with conn:
        mood['profile'] = profile
        conn.execute("UPDATE meta SET value=? WHERE key='mood_vocabulary'",(json.dumps(vocabulary),))
        for entry_id,band,bound,existing in planned:
            if existing:
                conn.execute("UPDATE mood_flags SET bound=?,lines_sig=? WHERE library_entry_id=? AND mood='grim'",(bound,signature,entry_id))
            else:
                conn.execute("INSERT INTO mood_flags(library_entry_id,mood,kind,band,bound,lines_sig,created_at) VALUES (?,'grim','band',?,?,?,?)",(entry_id,band,bound,signature,now))
        for content_hash,score in verified['scores'].items():
            for entry_id in entries.get(content_hash, []):
                conn.execute("INSERT INTO library_entry_moods(library_entry_id,mood,auto_score) VALUES (?,'grim',?) ON CONFLICT(library_entry_id,mood) DO UPDATE SET auto_score=excluded.auto_score",(entry_id,score))
    print('Activated reviewed profile. Database backup:',backup)


if __name__=='__main__': main()
