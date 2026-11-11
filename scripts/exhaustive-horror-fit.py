"""Controlled read-only test of prompt pruning versus a complete bounded search.

Select a diverse dictionary from deterministic fear-related templates, preserving
specialists for every flag. Recover exact reference first/second moments from
single/pair /score calls. Enumerate every equal-weight subset up to six lines.
All finalists are independently checked by the live analyzer and original flags.
The covariance factor is a numerical representation of reference moments, not
new reference tracks. No calibration, flag or vocabulary is written to the app.
"""
import argparse
from contextlib import closing
from datetime import datetime, timezone
import importlib.util
import itertools
import json
import math
import os
from pathlib import Path
import sqlite3
import sys
import time

import numpy as np
from scipy.linalg import helmert

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tagger'))
from tagger import tune
spec=importlib.util.spec_from_file_location('fit',ROOT/'scripts/fit-horror-flags.py')
fit=importlib.util.module_from_spec(spec);spec.loader.exec_module(fit);r=fit.r


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--out',type=Path,required=True);p.add_argument('--units',type=int,default=30);p.add_argument('--contexts',action='store_true');p.add_argument('--graveyard',action='store_true');p.add_argument('--fit-calibration',action='store_true');a=p.parse_args()
    db=Path(os.environ['APPDATA'])/'com.lutedrop/lutedrop.db'
    with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True)) as live, closing(sqlite3.connect(':memory:')) as conn:
        live.backup(conn);meta=r.export.meta(conn);vocab=r.export.vocabulary(meta);mood=next(m for m in vocab['moods'] if m['id']=='grim')
        entries=[dict(zip(['id','name','hash','path'],row)) for row in conn.execute("SELECT id,name,content_hash,file_path FROM library_entries WHERE entry_type='music' ORDER BY content_hash")]
        tags=dict(conn.execute("SELECT et.library_entry_id,t.name FROM library_entry_tags et JOIN tags t ON t.id=et.tag_id WHERE t.name LIKE 'ost: %'"))
        for e in entries:e['album']=r.export.album_of(e.pop('path'));e['source']=tags.get(e['id'],e['album'])
        hashes=[e['hash'] for e in entries];byhash={e['hash']:e for e in entries}
        request,skipped,_=r.export.build_request(conn,meta,'grim','compare',[],[],[mood['prompts']],focus='')
        if request['calibration']['source']!='reference':raise ValueError('This study requires frozen reference calibration')
        flags=request['flags'];idx=np.array([hashes.index(f['hash']) for f in flags]);flag_names=[byhash[f['hash']]['name'] for f in flags]
        lo=np.array([np.nan if f['min'] is None else f['min'] for f in flags]);hi=np.array([np.nan if f['max'] is None else f['max'] for f in flags]);weights=np.array([f['weight'] for f in flags])
        report={'capturedAt':datetime.now(timezone.utc).isoformat(),'vocabulary':vocab,'entries':entries,'flags':[dict(f,song=name) for f,name in zip(flags,flag_names)],'calls':[]}
        def save():a.out.write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
        def post(endpoint,body):
            start=time.monotonic();ans=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],endpoint,body,180)
            report['calls'].append({'endpoint':endpoint,'groups':len(body.get('mood_prompts',{})),'seconds':round(time.monotonic()-start,3)})
            return ans
        health=r.export.post(meta['autotag_url'].rstrip('/'),meta['autotag_token'],'/health',None,15);report['model']=health['model']
        baseline=post('/score',{'targets':hashes,'mood_prompts':{'current':mood['prompts']},'energy_prompts':vocab['energy']})
        ref=baseline['mood_reference'];report['reference']={'set':ref['set'],'tracks':ref['tracks']}
        before=np.array([r.spread.calibrated(baseline['mood'][h]['moods']['current'],ref['stats']['current']) for h in hashes])
        def bars(raw,mean,sd):return 1/(1+np.exp(np.clip(-request['calibration']['slope']*((raw-mean)/sd-request['calibration']['midpoint_z']),-700,700)))
        def misses(values,aim=0):
            return np.nan_to_num(np.maximum(0,lo+aim-values))+np.nan_to_num(np.maximum(0,values-(hi-aim)))
        baseline_held=misses(before[idx])<1e-9
        report['baseline']={'held':int(baseline_held.sum()),'scores':dict(zip(hashes,before.tolist()))}
        whole=[line for f in json.loads((ROOT/'scripts/horror-experiment-sets.json').read_text(encoding='utf-8')) if f['name'] in {'Horror literal','Ominous','Dread','Eerie','Unsettling','Sinister','Terrifying','Uncanny','Macabre','Nightmarish','Creepy','Unnerving','Haunted','Spooky','Disturbing','Disquieting','Eldritch'} for line in f['sets']]
        words=fit.WORDS+'haunting desolate bleak abandoned demonic malevolent diabolical infernal occult ritualistic sacrificial wretched alien forbidding malignant brooding'.split()
        whole += ['haunting, poignant, lingering, eerie, evocative music',
            'haunting music with a lingering poignant or unsettling emotional presence',
            'demonic music suggesting an evil and terrifying supernatural presence',
            'a demonic presence', 'infernal, malevolent, ominous choir music',
            'haunted, desolate, abandoned, eerie atmospheric music']
        pool=r.export.dedupe(mood['prompts']+whole+words+[w+' music' for w in words]+[w+' instrumental music' for w in words]+[f'{x}, {y} instrumental music' for x,y in itertools.combinations(words,2)])
        if a.contexts:
            contexts=['ambient music','orchestral music','horror soundtrack','dark ambient soundtrack','haunting melody','dissonant instrumental music','atmospheric game music','drone music','horror ambience']
            narratives=['a haunted house','something evil awakens','a monster approaches','an abandoned asylum','a dark forest at night','a lurking supernatural threat','fear of the unknown','a ghostly presence','the feeling of being watched','entering a forbidding place','unsettling solitude','a desolate landscape','a supernatural mystery','a disturbing discovery']
            pool=r.export.dedupe(pool+[f'{word} {context}' for word,context in itertools.product(words,contexts)]+narratives+[phrase+' music' for phrase in narratives])
        if a.graveyard:
            places=['graveyard','cemetery','crypt','tomb','catacombs','mausoleum','necropolis','burial ground','haunted church']
            grave_words=['sepulchral','funereal','funerary','necromantic','gothic','spectral','ghostly','haunted']
            grave_templates=['a {p}','a haunted {p}','a {p} at night','music for a haunted {p}','a haunted {p} at night',
                '{p} music','haunted {p} music','an eerie {p} soundtrack','a cold and desolate {p}']
            pool=r.export.dedupe(pool+[t.format(p=place) for place,t in itertools.product(places,grave_templates)]+
                [f'{word} {context}' for word,context in itertools.product(grave_words,['music','choir music','haunted music','graveyard atmosphere','horror soundtrack'])]+
                ['a graveyard at night, ghostly, eerie, supernatural dread music','haunted, ghostly, graveyard, ominous instrumental music',
                    'sepulchral music with a ghostly graveyard atmosphere','a haunted cemetery and an unsettling presence of the dead'])
        raw=[];means=[];sds=[]
        for offset in range(0,len(pool),24):
            batch=pool[offset:offset+24];ans=post('/score',{'targets':hashes,'mood_prompts':{r.spread.letter_id(i):[line] for i,line in enumerate(batch)},'energy_prompts':vocab['energy']})
            if ans['mood_reference']['set']!=report['reference']['set']:raise ValueError('Reference changed')
            for i in range(len(batch)):
                key=r.spread.letter_id(i);stat=ans['mood_reference']['stats'][key]
                raw.append([ans['mood'][h]['moods'][key] for h in hashes]);means.append(stat['mean']);sds.append(stat['sd'])
            print('DICTIONARY',min(offset+24,len(pool)),'/',len(pool),flush=True)
        raw=np.array(raw);means=np.array(means);sds=np.array(sds)
        single=bars(raw,means[:,None],sds[:,None]);loose=np.ones(len(hashes),dtype=bool);loose[idx]=False
        high=[i for i,f in enumerate(flags) if f['side']=='high'];low=[i for i,f in enumerate(flags) if f['side']=='low'];pairw=tune.ORDER_PENALTY*np.minimum.outer(weights[high],weights[low])
        def objectives(b,nlines):
            fv=b[:,idx];loss=(misses(fv,tune.AIM_MARGIN)*weights).sum(axis=1)
            if high and low:loss+=((fv[:,high,None]<=fv[:,None,low])*pairw).sum(axis=(1,2))
            stab=np.abs(b[:,loose]-before[loose]).mean(axis=1)*tune.STABILITY_WEIGHT
            stab+=tune.FIRES_WEIGHT*np.abs((b>=.5).sum(axis=1)-(before>=.5).sum())/max(int((before>=.5).sum()),tune.FIRES_FLOOR)
            return loss+stab+tune.LINE_COST*nlines
        solo=objectives(single,1);global_order=np.argsort(solo,kind='stable').tolist()
        specialist=[]
        for pos,f in enumerate(flags):
            vals=single[:,idx[pos]]
            merit=-vals if f['side']=='high' else vals if f['side']=='low' else np.abs(vals-(f['min']+f['max'])/2)
            specialist.append(sorted(range(len(pool)),key=lambda i:(merit[i],solo[i])))
        selected=[];sources={}
        def add(i,why):
            if i not in selected and len(selected)<a.units:selected.append(i)
            if i in selected:sources.setdefault(i,[]).append(why)
        for line in mood['prompts']:add(pool.index(line),'current line')
        for pos,order in enumerate(specialist):add(order[0],'specialist for '+flag_names[pos])
        for i in global_order[:8]:add(i,'global objective')
        for depth in range(1,5):
            for pos,order in enumerate(specialist):add(order[depth],'specialist for '+flag_names[pos])
        for i in global_order:add(i,'global objective')
        units=[pool[i] for i in selected];U=raw[selected];mu=means[selected];sd=sds[selected];n=len(units)
        report['poolCount']=len(pool);report['selected']=[{'line':pool[i],'source':sources[i],'singleObjective':float(solo[i])} for i in selected];save()
        cov=np.diag(sd*sd);pairs=list(itertools.combinations(range(n),2))
        for offset in range(0,len(pairs),24):
            batch=pairs[offset:offset+24]
            ans=post('/score',{'targets':[hashes[0]],'mood_prompts':{r.spread.letter_id(i):[units[x],units[y]] for i,(x,y) in enumerate(batch)},'energy_prompts':vocab['energy']})
            if ans['mood_reference']['set']!=report['reference']['set']:raise ValueError('Reference changed')
            for i,(x,y) in enumerate(batch):
                stat=ans['mood_reference']['stats'][r.spread.letter_id(i)]
                if abs(stat['mean']-(mu[x]+mu[y])/2)>1e-6:raise ValueError('Pair means do not match linear average')
                cov[x,y]=cov[y,x]=2*stat['sd']**2-(sd[x]**2+sd[y]**2)/2
        eigen,vectors=np.linalg.eigh(cov)
        if eigen.min() < -1e-7:raise ValueError('Reference covariance materially non-PSD')
        L=vectors*np.sqrt(np.maximum(eigen,0))[None,:]
        R=mu[:,None]+math.sqrt(n+1)*(L@helmert(n+1,full=False))
        report['numericBasis']={'raw':U.tolist(),'referenceMean':mu.tolist(),'referenceCovariance':cov.tolist()}
        report['covarianceMinEigenvalue']=float(eigen.min())
        current_rows=tuple(units.index(line) for line in mood['prompts'])
        mapping=tune.Mapping(**request['calibration']);localflags=[tune.FlagTarget(int(i),f['min'],f['max'],f['weight'],f['fresh'],f['side'],f['focus']) for i,f in zip(idx,flags)]
        reconstructed=tune.bars(U[list(current_rows)].mean(0),R[list(current_rows)].mean(0),mapping)
        report['baselineReconstructionMaxError']=float(np.max(np.abs(reconstructed-before)))
        if report['baselineReconstructionMaxError']>1e-4:raise ValueError('Reference reconstruction does not reproduce baseline')
        beam=tune.beam_search(U,R,units,localflags,mapping,seeds=[current_rows],before=before)
        report['controlledBeam']=[{'lines':list(c.lines),'objective':c.loss} for c in beam];save()
        best={};coverage_masks={};examined=0;started=time.monotonic()
        lower_positions=np.flatnonzero(~np.isnan(lo));upper_positions=np.flatnonzero(~np.isnan(hi));fullmask=(1<<len(lower_positions))-1
        def logit(v):return np.log(v/(1-v))
        personal_best=None
        kyne=next(i for i,name in enumerate(flag_names) if "Kyne's Peace" in name)
        lead=next(i for i,name in enumerate(flag_names) if 'Lead Your Fights' in name)
        def record(kind,key,rows,objective,held,newbreaks):
            if kind not in best or tuple(key)<tuple(best[kind]['key']):
                best[kind]={'key':[float(x) for x in key],'rows':list(map(int,rows)),'lines':[units[i] for i in rows],
                    'objective':float(objective),'held':int(held),'newBreaks':int(newbreaks)}
        for k in range(1,7):
            iterator=itertools.combinations(range(n),k)
            while batch:=list(itertools.islice(iterator,2048)):
                rows=np.array(batch);rr=U[rows].mean(axis=1);refraw=R[rows].mean(axis=1)
                b=bars(rr,refraw.mean(axis=1)[:,None],refraw.std(axis=1)[:,None]);fv=b[:,idx]
                miss=misses(fv);held=miss<1e-9;count=held.sum(1);newbreaks=(~held & baseline_held).sum(1);violation=miss.sum(1)
                objective=objectives(b,k)
                if a.fit_calibration:
                    # Exact interval feasibility for sigmoid(A*logit(score)+C),
                    # A>0. Every lower/upper pair bounds A; no model is refit.
                    q=logit(np.clip(fv,1e-12,1-1e-12));amin=np.ones(len(batch))*1e-4;amax=np.ones(len(batch))*1e6;valid=np.ones(len(batch),dtype=bool)
                    for lp in lower_positions:
                        for up in upper_positions:
                            diff=q[:,lp]-q[:,up];rhs=float(logit(min(.999,lo[lp]+.01))-logit(max(.001,hi[up]-.01)))
                            positive=diff>1e-12;negative=diff< -1e-12
                            amin=np.maximum(amin,np.where(positive,rhs/np.where(positive,diff,1),-np.inf))
                            amax=np.minimum(amax,np.where(negative,rhs/np.where(negative,diff,1),np.inf))
                            valid &= (positive|negative|(rhs<=0))
                    aa=np.minimum(np.maximum(1.,amin),amax);valid &= (aa>0)&(amin<=amax)
                    cmin=np.max(logit(np.minimum(.999,lo[lower_positions]+.01))[None,:]-aa[:,None]*q[:,lower_positions],axis=1)
                    cmax=np.min(logit(np.maximum(.001,hi[upper_positions]-.01))[None,:]-aa[:,None]*q[:,upper_positions],axis=1)
                    cc=np.clip(np.zeros(len(batch)),cmin,cmax);valid &= cmin<=cmax+1e-8
                    eligible_cal=np.flatnonzero(valid)
                    if len(eligible_cal):
                        severity=np.log(aa[eligible_cal])**2+cc[eligible_cal]**2
                        ii=int(np.argmin(severity));i=int(eligible_cal[ii]);key=float(severity[ii])
                        if personal_best is None or key<personal_best['severity']:
                            calibrated=1/(1+np.exp(np.clip(-(aa[i]*logit(np.clip(b[i],1e-12,1-1e-12))+cc[i]),-700,700)))
                            personal_best={'severity':key,'rows':rows[i].tolist(),'lines':[units[x] for x in rows[i]],'A':float(aa[i]),'C':float(cc[i]),
                                'fixedReferenceFlagsHeld':int(count[i]),'personalFlagsHeld':int((misses(calibrated[idx])<1e-8).sum()),
                                'meanTotalDrift':float(np.abs(calibrated-before).mean()),'flags':[{'song':name,'referenceScore':float(b[i,j]),'personalScore':float(calibrated[j])} for j,name in zip(idx,flag_names)],
                                'referenceScores':dict(zip(hashes,b[i].tolist())),'personalScores':dict(zip(hashes,calibrated.tolist()))}
                i=int(np.argmin(objective));record('App objective',[objective[i]],rows[i],objective[i],count[i],newbreaks[i])
                i=int(np.lexsort((objective,violation,-count,newbreaks))[0]);record('Flag coverage',[newbreaks[i],-count[i],violation[i],objective[i]],rows[i],objective[i],count[i],newbreaks[i])
                negatives=np.flatnonzero((newbreaks==0)&held[:,kyne]&held[:,lead])
                if len(negatives):
                    ii=int(np.lexsort((objective[negatives],violation[negatives],-count[negatives]))[0]);i=int(negatives[ii])
                    record('Kyne and Lead respected',[newbreaks[i],-count[i],violation[i],objective[i]],rows[i],objective[i],count[i],newbreaks[i])
                # Every upper bound must hold in BOTH split heads; every lower
                # bound must hold in at least one. Thus max(headA,headB) must
                # satisfy the ORIGINAL flags, including a middle-band flag.
                eligible=np.flatnonzero(np.all(fv[:,upper_positions]<=hi[upper_positions]+1e-9,axis=1))
                if len(eligible):
                    masks=((fv[eligible][:,lower_positions]>=lo[lower_positions]-1e-9)*(1<<np.arange(len(lower_positions)))).sum(axis=1)
                    for mask in np.unique(masks):
                        if not mask:continue
                        candidates_for_mask=eligible[masks==mask]
                        i=int(candidates_for_mask[np.argmin(objective[candidates_for_mask])])
                        if int(mask) not in coverage_masks or objective[i]<coverage_masks[int(mask)]['objective']:
                            coverage_masks[int(mask)]={'rows':rows[i].tolist(),'lines':[units[x] for x in rows[i]],'objective':float(objective[i]),'mask':int(mask)}
                examined+=len(batch)
            print('EXHAUSTIVE depth',k,'examined',examined,'best',[(key,b['held']) for key,b in best.items()],flush=True)
        report['examined']=examined;report['searchSeconds']=time.monotonic()-started;report['exhaustiveBest']=best;report['personalCalibration']=personal_best
        report['splitCoverageMasks']=[dict(c,covers=[flag_names[p] for bit,p in enumerate(lower_positions) if c['mask']&(1<<bit)]) for c in coverage_masks.values()]
        split_pairs=[]
        for x,y in itertools.combinations(coverage_masks.values(),2):
            if (x['mask']|y['mask'])!=fullmask:continue
            def head(c):
                rr=U[c['rows']].mean(axis=0);refraw=R[c['rows']].mean(axis=0)
                return bars(rr,refraw.mean(),refraw.std())
            bx,by=head(x),head(y);joint=np.maximum(bx,by);pair_rho=tune.spearman(bx,by)
            support=[int((b>=.75).sum()) for b in [bx,by]]
            sources=[len({e['source'] for i,e in enumerate(entries) if b[i]>=.75}) for b in [bx,by]]
            split_pairs.append({'a':x,'b':y,'rho':pair_rho,'support':support,'sourceGroups':sources,
                'jointHeld':int((misses(joint[idx])<1e-9).sum()),'meanJointDrift':float(np.abs(joint-before).mean()),
                'flagScores':[{'song':name,'a':float(bx[i]),'b':float(by[i]),'union':float(joint[i])} for i,name in zip(idx,flag_names)],
                'eligible':pair_rho<.9 and min(support)>=10 and min(sources)>=3})
        split_pairs.sort(key=lambda c:(not c['eligible'],c['meanJointDrift'],c['rho']))
        report['splitPairs']=split_pairs[:10];save()
        print('SPLIT COVERAGE MASKS',len(coverage_masks),'complete pairs',len(split_pairs),'eligible',sum(p['eligible'] for p in split_pairs),flush=True)
        candidates=[];seen=set()
        for source,lines in [(key,value['lines']) for key,value in best.items()]+[('Controlled beam',list(c.lines)) for c in beam]:
            key=json.dumps(lines)
            if key not in seen:seen.add(key);candidates.append({'source':source,'lines':lines})
        if split_pairs:
            for side in ['a','b']:
                lines=split_pairs[0][side]['lines'];key=json.dumps(lines)
                if key not in seen:seen.add(key);candidates.append({'source':'Split head '+side,'lines':lines})
        checked=post('/tune',dict(request,candidates=[c['lines'] for c in candidates]))
        report['validated']=[]
        for candidate,result in zip(candidates,checked['proposals']):
            report['validated'].append({**candidate,'result':result,'held':sum(f['holds'] for f in result['flags']),
                'newBreaks':[byhash[f['hash']]['name'] for f in result['flags'] if baseline_held[list(idx).index(hashes.index(f['hash']))] and not f['holds']],
                'flags':[dict(f,song=byhash[f['hash']]['name']) for f in result['flags']]})
        scored=post('/score',{'targets':hashes,'mood_prompts':{r.spread.letter_id(i):c['lines'] for i,c in enumerate(candidates)},'energy_prompts':vocab['energy']})
        for i,candidate in enumerate(report['validated']):
            key=r.spread.letter_id(i);stat=scored['mood_reference']['stats'][key]
            scores={h:r.spread.calibrated(scored['mood'][h]['moods'][key],stat) for h in hashes};candidate['scores']=scores
            candidate['meanDrift']=float(np.mean(np.abs(np.array([scores[h] for h in hashes])-before)))
            candidate['newHigh']=[byhash[h]['name'] for i,h in enumerate(hashes) if before[i]<.75<=scores[h]]
            candidate['lostHigh']=[byhash[h]['name'] for i,h in enumerate(hashes) if scores[h]<.75<=before[i]]
        if personal_best:
            special=dict(request,candidates=[personal_best['lines']])
            special['calibration']=dict(request['calibration'],slope=request['calibration']['slope']*personal_best['A'],
                midpoint_z=request['calibration']['midpoint_z']-personal_best['C']/(request['calibration']['slope']*personal_best['A']))
            personal_best['validation']=post('/tune',special)['proposals'][0]
        report['completedAt']=datetime.now(timezone.utc).isoformat();save()
        print('DONE',examined,'combinations;',round(report['searchSeconds'],2),'seconds',flush=True)
        for c in report['validated']:
            print('VALIDATED',c['source'],c['held'],'new breaks',c['newBreaks'],c['lines'],flush=True)
            for f in c['flags']:print(' ',f['song'],round(f['after'],3),f['holds'],flush=True)


if __name__=='__main__':main()
