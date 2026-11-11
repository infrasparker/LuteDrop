import base64
import numpy as np
import pytest
from tagger import adaptive, personal
from tagger.tune import FlagTarget
from test_personal import setup, HEADERS


def inputs():
    # Nearby positive, nearby negative, and acoustically unrelated examples.
    hidden = np.zeros((7,4,1024))
    for i in range(7): hidden[i,:,i] = 1
    hidden[1] = hidden[0] + hidden[1]*.01
    features = np.zeros((7,3))
    anchors = np.random.default_rng(12).normal(size=(100,3))
    return features,hidden,anchors


def test_sparse_correction_honors_feedback_without_moving_unrelated_audio():
    F,H,R = inputs()
    flags = [FlagTarget(0,None,None,1,True,'mid',polarity=1,confidence=4)]
    d = {}
    w,s,_ = adaptive.fit(F,H,flags,[1],2,reference_features=R,reference_geometry=personal.audio_geometry(np.random.default_rng(4).normal(size=(20,4,1024))),diagnostics=d)
    prior = personal.smooth_score(np.column_stack([np.ones(len(F)),F])@w)
    assert d['method']=='local'
    assert s[0]>.9 and s[1]>.9
    assert s[2:]==pytest.approx(prior[2:],abs=1e-7)
    local = personal.LocalCorrection.model_validate(d['local'].model_dump())
    assert s==pytest.approx(personal.smooth_score(np.column_stack([np.ones(len(F)),F])@w+personal.local_latent(local,personal.audio_geometry(H))),abs=1e-12)
    # Neither importing unrelated songs nor batch order changes saved inference.
    scores = personal.local_latent(local,personal.audio_geometry(H[::-1]))
    assert scores[::-1]==pytest.approx(personal.local_latent(local,personal.audio_geometry(H)),abs=1e-12)
    d2 = {}
    _,again,_ = adaptive.fit(np.vstack([F,F]),np.concatenate([H,H]),flags,[1],2,reference_features=R,reference_geometry=personal.audio_geometry(np.random.default_rng(4).normal(size=(20,4,1024))),diagnostics=d2)
    assert again[:len(F)]==pytest.approx(s,abs=1e-12)


def test_unflagged_audio_does_not_change_the_fit_and_no_flags_means_words():
    F,H,R = inputs()
    d = {}
    w,s,_ = adaptive.fit(F,H,[],[1],2,reference_features=R,reference_geometry=personal.audio_geometry(np.random.default_rng(4).normal(size=(20,4,1024))),diagnostics=d)
    assert d['method']=='words' and d['local'] is None
    assert s==pytest.approx(personal.smooth_score(np.column_stack([np.ones(len(F)),F])@w))


def test_comparisons_work_without_flags_and_without_global_membership_shift():
    F,H,R = inputs()
    d = {}
    _,s,_ = adaptive.fit(F,H,[],[1],2,reference_features=R,reference_geometry=personal.audio_geometry(np.random.default_rng(4).normal(size=(20,4,1024))),comparisons=[(0,2)],diagnostics=d)
    assert s[0]>s[2] and d['method']=='local'


def test_nested_exclusion_and_comparisons_do_not_use_the_held_recording():
    F,H,R = inputs()
    flags = [FlagTarget(i,None,None,1,True,'mid',polarity=1 if i<3 else -1,confidence=4) for i in range(6)]
    d = {}
    adaptive.fit(F,H,flags,[1],2,reference_features=R,reference_geometry=personal.audio_geometry(np.random.default_rng(4).normal(size=(20,4,1024))),comparisons=[(0,4)],diagnostics=d)
    errors = []
    for fold in adaptive.validation_folds(flags,list(range(len(flags))),personal.audio_geometry(H)):
        _,s,_ = adaptive.fit(F,H,[g for j,g in enumerate(flags) if j not in fold],[1],2,reference_features=R,reference_geometry=personal.audio_geometry(np.random.default_rng(4).normal(size=(20,4,1024))),comparisons=[] if 0 in fold or 4 in fold else [(0,4)],validate=False)
        for i in fold:
            f=flags[i]
            errors.append(personal.directional_losses([np.tan(np.pi*(s[f.index]-.5))],[f])[0][0])
    assert d['validation_loss']==pytest.approx(np.mean(errors),abs=1e-10)


def test_local_profile_rejects_corrupt_or_nonfinite_portable_exemplars():
    good=dict(width=.05,dimensions=2,coefficients=[1.],vectors=base64.b64encode(np.array([[1.,0.]],dtype='<f4').tobytes()).decode())
    personal.LocalCorrection(**good)
    for values in ([float('nan'),0.],[0.,0.]):
        with pytest.raises(ValueError): personal.LocalCorrection(**{**good,'vectors':base64.b64encode(np.array([values],dtype='<f4').tobytes()).decode()})
    with pytest.raises(ValueError): personal.LocalCorrection(**{**good,'dimensions':3})


