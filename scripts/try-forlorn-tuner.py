"""Read-only live tuner trials using the user's Ominous imagery and current flags."""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import time

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
        def post(ep, body=None):
            return r.export.post(meta['autotag_url'].rstrip('/'), meta['autotag_token'], ep, body, 240)
        for _ in range(10):
            health = post('/health')
            if health.get('ready'):
                break
            time.sleep(5)
        if not health.get('ready'):
            raise RuntimeError('Analyzer not ready')
        families = json.loads((ROOT / 'scripts/forlorn-ominous-sets.json').read_text(encoding='utf-8'))
        old = json.loads((a.out.parent / 'horror-exhaustive-context-fit.json').read_text(encoding='utf-8'))
        pool = r.export.dedupe([line for f in families for line in f['sets']] + [s['line'] for s in old['selected']])
        names = dict(conn.execute('SELECT content_hash,name FROM library_entries'))
        out = {'capturedAt': datetime.now(timezone.utc).isoformat(), 'pool': pool, 'health': health, 'trials': []}
        for mode in ['arrange', 'suggest']:
            req, _, _ = r.export.build_request(conn, meta, 'grim', mode, pool, [], [], focus='')
            print('SEARCH', mode, 'semantic lines', len(pool), flush=True)
            start = time.monotonic()
            result = post('/tune', req)
            out['trials'].append({'mode': mode, 'seconds': time.monotonic() - start, 'flags': req['flags'], 'result': result})
            a.out.write_text(json.dumps(out, indent=2) + '\n', encoding='utf-8')
            for proposal in result['proposals']:
                print('PROPOSAL', sum(f['holds'] for f in proposal['flags']), '/', len(proposal['flags']), 'loss', round(proposal['loss'], 3), proposal['lines'], flush=True)
                for f in proposal['flags']:
                    print(' ', names[f['hash']], round(f['after'], 3), f['holds'], flush=True)
            print('SPLIT', bool(result['split']), 'seconds', round(time.monotonic() - start, 1), flush=True)


if __name__ == '__main__':
    main()
