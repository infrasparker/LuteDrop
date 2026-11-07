// Unit tests for the pure helpers in src/utils/moodFlags.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { flagControlVisible, flagLabel } from '../src/utils/moodFlags.ts';

test('flagLabel reads Devin\'s words', () => {
  assert.equal(flagLabel({ kind: 'band', band: 'understandable', bound: 'at_least', stale: false }), 'At least Understandable');
  assert.equal(flagLabel({ kind: 'band', band: 'stretch', bound: 'in', stale: false }), 'Stretch');
  assert.equal(flagLabel({ kind: 'higher', band: null, bound: 'in', stale: true }), 'Higher · made under older wording');
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