@pytest.mark.parametrize('version',[6,7,8])
def test_api_adaptive_preview_matches_frozen_profile_and_preserves_old_profiles(tmp_path,version):
    client,body,_ = setup(tmp_path)
    body['scoring_version']=version
    body['current_lines']=['ominous','- comforting']
    for i,f in enumerate(body['flags']): f.update(polarity=1 if i>2 else -1,confidence=4)
    response=client.post('/tune',json=body,headers=HEADERS)
    assert response.status_code==200,response.text
    proposal=response.json()['proposals'][0]
    assert proposal['profile']['version']==version and proposal['validation_loss'] is not None
    assert proposal['profile']['method'] in ('local','global')
    saved=client.post('/score',json={'targets':body['targets'],'mood_prompts':{'grim':body['current_lines']},'profiles':{'grim':proposal['profile']}},headers=HEADERS)
    assert saved.status_code==200,saved.text
    for f in proposal['flags']:
        assert saved.json()['mood'][f['hash']]['learned']['grim']==pytest.approx(f['after'],abs=1e-12)


@pytest.mark.parametrize('mode',['suggest','arrange'])
@pytest.mark.parametrize('version',[6,7,8])
def test_adaptive_wording_helpers_keep_signed_description_and_score_parity(tmp_path,mode,version):
    client,body,_ = setup(tmp_path)
    body.update(scoring_version=version,mode=mode,current_lines=['ominous','- comforting'],extra_words=['ominous','- comforting'])
    for i,f in enumerate(body['flags']): f.update(polarity=1 if i>2 else -1,confidence=4)
    response=client.post('/tune',json=body,headers=HEADERS)
    assert response.status_code==200,response.text
    for proposal in response.json()['proposals']:
        scored=client.post('/score',json={'targets':body['targets'],'mood_prompts':{'grim':proposal['lines']},'profiles':{'grim':proposal['profile']}},headers=HEADERS)
        assert scored.status_code==200,scored.text
        for f in proposal['flags']:
            assert scored.json()['mood'][f['hash']]['learned']['grim']==pytest.approx(f['after'],abs=1e-12)


def test_fixed_reference_penalizes_broad_unsupported_corrections():
    F,H,R=inputs()
    flags=[FlagTarget(0,None,None,1,True,'mid',polarity=1,confidence=4)]
    near_reference=np.repeat(personal.audio_geometry(H[:1]),100,axis=0)
    far_reference=personal.audio_geometry(np.random.default_rng(4).normal(size=(100,4,1024)))
    _,near,_=adaptive.fit(F,H,flags,[1],2,reference_features=R,reference_geometry=near_reference)
    _,far,_=adaptive.fit(F,H,flags,[1],2,reference_features=R,reference_geometry=far_reference)
    assert near[0]<far[0]-.2


def test_negative_descriptions_only_penalize_matching_evidence_and_keep_legacy_scores():
    raw=np.array([[.2,-2.],[.2,0.],[.2,2.]])
    old=personal.word_features(raw,[0,0],[1,1],[1,-1])
    new=personal.word_features(raw,[0,0],[1,1],[1,-1],penalty_only=True)
    assert old[:,1]==pytest.approx([-2,0,2])
    assert new[:,1]==pytest.approx([0,0,2])
    assert new@np.array([1.,-1.])==pytest.approx([.2,.2,-1.8])
    class Evidence: id='test'
    kwargs=dict(id='frozen',evidence='test',createdAt='now',trainingCount=0,bias=-.5,
        audioWeights=[0.],wordAxes=[dict(label='positive',prompts=['funky'],mean=0,sd=1,weight=1),
        dict(label='negative',prompts=['sad'],mean=0,sd=1,weight=-1)],method='words')
    legacy=personal.Profile(version=6,**kwargs)
    corrected=personal.Profile(version=7,**kwargs)
    before=personal.profile_scores(legacy,Evidence(),np.zeros((3,1)),raw)
    after=personal.profile_scores(corrected,Evidence(),np.zeros((3,1)),raw)
    frozen=personal.Profile(version=8,wordVersion=6,**kwargs)
    assert personal.profile_scores(frozen,Evidence(),np.zeros((3,1)),raw)==pytest.approx(before,abs=1e-12)
    assert before==pytest.approx(personal.smooth_score(np.array([1.7,-.3,-2.3])))
    assert after[0]==pytest.approx(after[1])
    assert after[2]<after[1]


