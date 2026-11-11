import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mapPointAction } from '../src/utils/mapPointer.ts';
test('Shift-right-click removes only an existing seed; ordinary right-click still deselects', () => {
  assert.equal(mapPointAction(2, true, true), 'seed');
  assert.equal(mapPointAction(2, true, false), 'deselect');
  assert.equal(mapPointAction(2, false, true), 'deselect');
  assert.equal(mapPointAction(0, true, true), 'seed');
  assert.equal(mapPointAction(0, true, false), 'seed');
  assert.equal(mapPointAction(0, false, true), 'play');
});
