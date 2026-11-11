"""Read-only snapshot of current music, mood flags and analyzer scores. No credentials saved."""
import argparse
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import sqlite3

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('fit', ROOT / 'scripts/fit-horror-flags.py')
fit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fit)
r = fit.r


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out', type=Path, required=True)
    a = p.parse_args()
    with closing(sqlite3.connect((Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db').resolve().as_uri() + '?mode=ro', uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn)
        meta = r.export.meta(conn)
        vocab = r.export.vocabulary(meta)
        entries = [dict(zip(['id', 'name', 'hash', 'path'], row)) for row in conn.execute("SELECT id,name,content_hash,file_path FROM library_entries WHERE entry_type='music' ORDER BY content_hash,id")]
        for e in entries:
            e['album'] = r.export.album_of(e.pop('path'))
        hashes = list(dict.fromkeys(e['hash'] for e in entries))
        def post(ep, body):
            return r.export.post(meta['autotag_url'].rstrip('/'), meta['autotag_token'], ep, body, 180)
        scored = post('/score', {'targets': hashes, 'mood_prompts': {m['id']: m['prompts'] for m in vocab['moods']}, 'energy_prompts': vocab['energy']})
        moods = []
        for mood in vocab['moods']:
            req, skipped, _ = r.export.build_request(conn, meta, mood['id'], 'compare', [], [], [mood['prompts']], focus='')
            stat = scored['mood_reference']['stats'][mood['id']]
            moods.append(dict(mood, flags=req['flags'], skipped=skipped, calibration=req['calibration'],
                raw={h: scored['mood'][h]['moods'][mood['id']] for h in hashes},
                scores={h: r.spread.calibrated(scored['mood'][h]['moods'][mood['id']], stat) for h in hashes}))
        output = {'entries': entries, 'hashes': hashes, 'moods': moods, 'reference': scored['mood_reference'], 'model': scored['model']}
        a.out.write_text(json.dumps(output, indent=2) + '\n', encoding='utf-8')
        print('Snapshot:', len(hashes), 'songs;', [(m['name'], len(m['flags'])) for m in moods])


if __name__ == '__main__':
    main()
