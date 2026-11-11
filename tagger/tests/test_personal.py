from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import numpy as np
import pytest
from fastapi.testclient import TestClient
from scipy.special import expit

from tagger import personal
from tagger.app import create_app
from tagger.embed import EmbeddingCache
from tagger.reference import ReferenceSet

HEADERS = {'X-LuteDrop-Token': 'test'}


def test_fixed_reference_stops_one_positive_from_turning_everything_positive():
    from tagger.tune import FlagTarget
    features=np.zeros((100,2));anchors=np.zeros((300,2))
    flag=FlagTarget(0,None,None,1.,True,'high',polarity=1,confidence=4.)
    weights,scores,_=personal.fit_directional(features,[flag],[1],1,reference_features=anchors)
    assert np.max(scores)<.85,'One selected positive is not evidence that all music belongs'
    more=np.vstack([features,np.array([[10.,10.]])])
    again,_,_=personal.fit_directional(more,[flag],[1],1,reference_features=anchors)
    assert weights==pytest.approx(again,abs=1e-12),'Importing unjudged songs cannot change the fixed anchor'
    baseline,unchanged,_=personal.fit_directional(features,[],[1],1,reference_features=anchors)
    assert unchanged==pytest.approx(personal.smooth_score(np.column_stack([np.ones(100),features])@baseline))


def test_reference_fit_rejects_missing_anchors_instead_of_falling_back(tmp_path):
    client,body,features=setup(tmp_path)
    features.evidence.reference_audio=None
    body['scoring_version']=5
    response=client.post('/tune',json=body,headers=HEADERS)
    assert response.status_code==409 and 'reference audio anchors' in response.text


def test_directional_feedback_rewards_extremes_instead_of_point_targets():
    from tagger.tune import FlagTarget
    yes = FlagTarget(0,None,None,1,True,'high',polarity=1,confidence=4)
    no = FlagTarget(0,None,None,1,True,'low',polarity=-1,confidence=4)
    latent = lambda p: np.tan(np.pi*(p-.5))
    loss = lambda p,f: personal.directional_losses([latent(p)],[f])[0][0]
    assert loss(.99,yes) < loss(.95,yes) < loss(.9,yes) < loss(.75,yes)
    assert loss(.01,no) < loss(.1,no) < loss(.3,no) < loss(.5,no)
    assert loss(.75,yes) == pytest.approx(loss(.25,no))
    z = np.array([-.7,.8]); flags = [yes,no]
    _, gradient = personal.directional_losses(z,flags)
    epsilon = 1e-6
    numeric = (personal.directional_losses(z+epsilon,flags)[0]-personal.directional_losses(z-epsilon,flags)[0])/(2*epsilon)
    assert gradient == pytest.approx(numeric,abs=1e-8)
    # A mood with only historical limits still reports the production loss.
    from tagger import tune
    cap = FlagTarget(0,None,.9,1.,True,'low',feedback_version=4)
    expected = personal.directional_losses([latent(.7)],[cap])[0][0]
    assert tune.loss(np.array([.7]),[cap]) == pytest.approx(expected)
    assert expected > 0 and tune.broken(np.array([.95]),[cap]) == []


def test_directional_fit_uses_same_semantics_for_each_mood_and_keeps_limits():
    from tagger.tune import FlagTarget
    features = np.array([[-2.,0.],[-1.,0.],[1.,0.],[2.,0.]])
    flags = [FlagTarget(i,None,None,1,True,'mid',polarity=1 if i>1 else -1,confidence=4) for i in range(4)]
    weights,scores,_ = personal.fit_directional(features,flags,[1],1,validate=False)
    assert scores[0]<.1 and scores[3]>.9
    _,again,_ = personal.fit_directional(features,flags,[1],1,validate=False)
    assert scores == pytest.approx(again,abs=1e-12)
    limit = FlagTarget(0,None,.9,1,True,'low')
    below = personal.directional_losses([np.tan(np.pi*(.7-.5))],[limit])[0][0]
    above = personal.directional_losses([np.tan(np.pi*(.95-.5))],[limit])[0][0]
    assert below < above


