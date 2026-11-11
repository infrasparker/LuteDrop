"""Build a read-only research manifest, without saving analyzer credentials."""
import json
import os
from pathlib import Path
import sqlite3
import sys

root=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(root/'tagger'))
from tagger.tune_vocab import VOCAB
from tagger.tune_phrases import PHRASES

directory=Path(sys.argv[1])
snapshot=json.loads((directory/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
conn=sqlite3.connect((Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db').resolve().as_uri()+'?mode=ro',uri=True)
meta=dict(conn.execute('SELECT key,value FROM meta'))
prefix=meta['autotag_local_prefix'].replace('\\','/').rstrip('/')
remote=meta.get('autotag_remote_prefix','/media/audio').rstrip('/')
paths={h:p.replace('\\','/') for h,p in conn.execute('SELECT content_hash,file_path FROM library_entries')}
entries=[]
for h in snapshot['hashes']:
    path=paths[h]
    if not path.lower().startswith(prefix.lower()+'/'):raise ValueError('Unmapped audio path')
    entries.append({'hash':h,'path':remote+path[len(prefix):]})
families=json.loads((root/'scripts/forlorn-ominous-sets.json').read_text(encoding='utf-8'))
texts=list(dict.fromkeys(VOCAB+PHRASES+[line for m in snapshot['moods'] for line in m['prompts']]+[line for f in families for line in f['sets']]))
(directory/'clap-manifest.json').write_text(json.dumps({'entries':entries,'texts':texts},indent=2)+'\n',encoding='utf-8')
print('Prepared',len(entries),'music files;',len(texts),'descriptions')
