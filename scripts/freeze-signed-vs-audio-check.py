"""Freeze matched finalists before collecting a discriminating human check."""
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
import sys

base = Path(sys.argv[1])
source = base / 'signed-prompts-vs-audio-17.json'
report = json.loads(source.read_text(encoding='utf-8'))
snapshot = json.loads((base / 'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
entries = {e['name']: e for e in snapshot['entries']}
blind = ['Throne Of The Lost King', 'Blood on the Cobblestones', 'Frostfall', '13 If Their Bones Sleep']
trained = {f['hash'] for f in report['developmentFlags']}
if any(entries[n]['hash'] in trained for n in blind): raise ValueError('A check song has a training judgment')
names = ['text-lines-linear', 'text-bank-sparse-3-per-side', 'learned-audio-plus-current']
finalists = [m for m in report['models'] if m['name'] in names]
frozen = {'frozenAt': datetime.now(timezone.utc).isoformat(), 'sourceSha256': hashlib.sha256(source.read_bytes()).hexdigest(),
          'trainingJudgments': report['developmentFlags'], 'checkSongs': blind, 'models': finalists,
          'selection': 'Two word finalists: lowest nested-song-out interval error, and strongest compact signed-bank band count. Audio finalist: strongest nested-song-out band count.',
          'checkSelection': 'Untrained songs with substantial word/audio disagreement; not a random library accuracy sample.',
          'prompts': next(m['prompts'] for m in snapshot['moods'] if m['id'] == 'grim')}
path = base / 'frozen-signed-vs-audio-check.json'
if path.exists(): raise ValueError('Never overwrite a frozen check')
path.write_text(json.dumps(frozen, indent=2) + '\n', encoding='utf-8', newline='\n')
print('Frozen comparison SHA256:', hashlib.sha256(path.read_bytes()).hexdigest())
for n in blind: print(n, {m['name']: round(m['scores'][entries[n]['hash']], 3) for m in finalists})