@pytest.mark.parametrize('version',[4,5])
def test_directional_profile_preview_matches_inference(tmp_path,version):
    client,body,_ = setup(tmp_path)
    body['scoring_version'] = version
    for i,f in enumerate(body['flags']): f.update(polarity=1 if i>2 else -1,confidence=4,pivot=.5)
    response = client.post('/tune',json=body,headers=HEADERS)
    assert response.status_code == 200,response.text
    proposal = response.json()['proposals'][0]
    assert proposal['profile']['version'] == version
    assert proposal['validation_loss'] is not None and proposal['validation_mae'] is None
    scored = client.post('/score',json={'targets':body['targets'],'mood_prompts':{'grim':body['current_lines']},'profiles':{'grim':proposal['profile']}},headers=HEADERS).json()['mood']
    for row in proposal['flags']:
        assert scored[row['hash']]['learned']['grim'] == pytest.approx(row['after'],abs=1e-12)


def test_directional_exclusion_does_not_leak_comparisons():
    from tagger.tune import FlagTarget
    features = np.random.default_rng(3).normal(size=(5,3))
    flags = [FlagTarget(i,None,None,1.,True,'mid',polarity=1 if i>1 else -1,confidence=4.) for i in range(5)]
    diagnostics = {}
    personal.fit_directional(features,flags,[1],2,comparisons=[(4,0)],diagnostics=diagnostics)
    errors = []
    for i,f in enumerate(flags):
        remaining = [g for j,g in enumerate(flags) if i!=j]
        pairs = [(4,0)] if i not in (4,0) else []
        _,scores,_ = personal.fit_directional(features,remaining,[1],2,comparisons=pairs)
        latent = np.tan(np.pi*(scores[i]-.5))
        errors.append(personal.directional_losses([latent],[f])[0][0])
    assert diagnostics['validation_loss'] == pytest.approx(np.mean(errors),abs=1e-10)
    _,scores,_ = personal.fit_directional(features,[],[1],2,comparisons=[(4,0)])
    assert scores[4]>scores[0]


def test_ratings_have_no_dead_zone_and_comparisons_work_without_ratings():
    from tagger.tune import FlagTarget
    features = np.array([[-1.,0.], [0.,0.], [1.,0.]])
    no = FlagTarget(1,None,.5,1.,True,'low',target=.1)
    _, values, _ = personal.fit_general(features,[no],[1],1,validate=False)
    assert values[1] < .2, 'No must continue pulling below the old .5 ceiling'
    _, ordered, _ = personal.fit_general(features,[],[1],1,validate=False,comparisons=[(2,0)])
    assert ordered[2] > ordered[0] + .1
    _, reversed_, _ = personal.fit_general(features,[],[1],1,validate=False,comparisons=[(0,2)])
    assert reversed_[0] > reversed_[2] + .1


def test_exclusion_removes_comparisons_touching_held_song():
    from tagger.tune import FlagTarget
    rng = np.random.default_rng(1)
    features = rng.normal(size=(5,3))
    flags = [FlagTarget(i,None,None,1.,True,'mid',target=.1+.2*i) for i in range(5)]
    diagnostics = {}
    personal.fit_general(features,flags,[1],2,comparisons=[(4,0)],diagnostics=diagnostics)
    errors = []
    for i,f in enumerate(flags):
        remaining = [g for j,g in enumerate(flags) if i != j]
        pairs = [(4,0)] if i not in (4,0) else []
        _, scores, _ = personal.fit_general(features,remaining,[1],2,comparisons=pairs)
        errors.append(abs(scores[i]-f.target))
    assert diagnostics['validation_mae'] == pytest.approx(np.mean(errors), abs=1e-10)


class Embedder:
    name = 'fake'
    def load(self): pass
    def embed_text(self, texts):
        rows = []
        for text in texts:
            seed = sum((i+1)*ord(c) for i,c in enumerate(text))
            v = np.random.default_rng(seed).normal(size=8)
            rows.append(v / np.linalg.norm(v))
        return np.array(rows)


