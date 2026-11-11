"""Reject a one-phrase spike before using it as library-wide support."""
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('audit',Path(__file__).parents[1]/'scripts/audit-mood-vocabulary.py')
audit=importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class WholeVocabularyTest(unittest.TestCase):
    def test_one_flattering_form_is_not_consistent_support(self):
        self.assertFalse(audit.consistency_support(.96,[.99,.18,.20]))
        self.assertFalse(audit.consistency_support(.96,[.99,.98,.10]))
        self.assertTrue(audit.consistency_support(.86,[.81,.88,.60]))
        self.assertFalse(audit.consistency_support(.7,[.9,.9,.8]))

    def test_shared_membership_overlap_counts_songs_not_scores(self):
        self.assertAlmostEqual(audit.jaccard(audit.mask([0,1,2]),audit.mask([1,2,3])),.5)
        self.assertEqual(audit.jaccard(audit.mask([0,1]),audit.mask([2,3])),0)
        self.assertEqual(audit.jaccard(0,0),0)


if __name__=='__main__':unittest.main()
