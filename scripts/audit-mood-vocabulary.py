"""Evaluate candidate word sets as a WHOLE-LIBRARY vocabulary, not anecdotes.

python scripts/audit-mood-vocabulary.py --base <battle-tests.json> --extra <qualifier-tests.json> --out <audit.json>

No analyzer rescoring is needed: both inputs contain genuine /score results for
every track and must share the exact baseline vocabulary, hashes and reference.
Measures support, wording consistency, album concentration, full ranking and
high-set overlap. Enumerates small, nonredundant portfolios instead of giving
every song its own adjective. Results are diagnostic, not semantic ground truth.
"""
import argparse
from collections import Counter
import importlib.util
import itertools
import json
import os
from pathlib import Path
import sqlite3
import statistics
from contextlib import closing

spec=importlib.util.spec_from_file_location('prompt_comparison',Path(__file__).with_name('compare-mood-prompts.py'))
comparison=importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


def jaccard(a,b):
    return (a&b).bit_count()/max(1,(a|b).bit_count())


def mask(indices):
    result=0
    for i in indices:
        result|=1<<i
    return result


def consistency_support(combined,forms,threshold=.75,floor=.5):
    """Require combined support, two agreeing formulations, and no collapse."""
    return combined>=threshold and sum(s>=threshold for s in forms)>=2 and min(forms)>=floor


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base',type=Path,required=True)
    parser.add_argument('--extra',type=Path,required=True)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--db',type=Path,default=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db')
    args=parser.parse_args()
    a=json.loads(args.base.read_text(encoding='utf-8'));b=json.loads(args.extra.read_text(encoding='utf-8'))
    if a['vocabulary']!=b['vocabulary'] or a['current']!=b['current'] or a['reference']!=b['reference']:
        raise ValueError('Input studies do not share the same baseline and reference')
    hashes=sorted(a['currentMax']);entries={e['hash']:e for e in a['entries']}
    if set(hashes)!={e['hash'] for e in b['entries']}:
        raise ValueError('Input library snapshots differ')
    tracks=[entries[h] for h in hashes]
    with closing(sqlite3.connect(args.db.resolve().as_uri()+'?mode=ro',uri=True)) as conn:
        ost_tags={eid:tag for eid,tag in conn.execute("SELECT et.library_entry_id,t.name FROM library_entry_tags et JOIN tags t ON t.id=et.tag_id WHERE t.name LIKE 'ost: %'")}
    groups={h:ost_tags.get(entries[h]['id'],entries[h]['album']) for h in hashes}
    current=a['current'];ranked={name:comparison.ranks([s[h] for h in hashes]) for name,s in current.items()}
    high={name:mask(i for i,h in enumerate(hashes) if scores[h]>=.75) for name,scores in current.items()}
    top={name:mask(sorted(range(len(hashes)),key=lambda i:-scores[hashes[i]])[:20]) for name,scores in current.items()}
    base_top=0
    for value in top.values():base_top|=value
    weak=mask(i for i,h in enumerate(hashes) if a['currentMax'][h]<.75)
    source={q['name']:q for q in a['qualifiers']+b['qualifiers']}
    if len(source)!=len(a['qualifiers'])+len(b['qualifiers']):
        raise ValueError('Candidate names repeat across studies; choose an explicit version')
    candidates=[];supports={};candidate_tops={}
    semantic_review={'Sacred':'High sea-song matches suggest a vocal/chant proxy; semantic review needed.',
        'Ritualistic':'Top results follow vocal/chant material, including sea songs; semantic review needed.'}
    for name,q in source.items():
        if set(q['scores'])!=set(hashes) or any(set(f['scores'])!=set(hashes) for f in q['forms']):
            raise ValueError('A candidate is not scored across the complete library')
        support=mask(i for i,h in enumerate(hashes) if consistency_support(q['scores'][h],[f['scores'][h] for f in q['forms']]))
        strict=mask(i for i,h in enumerate(hashes) if q['scores'][h]>=.75 and min(f['scores'][h] for f in q['forms'])>=.75)
        supports[name]=support
        candidate_tops[name]=mask(sorted(range(len(hashes)),key=lambda i:-q['scores'][hashes[i]])[:20])
        ranked[name]=comparison.ranks([q['scores'][h] for h in hashes])
        albums=Counter(entries[h]['album'] for i,h in enumerate(hashes) if support>>i&1)
        sources=Counter(groups[h] for i,h in enumerate(hashes) if support>>i&1)
        corrs=[{'name':n,'rho':statistics.correlation(ranked[name],ranked[n]),'jaccard':jaccard(support,high[n])} for n in current]
        closest=max(corrs,key=lambda x:x['rho']);negative=min(corrs,key=lambda x:x['rho'])
        reasons=[]
        if support.bit_count()<10:reasons.append('Fewer than 10 consistently supported tracks')
        if len(albums)<3:reasons.append('Support spans fewer than 3 albums')
        if len(sources)<3:reasons.append('Support spans fewer than 3 game/series or non-OST source groups')
        if q['agreement']<.7:reasons.append('Wording agreement below 0.70')
        if closest['rho']>=.85:reasons.append('Ranking overlap with an existing mood is at least 0.85')
        if albums and max(albums.values())/support.bit_count()>.6:reasons.append('More than 60% of support comes from one album')
        if sources and max(sources.values())/support.bit_count()>.6:reasons.append('More than 60% of support comes from one game/series or source group')
        if name in semantic_review:reasons.append(semantic_review[name])
        candidates.append({'name':name,'kind':q['kind'],'meaning':q['meaning'],'sets':q['sets'],
            'strong':sum(s>=.75 for s in q['scores'].values()),'support':support.bit_count(),'strictSupport':strict.bit_count(),
            'albums':len(albums),'albumCounts':dict(albums),'albumDominance':max(albums.values(),default=0)/max(1,support.bit_count()),
            'sourceGroups':len(sources),'sourceCounts':dict(sources),'supportAfterLargestSourceRemoved':support.bit_count()-max(sources.values(),default=0),
            'sourceDominance':max(sources.values(),default=0)/max(1,support.bit_count()),
            'agreement':q['agreement'],'closestCurrent':closest,'mostNegativeCurrent':negative,'currentOverlaps':corrs,
            'newWeak':(support&weak).bit_count(),'newTop20':(candidate_tops[name]&~base_top).bit_count(),
            'eligible':not reasons,'reasons':reasons,
            'supportIndices':[i for i in range(len(hashes)) if support>>i&1],
            'topIndices':[i for i in range(len(hashes)) if candidate_tops[name]>>i&1],
            'thresholds':{str(level):sum(consistency_support(q['scores'][h],[f['scores'][h] for f in q['forms']],level,min(.5,level)) for h in hashes) for level in [.65,.75,.85]},
            'supportTracks':[entries[h]['name'] for i,h in enumerate(hashes) if support>>i&1]})
    pairs={}
    for x,y in itertools.combinations(ranked,2):
        pairs[tuple(sorted((x,y)))]=statistics.correlation(ranked[x],ranked[y])
    def rho(x,y):return pairs[tuple(sorted((x,y)))]
    eligible=[c for c in candidates if c['eligible']]
    portfolios=[];examined=0
    for kind in ['feel','all']:
        pool=[c for c in eligible if kind=='all' or c['kind']=='feel']
        for k in range(1,4):
            rows=[]
            for combo in itertools.combinations(pool,k):
                examined+=1;names=[c['name'] for c in combo]
                if {'Choral','Vocal'}.issubset(names):continue  # Nested concepts despite imperfect model overlap.
                internal=[rho(x,y) for x,y in itertools.combinations(names,2)]
                # Reject positively redundant additions. Opposite rankings are
                # reported separately rather than called positive overlap.
                if internal and max(internal)>=.8:continue
                support=0;surfaced=0
                for name in names:support|=supports[name];surfaced|=candidate_tops[name]
                correlations=[max(0,rho(name,base)) for name in names for base in current]+[max(0,r) for r in internal]
                rows.append({'kind':kind,'names':names,'k':k,'support':support.bit_count(),'weakCovered':(support&weak).bit_count(),
                    'newTop20':(surfaced&~base_top).bit_count(),'maxOverlap':max(correlations),'meanOverlap':statistics.mean(correlations),
                    'meanInternalOverlap':statistics.mean(internal) if internal else 0})
            # Retain the best overlap trade-off for each coverage/access pair.
            cells={}
            for row in rows:
                key=(row['weakCovered'],row['newTop20'])
                old=cells.get(key)
                if old is None or (row['maxOverlap'],row['meanOverlap'])<(old['maxOverlap'],old['meanOverlap']):cells[key]=row
            frontier=[]
            for row in cells.values():
                if not any(other['weakCovered']>=row['weakCovered'] and other['newTop20']>=row['newTop20'] and other['maxOverlap']<=row['maxOverlap']
                    and (other['weakCovered']>row['weakCovered'] or other['newTop20']>row['newTop20'] or other['maxOverlap']<row['maxOverlap']) for other in cells.values()):
                    frontier.append(row)
            portfolios.extend(sorted(frontier,key=lambda r:(-r['weakCovered'],-r['newTop20'],r['maxOverlap']))[:12])
    current_pairs=[{'a':x,'b':y,'rho':rho(x,y),'jaccard':jaccard(high[x],high[y])} for x,y in itertools.combinations(current,2)]
    candidate_pairs=[{'a':x,'b':y,'rho':rho(x,y),'jaccard':jaccard(supports[x],supports[y])} for x,y in itertools.combinations(source,2)]
    raw_scores={**current,**{name:q['scores'] for name,q in source.items()}}
    holdout_pairs=[('Heroic','Battle'),('Ferocious','Aggressive'),('Ferocious','Defiant'),('Tense','Menacing'),
        ('Ethereal','Mysterious'),('Triumphant','Heroic'),('Reflective','Melancholic'),('Choral','Vocal')]
    holdouts=[]
    for x,y in holdout_pairs:
        values=[]
        for group in sorted(set(groups.values())):
            remaining=[h for h in hashes if groups[h]!=group]
            value=comparison.correlation([raw_scores[x][h] for h in remaining],[raw_scores[y][h] for h in remaining])
            values.append({'removedSource':group,'remaining':len(remaining),'rho':value})
        holdouts.append({'a':x,'b':y,'wholeRho':rho(x,y),'minRho':min(v['rho'] for v in values),'maxRho':max(v['rho'] for v in values),'sources':values})
    base_stats={'meanPositiveRho':statistics.mean(max(0,p['rho']) for p in current_pairs),'maxPositiveRho':max(p['rho'] for p in current_pairs),
        'covered':len(hashes)-weak.bit_count(),'weak':weak.bit_count(),'outsideTop20':len(hashes)-base_top.bit_count()}
    report={'sourceDates':[a['capturedAt'],b['capturedAt']],'tracks':tracks,'libraryCount':len(hashes),'reference':a['reference'],'model':a['model'],
        'currentNames':list(current),'currentPairs':current_pairs,'candidatePairs':candidate_pairs,'baseline':base_stats,
        'weakIndices':[i for i in range(len(hashes)) if weak>>i&1],
        'currentTopIndices':[i for i in range(len(hashes)) if base_top>>i&1],
        'candidates':candidates,'portfolios':portfolios,'examinedPortfolios':examined,'sourceGroups':groups,'sourceHoldouts':holdouts,
        'rules':{'strongThreshold':.75,'formFloor':.5,'requiredForms':2,'minimumSupport':10,'minimumAlbums':3,'maximumAlbumShare':.6,
            'minimumSourceGroups':3,'minimumAgreement':.7,'maximumExistingRho':.85,'maximumInternalRho':.8,'maximumAdditions':3}}
    args.out.write_text(json.dumps(report,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print('Baseline',base_stats,'eligible',len(eligible),'portfolios examined',examined)
    for c in sorted(candidates,key=lambda c:(not c['eligible'],-c['newTop20'],c['closestCurrent']['rho'])):
        print(f"{c['name']:14} {'PASS' if c['eligible'] else 'HOLD':4} support {c['support']:3} strict {c['strictSupport']:3} albums {c['albums']:2} closest {c['closestCurrent']['name']} {c['closestCurrent']['rho']:.2f} newWeak {c['newWeak']} newTop20 {c['newTop20']} | {'; '.join(c['reasons'])}")
    for kind in ['feel','all']:
        print('\nPORTFOLIOS '+kind)
        for row in portfolios:
            if row['kind']==kind:print(row)


if __name__=='__main__':main()