class Evidence:
    id = 'test-evidence'
    metadata = {'modelRevision': 'fixed'}
    reference_set = 'fixed-ref'
    reference_audio = np.random.default_rng(31).uniform(size=(100,8))
    reference_geometry = personal.audio_geometry(np.random.default_rng(31).normal(size=(100,4,1024)))
    def features(self, hidden):
        return np.column_stack([expit(hidden[:, 0, 0] * (i+1)/4) for i in range(8)])


def setup(tmp_path):
    evidence = Evidence(); cache = EmbeddingCache(tmp_path)
    features = personal.FeatureCache(tmp_path, evidence)
    rng = np.random.default_rng(42)
    reference = rng.normal(size=(100, 8)); reference /= np.linalg.norm(reference, axis=1, keepdims=True)
    for i in range(7):
        v = rng.normal(size=(2, 8)); cache.put(f'h{i}', v)
        H = np.ones((4, 1024)); H[0, 0] = i-3
        features.put(f'h{i}', H)
    app = create_app(Embedder(), cache, 'test', executor_factory=lambda: ThreadPoolExecutor(1),
                     reference=ReferenceSet('fixed-ref', 'fake', reference), evidence=evidence)
    client = TestClient(app)
    body = {'mood_id':'grim', 'name':'Horror', 'current_lines':['ominous'], 'mode':'learn',
            'targets':[f'h{i}' for i in range(6)],
            'flags':[{'hash':f'h{i}', 'min':.75 if i>2 else None, 'max':None if i>2 else .5, 'side':'high' if i>2 else 'low'} for i in range(6)],
            'calibration':{'source':'reference', 'midpoint_z':.674, 'slope':1.8}}
    return client, body, features


def test_learn_review_matches_saved_inference_and_imports_do_not_shift_it(tmp_path):
    client, body, features = setup(tmp_path)
    response = client.post('/tune', json=body, headers=HEADERS)
    assert response.status_code == 200, response.text
    result = response.json()['proposals'][0]; profile = result['profile']
    assert profile['trainingCount'] == 6 and result['loo']['total'] == 6
    assert profile['version'] == 2 and len(profile['wordAxes']) == 1 and len(profile['audioWeights']) == 8
    score = {'targets': body['targets'], 'mood_prompts':{'grim':['ominous']}, 'profiles':{'grim':profile}}
    first = client.post('/score', json=score, headers=HEADERS)
    assert first.status_code == 200, first.text
    values = {h: x['learned']['grim'] for h,x in first.json()['mood'].items()}
    for row in result['flags']:
        assert values[row['hash']] == pytest.approx(row['after'], abs=1e-12)
    score['targets'].append('h6')
    after = client.post('/score', json=score, headers=HEADERS).json()['mood']
    assert all(after[h]['learned']['grim'] == v for h,v in values.items())
    assert all('grim' not in after[h]['moods'] for h in values), 'saved fits cannot fall back to word calibration'
    profile['evidence'] = 'another-version'
    assert client.post('/score', json=score, headers=HEADERS).status_code == 409


def test_strength_preview_and_comparisons_match_frozen_inference(tmp_path):
    client, body, _ = setup(tmp_path)
    body['scoring_version'] = 3
    for i,f in enumerate(body['flags']): f['target'] = .1 if i < 3 else .9
    body['comparisons'] = [['h5','h0']]
    response = client.post('/tune', json=body, headers=HEADERS)
    assert response.status_code == 200, response.text
    result = response.json()['proposals'][0]
    assert result['profile']['version'] == 3
    assert result['loo']['total'] == 0 and result['unsatisfied'] == []
    assert 0 <= result['validation_mae'] <= 1
    scored = client.post('/score', json={'targets':body['targets'], 'mood_prompts':{'grim':body['current_lines']}, 'profiles':{'grim':result['profile']}}, headers=HEADERS).json()['mood']
    for f in result['flags']: assert scored[f['hash']]['learned']['grim'] == pytest.approx(f['after'], abs=1e-12)
    assert result['loss'] == pytest.approx(np.mean([(f['after']-target['target'])**2 for f,target in zip(result['flags'],body['flags'])]))


