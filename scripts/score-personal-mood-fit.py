"""Run a portable personal mood scorer through the live analyzer, read-only.

No app settings, flags, calibration or stored mood values are modified. This
separate prototype scores every current music entry, including newly added
tracks once the analyzer has cached their audio. Credentials are never saved.
"""
import argparse
from contextlib import closing
import importlib.util
import json
import os
from pathlib import Path
import sqlite3

import numpy as np
from scipy.special import expit

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('fixture_export',ROOT/'ui-preview/export_fixtures.py')
export=importlib.util.module_from_spec(spec);spec.loader.exec_module(export)


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('profile',type=Path);p.add_argument('--out',type=Path,required=True);a=p.parse_args()
    profile=json.loads(a.profile.read_text(encoding='utf-8'))
    if profile['schema']!='lutedrop.personal-mood-fit/v1':raise ValueError('Unknown scorer schema')
    knots=np.array(profile['fitMapping']['input']);values=np.array(profile['fitMapping']['output'])
    if not (len(knots)==len(values)>=2 and knots[0]==values[0]==0 and knots[-1]==values[-1]==1):raise ValueError('Invalid curve endpoints')
    slope=np.diff(values)/np.diff(knots)
    if not np.isfinite(slope).all() or (slope<.1-1e-6).any() or (slope>3+1e-6).any():raise ValueError('Invalid curve slope')
    with closing(sqlite3.connect((Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db').resolve().as_uri()+'?mode=ro',uri=True)) as live,closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=export.meta(conn)
        entries=[dict(zip(['id','name','hash'],row)) for row in conn.execute("SELECT id,name,content_hash FROM library_entries WHERE entry_type='music' ORDER BY name,id")]
        hashes=list(dict.fromkeys(e['hash'] for e in entries))
        vocabulary=export.vocabulary(meta)
        answer=export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/score',
            {'targets':hashes,'mood_prompts':{'positive':profile['positive'],'negative':profile['negative']},'energy_prompts':vocabulary['energy']},180)
    if answer['model']!=profile['model']:raise ValueError('Model changed; rebuild this profile')
    ref=answer['mood_reference']
    if (ref['set'],ref['tracks'])!=(profile['reference']['set'],profile['reference']['tracks']):raise ValueError('Reference changed; rebuild this profile')
    expected_mean=ref['stats']['positive']['mean']-profile['negativeWeight']*ref['stats']['negative']['mean']
    if abs(expected_mean-profile['reference']['mean'])>1e-6:raise ValueError('Text embeddings changed; rebuild this profile')
    results=[];missing=[];mapping=profile['referenceMapping']
    for e in entries:
        if e['hash'] not in answer['mood']:missing.append(e);continue
        scores=answer['mood'][e['hash']]['moods']
        raw=scores['positive']-profile['negativeWeight']*scores['negative']
        z=(raw-profile['reference']['mean'])/profile['reference']['sd']
        base=float(expit(mapping['slope']*(z-mapping['midpoint_z'])))
        fit=float(np.interp(base,knots,values))
        results.append(dict(e,referenceScore=base,personalFit=fit))
    byhash={e['hash']:e for e in results}
    flags=[]
    for f in profile['trainingFlags']:
        if f['hash'] not in byhash:continue
        e=byhash[f['hash']];value=e['personalFit']
        flags.append(dict(song=e['name'],score=value,holds=bool((f['min'] is None or value>=f['min']-1e-6) and (f['max'] is None or value<=f['max']+1e-6))))
    results.sort(key=lambda e:-e['personalFit'])
    a.out.write_text(json.dumps({'profile':profile['name'],'meaning':profile['meaning'],'evidence':profile['evidence'],'flags':flags,'scores':results,'missing':missing},indent=2)+'\n',encoding='utf-8')
    print('Read-only live verification:',len(results),'songs;',sum(f['holds'] for f in flags),'/',len(flags),'flags; missing',len(missing))
    for f in flags:print(f['song'],round(f['score'],3),f['holds'])


if __name__=='__main__':main()
