// Unit tests for the pure helpers in src/utils/moodFlags.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { flagControlVisible, flagLabel, flagMark, reviewTargetLabel } from '../src/utils/moodFlags.ts';

test('flagLabel reads Devin\'s words', () => {
  assert.equal(flagLabel({ kind: 'band', band: 'understandable', bound: 'at_least', stale: false }), 'At least Understandable');
  assert.equal(flagLabel({ kind: 'band', band: 'stretch', bound: 'in', stale: false }), 'Stretch');
  assert.equal(
    flagLabel({ kind: 'higher', band: null, bound: 'in', stale: true, staleReason: 'wording' }),
    'Higher · made under older wording',
  );
});
test('flagLabel names an older calibration apart from older wording', () => {
  assert.equal(
    flagLabel({ kind: 'lower', band: null, bound: 'at_most', stale: true, staleReason: 'calibration' }),
    'Lower · made under an older calibration',
  );
  assert.equal(
    flagLabel({ kind: 'band', band: 'no', bound: 'in', stale: true, staleReason: 'wording' }),
    'No · made under older wording',
  );
});
test('flagMark is the band initial or an arrow', () => {
  const band = (b) => ({ kind: 'band', band: b, bound: 'in' });
  assert.deepEqual(['definitely', 'understandable', 'stretch', 'no'].map((b) => flagMark(band(b))), ['D', 'U', 'S', 'N']);
  assert.equal(flagMark({ kind: 'higher', band: null, bound: 'at_least' }), '↑');
  assert.equal(flagMark({ kind: 'lower', band: null, bound: 'at_most' }), '↓');
});
test('flagLabel covers at_most, lower and definitely', () => {
  assert.equal(flagLabel({ kind: 'band', band: 'stretch', bound: 'at_most', stale: false }), 'At most Stretch');
  assert.equal(flagLabel({ kind: 'lower', band: null, bound: 'in', stale: false }), 'Lower');
  assert.equal(flagLabel({ kind: 'band', band: 'definitely', bound: 'in', stale: false }), 'Definitely');
});
test('flagControlVisible hides for bulk and Energy', () => {
  assert.equal(flagControlVisible(1, 'grim'), true);
  assert.equal(flagControlVisible(3, 'grim'), false);
  assert.equal(flagControlVisible(1, 'energy'), false);
});
test('a review row reads a hand-set level as one, and a removed flag as removed', () => {
  const flag = { kind: 'band', band: 'stretch', bound: 'in', stale: false, staleReason: null };
  assert.equal(reviewTargetLabel(flag, 1), 'Stretch');                 // a flag wins over a level
  assert.equal(reviewTargetLabel(null, 1), 'Hand-set: Definitely');
  assert.equal(reviewTargetLabel(null, 0.75), 'Hand-set: Understandable');
  assert.equal(reviewTargetLabel(null, 0.5), 'Hand-set: Stretch');
  assert.equal(reviewTargetLabel(null, 0.25), 'Hand-set: No');
  assert.equal(reviewTargetLabel(null, 0), 'Hand-set: No');
  assert.equal(reviewTargetLabel(null, null), 'flag removed');
});
