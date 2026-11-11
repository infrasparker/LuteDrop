import test from 'node:test';
import assert from 'node:assert/strict';
import { splitDescription, joinDescription } from '../src/utils/moodDescription.ts';

test('More/Less fields preserve whole phrases and signs through editing', () => {
  const original = 'eerie, haunting\nominous\n- uplifting, triumphant';
  const fields = splitDescription(original);
  assert.deepEqual(fields, { more: 'eerie, haunting\nominous', less: 'uplifting, triumphant' });
  assert.equal(joinDescription(fields.more, fields.less), original);
  const edited = joinDescription(fields.more, 'peaceful, reassuring');
  assert.equal(edited, 'eerie, haunting\nominous\n- peaceful, reassuring');
  assert.equal(fields.less, 'uplifting, triumphant');
});