def test_small_training_sets_work_but_missing_features_are_explicit(tmp_path):
    client, body, features = setup(tmp_path)
    body['flags'] = body['flags'][:4]
    small = client.post('/tune', json=body, headers=HEADERS)
    assert small.status_code == 200 and small.json()['proposals'][0]['loo']['total'] == 0
    features.path('h0').unlink()
    response = client.post('/tune', json=body, headers=HEADERS)
    assert response.status_code == 409 and 'preparing' in response.json()['detail']


def test_one_sided_examples_are_regularized_and_available_for_any_mood(tmp_path):
    client, body, _ = setup(tmp_path)
    for flag in body['flags']:
        flag.update(min=.75, max=None, side='high')
    response = client.post('/tune', json=body, headers=HEADERS)
    assert response.status_code == 200
    body['mood_id'] = 'festive'
    body['current_lines'] = ['bright, festive, celebratory', '- ominous, threatening']
    result = client.post('/tune', json=body, headers=HEADERS)
    assert result.status_code == 200
    axes = result.json()['proposals'][0]['profile']['wordAxes']
    assert len(axes) == 2 and axes[0]['weight'] >= 0 and axes[1]['weight'] <= 0


def test_reference_bundle_features_are_finite_and_cache_is_versioned(tmp_path):
    root = Path(__file__).resolve().parents[1] / 'tagger/reference'
    evidence = personal.Evidence.load(root)
    hidden = np.random.default_rng(0).normal(size=(2,4,1024))
    values = evidence.features(hidden)
    assert values.shape == (2,19) and np.isfinite(values).all()
    assert ((values >= 0) & (values <= 1)).all()
    cache = personal.FeatureCache(tmp_path, evidence)
    cache.put('one', hidden[0]); assert cache.get('one') is not None
    evidence.metadata = dict(evidence.metadata, modelRevision='new-weights')
    assert cache.get('one') is None
    with pytest.raises(ValueError): cache.put('../escape', hidden[0])


def test_smooth_mapping_preserves_high_end_ranking_without_clipping():
    values = personal.smooth_score(np.array([-100., -10., 0., 10., 100., 1000.]))
    assert np.all(np.diff(values) > 0)
    assert np.all((values > 0) & (values < 1))


def test_description_only_preview_has_no_training_and_uses_edited_lines(tmp_path):
    client, body, _ = setup(tmp_path)
    body['flags'] = []
    body['extra_words'] = ['joyful, celebratory', '- dark, threatening']
    response = client.post('/tune', json=body, headers=HEADERS)
    assert response.status_code == 200, response.text
    p = response.json()['proposals'][0]
    assert p['lines'] == body['extra_words']
    assert p['profile']['trainingCount'] == 0
    assert not any(p['profile']['audioWeights'])
    assert p['profile']['wordAxes'][0]['prompts'] == ['joyful, celebratory']
    body['extra_words'] = ['- dark']
    assert client.post('/tune', json=body, headers=HEADERS).status_code == 422


@pytest.mark.parametrize('mode', ['arrange', 'suggest'])
def test_wording_helpers_use_the_same_signed_model_as_apply(tmp_path, monkeypatch, mode):
    from tagger import tune
    def legacy_search(*args, **kwargs):
        raise AssertionError('A signed wording proposal must not use the positive-only surrogate')
    monkeypatch.setattr(tune, 'beam_search', legacy_search)
    client, body, _ = setup(tmp_path)
    original = ['ominous', '- reassuring']
    body.update(mode=mode, scoring_version=2, current_lines=original[:], extra_words=original[:], bank=['calming'])
    response = client.post('/tune', json=body, headers=HEADERS)
    assert response.status_code == 200, response.text
    proposal = response.json()['proposals'][0]
    assert body['current_lines'] == original
    assert proposal['profile']['version'] == 2
    saved = client.post('/score', json={'targets':body['targets'], 'mood_prompts':{'grim':proposal['lines']}, 'profiles':{'grim':proposal['profile']}}, headers=HEADERS)
    for flag in proposal['flags']:
        assert saved.json()['mood'][flag['hash']]['learned']['grim'] == pytest.approx(flag['after'], abs=1e-12)