def test_feedback_refinement_keeps_foundation_and_unrelated_placements():
    _,hidden,_ = inputs()
    geometry = personal.audio_geometry(hidden)
    reference = personal.audio_geometry(np.random.default_rng(4).normal(size=(20,4,1024)))
    prior = np.full(len(hidden),np.tan(np.pi*(.85-.5)))
    no = FlagTarget(0,None,None,1,True,'mid',polarity=-1,confidence=4)
    correction,values = adaptive.refine(prior,geometry,[no],reference)
    assert values[0]<.1 and values[1]<.1  # acoustically near duplicate
    assert values[2:]==pytest.approx(np.full(5,.85),abs=1e-7)
    assert values==pytest.approx(personal.smooth_score(prior+personal.local_latent(correction,geometry)),abs=1e-12)
    _,again = adaptive.refine(prior,geometry,[no],reference)
    assert again==pytest.approx(values,abs=1e-12)
    _,imported = adaptive.refine(np.r_[prior,prior],np.vstack([geometry,geometry]),[no],reference)
    assert imported[:len(prior)]==pytest.approx(values,abs=1e-12)
    # Removing feedback restores the immutable foundation, without undo stacks.
    removed,restored = adaptive.refine(prior,geometry,[],reference)
    assert removed is None and restored==pytest.approx(np.full(7,.85))
    _,ordered = adaptive.refine(prior,geometry,[],reference,[(2,3)])
    assert ordered[2]>ordered[3]


@pytest.mark.parametrize('version',[6,7,8])
@pytest.mark.parametrize('method',['global','local'])
def test_same_wording_refines_frozen_global_model_and_repeated_apply_does_not_stack(tmp_path,version,method):
    client,body,features = setup(tmp_path)
    # Audio-independent global foundation; unrelated clips must retain .85.
    for i in range(7):
        hidden=np.zeros((4,1024));hidden[:,i]=1
        features.put(f'h{i}',hidden)
    foundation=dict(version=version,id='accepted',evidence='test-evidence',createdAt='now',
        trainingCount=0,bias=float(np.tan(np.pi*(.85-.5))),audioWeights=[0.]*8,
        wordAxes=[dict(label='ominous',prompts=['ominous'],mean=0,sd=1,weight=0)],method='global')
    if method=='local':
        foundation.update(method='local',local=dict(width=.1,dimensions=4096,coefficients=[.5],
            vectors=base64.b64encode(personal.audio_geometry(np.array([features.get('h5')])).astype('<f4').tobytes()).decode()))
    old=client.post('/score',json=dict(targets=body['targets'],mood_prompts={'grim':['ominous']},profiles={'grim':foundation}),headers=HEADERS).json()['mood']
    body.update(scoring_version=8,active_profile=foundation,
        flags=[dict(hash='h0',side='low',polarity=-1,confidence=4)])
    first=client.post('/tune',json=body,headers=HEADERS)
    assert first.status_code==200,first.text
    p=first.json()['proposals'][0];profile=p['profile']
    assert profile['method']==method and profile['adjustment'] is not None
    assert profile['local']==foundation.get('local')
    assert profile['wordVersion']==version and profile['bias']==foundation['bias']
    assert profile['audioWeights']==foundation['audioWeights']
    assert p['validation_loss'] is None  # foundation was not independently held out
    score_body=dict(targets=body['targets'],mood_prompts={'grim':['ominous']},profiles={'grim':profile})
    scored=client.post('/score',json=score_body,headers=HEADERS)
    assert scored.status_code==200,scored.text
    assert scored.json()['mood']['h0']['learned']['grim']==pytest.approx(p['flags'][0]['after'],abs=1e-12)
    assert scored.json()['mood']['h5']['learned']['grim']==pytest.approx(old['h5']['learned']['grim'],abs=1e-7)
    body['active_profile']=profile
    again=client.post('/tune',json=body,headers=HEADERS).json()['proposals'][0]
    assert again['profile']['id']==profile['id']
    assert again['flags'][0]['after']==pytest.approx(p['flags'][0]['after'],abs=1e-12)
    # Flag removal really removes the residual; it cannot undo the foundation.
    body['flags']=[]
    removed=client.post('/tune',json=body,headers=HEADERS).json()['proposals'][0]['profile']
    assert removed['adjustment'] is None and removed['bias']==foundation['bias']
    # Only an explicit rebuild or a changed description can replace the model.
    body.update(rebuild=True)
    rebuilt=client.post('/tune',json=body,headers=HEADERS).json()['proposals'][0]['profile']
    assert rebuilt['adjustment'] is None and rebuilt['bias']!=foundation['bias']
    body.update(rebuild=False,extra_words=['eerie'])
    edited=client.post('/tune',json=body,headers=HEADERS).json()['proposals'][0]['profile']
    assert edited['adjustment'] is None and edited['wordAxes'][0]['prompts']==['eerie']
