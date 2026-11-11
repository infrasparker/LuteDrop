"""Inspect preserved pre-migration labels without reading credentials or editing backups."""
from pathlib import Path
import os
import sqlite3

directory=Path(os.environ['APPDATA'])/'com.lutedrop/backups'
for pattern in ['lutedrop.pre-opus*','pre-migration-v6-*','manual-pre-tagcleanup*','pre-migration-v13-*']:
    for path in directory.glob(pattern+'.db'):
        with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as c:
            tables={r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            print('\nBACKUP',path.name)
            for table in ['tags','library_entry_tags','library_entries','autotag_scores','tag_feedback']:
                if table not in tables:continue
                columns=[r[1] for r in c.execute('PRAGMA table_info('+table+')')]
                print('TABLE',table,columns,'count',c.execute('SELECT COUNT(*) FROM '+table).fetchone()[0])
            print('TAGS',c.execute('SELECT * FROM tags LIMIT 45').fetchall())
            print('TAG LINKS',c.execute('SELECT * FROM library_entry_tags LIMIT 3').fetchall())
