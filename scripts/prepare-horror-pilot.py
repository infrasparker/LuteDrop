"""Convert the reviewed research candidate to the app's frozen profile format.

Read-only against the app database. Checks the deployed analyzer's inference;
never saves or activates the profile, never prints analyzer credentials.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('directory', type=Path)
    a = p.parse_args(); base = a.directory
    candidate = json.loads((base / 'horror-fusion-candidate.json').read_text(encoding='utf-8'))
    evidence = json.loads(Path('tagger/tagger/reference/audio-evidence.json').read_text(encoding='utf-8'))
    c = candidate['coefficients']
    words = [dict(label=axis['label'], prompts=axis['prompts'], mean=axis['reference']['mean'], sd=axis['reference']['sd'], weight=c[9+i]) for i,axis in enumerate(candidate['textAxes'])]
    identity = hashlib.sha256(json.dumps([evidence['id'], c, words], sort_keys=True).encode()).hexdigest()
    profile = {'version': 1, 'id': identity, 'evidence': evidence['id'], 'createdAt': datetime.now(timezone.utc).isoformat(),
               'trainingSignature': hashlib.sha256(json.dumps(candidate['trainingJudgments'], sort_keys=True).encode()).hexdigest(),
               'trainingCount': len(candidate['trainingJudgments']), 'bias': c[0], 'audioWeights': c[1:9], 'wordAxes': words}
    spec = importlib.util.spec_from_file_location('export', 'ui-preview/export_fixtures.py')
    export = importlib.util.module_from_spec(spec); spec.loader.exec_module(export)
    db = Path(os.environ['APPDATA']) / 'com.lutedrop/lutedrop.db'
    with sqlite3.connect(db.resolve().as_uri()+'?mode=ro', uri=True) as conn: meta = export.meta(conn)
    snapshot = json.loads((base / 'audio-geometry-snapshot.json').read_text(encoding='utf-8'))
    response = export.post(meta['autotag_url'].rstrip('/'), meta['autotag_token'], '/score',
                           {'targets': snapshot['hashes'], 'mood_prompts': {'grim': next(m['prompts'] for m in snapshot['moods'] if m['id']=='grim')}, 'profiles': {'grim': profile}}, 180)
    expected = {r['hash']: r['fit'] for r in json.loads((base / 'horror-fusion-runtime-check.json').read_text())['scores']}
    actual = {h: values['learned']['grim'] for h,values in response['mood'].items()}
    if set(expected) != set(actual): raise ValueError('The analyzer did not score all frozen tracks')
    error = max(abs(expected[h]-actual[h]) for h in expected)
    if error > 1e-6: raise ValueError(f'The deployed scorer differs from the reviewed candidate: {error}')
    (base / 'app-horror-profile.json').write_text(json.dumps(profile, indent=2)+'\n', encoding='utf-8')
    (base / 'app-horror-profile-verification.json').write_text(json.dumps({'maximumError':error, 'songs':len(actual), 'profileId':identity, 'scores':actual}, indent=2)+'\n', encoding='utf-8')
    print('Deployed combined inference verified:', len(actual), 'songs; maximum error', error)


if __name__ == '__main__': main()
