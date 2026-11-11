"""Recover explicit user tags from pre-migration backups; never auto tags.

Absence of a Horror tag is not an explicit negative. Export provenance so the
learner can compare conservative tagged negatives with weak absence labels.
"""
from pathlib import Path
import json
import os
import sqlite3
import sys
from collections import Counter

out=Path(sys.argv[1])
snapshot=json.loads((out/'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
byid={e['id']:e for e in snapshot['entries']}
path=next((Path(os.environ['APPDATA'])/'com.lutedrop/backups').glob('pre-migration-v6-*.db'))
with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as c:
    entries={row[0]:{'name':row[1],'type':row[2]} for row in c.execute('SELECT id,name,entry_type FROM library_entries')}
    tags={}
    sources=Counter()
    for eid,tag,source in c.execute('SELECT et.library_entry_id,t.name,et.source FROM library_entry_tags et JOIN tags t ON t.id=et.tag_id'):
        sources[source]+=1
        if source=='user':tags.setdefault(eid,[]).append(tag)
rows=[]
for eid,labels in tags.items():
    if eid not in byid or entries[eid]['type']!='music':continue
    rows.append(dict(byid[eid],historicalName=entries[eid]['name'],userTags=labels))
result={'source':path.name,'provenance':'Only library_entry_tags.source=user; matched by stable entry ID; no auto tags.',
        'linkSources':dict(sources),'entries':rows}
(out/'historical-user-tags.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
print('Recovered',len(rows),'music entries; provenance',dict(sources))
print('USER TAG COUNTS',Counter(t for e in rows for t in e['userTags']))
print('HORROR USER TAGS',[(e['name'],e['userTags']) for e in rows if {'horror','haunting','ominous'}&set(e['userTags'])])
