"""Read-only comparison of Horror descriptions under the strength-rating fitter."""
import copy
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import sqlite3
import urllib.request

BASE = Path(r'C:\Users\Infrasparker\.codex\visualizations\2026\11\08\01a1b8e6-30b2-7ea0-b750-7cc1eefbb98e')


def main():
    original = json.loads((BASE/'horror-strength-ratings-check.json').read_text(encoding='utf-8'))
    request = original['request']
    with sqlite3.connect((Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db').as_uri()+'?mode=ro', uri=True) as conn:
        meta = dict(conn.execute('SELECT key,value FROM meta'))
        vocabulary = json.loads(meta['mood_vocabulary'])
        saved = next(m for m in vocabulary['moods'] if m['id'] == request['mood_id'])
        assert saved['prompts'] == request['current_lines'], 'Saved wording changed; refresh the source request'
        names = dict(conn.execute("SELECT content_hash,name FROM library_entries WHERE entry_type='music'"))
    request['active_profile'] = saved['profile']
    positive = [line for line in saved['prompts'] if not line.startswith('-')]
    candidates = {
        'current': saved['prompts'],
        'slow_and_violent': [
            'eerie, haunting, ghostly, spectral',
            'unnerving, disturbing, sinister, menacing',
            'ominous, foreboding, desolate, funereal',
            'violent, frantic, nightmarish, terrifying',
            '- safe, reassuring, comforting, carefree',
        ],
        'unsettling_contrast': [
            'eerie, haunting, ghostly, spectral',
            'unnerving, unsettling, disturbing, sinister',
            'dark, ominous, dread, horror',
            'somber, desolate, bleak, funereal',
            '- safe, reassuring, cheerful',
        ],
        'horror_two_facets': [
            'haunted, eerie, ominous, foreboding',
            'terrifying, horrific, nightmarish, sinister',
            '- safe, reassuring, carefree, cheerful',
        ],
        'without_uplifting': [line for line in saved['prompts'] if 'uplifting' not in line],
        'positive_only': positive,
        'safety_contrast': positive + ['- safe, reassuring, comforting, carefree'],
        'macabre': positive + ['macabre, grotesque, monstrous, terrifying', '- safe, reassuring, carefree, cheerful'],
        'threat_and_desolation': [
            'eerie, haunting, ghostly, spectral',
            'sinister, threatening, menacing, dread',
            'macabre, grotesque, horrific, terrifying',
            'ominous, bleak, desolate, funereal',
            '- safe, reassuring, carefree, cheerful',
        ],
        'horror_music': [
            'eerie, haunting, ghostly, spectral music',
            'sinister, disturbing, terrifying horror music',
            'dark, ominous, threatening music',
            'bleak, desolate, funereal music',
            '- safe, reassuring, carefree music',
        ],
    }
    targets = {f['hash']:f['target'] for f in request['flags']}
    def run(item):
        label, lines = item
        if label == 'current': result = original['result']
        elif (BASE/'horror-strength-word-diagnosis.json').exists() and label in json.loads((BASE/'horror-strength-word-diagnosis.json').read_text(encoding='utf-8')):
            result=json.loads((BASE/'horror-strength-word-diagnosis.json').read_text(encoding='utf-8'))[label]['result']
        else:
            body = copy.deepcopy(request); body['extra_words'] = lines
            req = urllib.request.Request(meta['autotag_url'].rstrip('/')+'/tune',data=json.dumps(body).encode(),headers={'Content-Type':'application/json','X-LuteDrop-Token':meta['autotag_token']})
            with urllib.request.urlopen(req,timeout=300) as response: result=json.load(response)
        proposal = result['proposals'][0]
        flags = proposal['flags']
        summary = dict(name=label,mae=sum(abs(f['after']-targets[f['hash']]) for f in flags)/len(flags), validation=proposal['validation_mae'],
                       key={names.get(f['hash'],''):round(f['after'],3) for f in flags if any(t in names.get(f['hash'],'').lower() for t in ['bloodletting','forlorn','kyne','dawn','burial','cove battle','lead your'])})
        print(json.dumps(summary),flush=True)
        return label,dict(lines=lines,summary=summary,result=result)
    with ThreadPoolExecutor(max_workers=2) as pool: results = dict(pool.map(run,candidates.items()))
    (BASE/'horror-strength-word-diagnosis.json').write_text(json.dumps(results),encoding='utf-8')


if __name__ == '__main__': main()
