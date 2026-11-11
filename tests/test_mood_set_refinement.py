"""Keep ablations faithful to the app's multi-line averaging semantics."""
import importlib.util
from pathlib import Path
import unittest

spec=importlib.util.spec_from_file_location('refine',Path(__file__).parents[1]/'scripts/refine-mood-sets.py')
refine=importlib.util.module_from_spec(spec)
spec.loader.exec_module(refine)


class AblationTest(unittest.TestCase):
    def test_remove_one_subword_preserves_other_lines_and_original_input(self):
        original=['tender, intimate, inspiring','soft flute, sentimental']
        results=dict(refine.ablations(original))
        self.assertEqual(results['Remove subword: intimate'],['tender, inspiring','soft flute, sentimental'])
        self.assertEqual(results['Remove line: soft flute, sentimental'],['tender, intimate, inspiring'])
        self.assertEqual(original,['tender, intimate, inspiring','soft flute, sentimental'])
        self.assertTrue(all(lines and all(line.strip() for line in lines) for lines in results.values()))

    def test_a_single_indivisible_prompt_is_never_deleted_to_make_an_empty_request(self):
        self.assertEqual(list(refine.ablations(['dreamy'])),[])
        self.assertEqual(list(refine.ablations([])),[])


if __name__=='__main__':
    unittest.main()
