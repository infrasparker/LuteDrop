"""Saved-model guarantees: batch changes never recalibrate existing scores."""
import importlib.util
from pathlib import Path
import numpy as np
import pytest

spec = importlib.util.spec_from_file_location('fusion', Path(__file__).resolve().parents[1] / 'scripts/score-horror-fusion-candidate.py')
fusion = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fusion)


def fixture():
    profile = {'heads': [{'members': [{'layer': 3, 'coefficient': 'head', 'intercept': 0., 'calibration': [1., 0.]}]}],
               'textAxes': [{'reference': {'mean': .1, 'sd': .2}}], 'coefficients': [.1, .4, -.2]}
    w = np.zeros(1024); w[0] = .2
    hidden = np.zeros((2, 1, 1024)); hidden[0, 0, 0] = 1; hidden[1, 0, 1] = 1
    return profile, {'head': w}, hidden, np.array([[.1], [.2]])


def test_adding_new_music_and_batch_order_cannot_change_existing_fit():
    p, w, h, t = fixture()
    expected = fusion.predict(p, w, h, [3], t)
    new_h = np.ones((1, 1, 1024))
    enlarged = fusion.predict(p, w, np.concatenate([h, new_h]), [3], np.concatenate([t, [[.9]]]))
    np.testing.assert_allclose(enlarged[:2], expected, rtol=0, atol=1e-15)
    np.testing.assert_allclose(fusion.predict(p, w, h[::-1], [3], t[::-1]), expected[::-1], rtol=0, atol=1e-15)


def test_feedback_is_provenance_not_an_inference_input():
    p, w, h, t = fixture()
    before = fusion.predict(p, w, h, [3], t)
    p['trainingJudgments'] = [{'hash': 'anything', 'min': 1., 'max': None}]
    after = fusion.predict(p, w, h, [3], t)
    np.testing.assert_array_equal(before, after)


def test_unknown_layer_and_corrupt_inputs_are_rejected():
    p, w, h, t = fixture()
    with pytest.raises(ValueError, match='layer missing'): fusion.predict(p, w, h, [6], t)
    with pytest.raises(ValueError, match='Empty'): fusion.predict(p, w, np.zeros_like(h), [3], t)
    t[0, 0] = np.nan
    with pytest.raises(ValueError, match='finite'): fusion.predict(p, w, h, [3], t)
