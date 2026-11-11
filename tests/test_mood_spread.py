"""Validate that the study compares candidate bars on the app's reference scale."""
from contextlib import redirect_stdout
import importlib.util
import io
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('spread', Path(__file__).parents[1] / 'scripts/mood-spread-study.py')
spread = importlib.util.module_from_spec(spec)
spec.loader.exec_module(spread)


class ReferenceScaleTest(unittest.TestCase):
    def test_same_percentile_landmarks_as_the_app(self):
        stats = {'mean': .1, 'sd': .02}
        self.assertAlmostEqual(spread.calibrated(.1 + .674 * .02, stats), .5)
        self.assertAlmostEqual(spread.calibrated(.1 + 1.2816 * .02, stats), .75)

    def test_affine_raw_score_changes_do_not_favor_a_word(self):
        self.assertAlmostEqual(spread.calibrated(.14, {'mean': .1, 'sd': .02}), spread.calibrated(.78, {'mean': .7, 'sd': .04}))
        self.assertIsNone(spread.calibrated(.1, {'mean': .1, 'sd': 0}))
        self.assertTrue(0 <= spread.calibrated(-1000, {'mean': 0, 'sd': 1}) < .001)
        self.assertTrue(.999 < spread.calibrated(1000, {'mean': 0, 'sd': 1}) <= 1)

    def test_rank_agreement_treats_ties_symmetrically_and_rejects_constant_vectors(self):
        self.assertEqual(spread.comparison.ranks([4, 2, 2, 9]), [2, .5, .5, 3])
        self.assertAlmostEqual(spread.comparison.correlation([1, 1, 2, 3], [2, 2, 4, 6]), 1)
        self.assertIsNone(spread.comparison.correlation([1, 1, 1], [2, 3, 4]))


if __name__ == '__main__':
    unittest.main()
