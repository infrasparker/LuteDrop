"""List only mood/feedback tables and schemas in preserved historical snapshots."""
from pathlib import Path
import os
import sqlite3

directory=Path(os.environ['APPDATA'])/'com.lutedrop/backups'
for version in [6,8,9,10,11,12,14]:
    path=next(directory.glob(f'pre-migration-v{version}-*.db'))
    with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as c:
        print('\nVERSION',version)
        columns=[r[1] for r in c.execute('PRAGMA table_info(library_entries)')]
        user_columns=[col for col in columns if any(s in col for s in ['user','feel','dark','bright','review'])]
        print('USER FIELDS',user_columns)
        for col in user_columns:
            print(col,c.execute('SELECT '+col+',COUNT(*) FROM library_entries WHERE '+col+' IS NOT NULL GROUP BY '+col+' LIMIT 8').fetchall())
        tables=[r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table'") if any(s in r[0] for s in ['mood','feel','annotation','feedback','dimension'])]
        for table in tables:
            print(table,[r[1] for r in c.execute('PRAGMA table_info('+table+')')],c.execute('SELECT COUNT(*) FROM '+table).fetchone()[0])
            print('SAMPLE',c.execute('SELECT * FROM '+table+' LIMIT 2').fetchall())
