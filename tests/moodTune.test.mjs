// Unit tests for the pure view-model helpers in src/utils/moodTune.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  albumOf,
  flaggedEntries,
  holdingEntryIds,
  overlapRises,
  reviewTargetIsHigh,
  splitName,
  targetIsHigh,
  tuneTargetIds,
  versionProblem,
} from '../src/utils/moodTune.ts';

const song = (entryId) => ({ entryId, name: entryId, album: '' });

test('holding entry ids are the fresh flags that hold', () => {
  const r = {
    flags: [
      { song: song('a'), holds: true, fresh: true },
      { song: song('b'), holds: true, fresh: false },
      { song: song('c'), holds: false, fresh: true },
    ],
  };
  assert.deepEqual(holdingEntryIds(r), ['a']);
});

test('overlap rises of at least 0.1 are reported', () => {
  const rises = overlapRises([
    { moodId: 'a', before: 0.3, after: 0.45 },
    { moodId: 'b', before: 0.3, after: 0.35 },
    { moodId: 'c', before: 0.5, after: 0.6 },
  ]);
  assert.deepEqual(rises.map((o) => o.moodId), ['a', 'c']);
});

test('flagged entries lists the songs with a flag on the mood', () => {
  const flag = { kind: 'band', band: 'no', bound: 'in', barAtFlag: null, calibratedAt: null, stale: false };
  const entries = [
    { id: '1', moods: [{ mood: 'sad', auto: 0.5, user: null, flag }] },
    { id: '2', moods: [{ mood: 'sad', auto: 0.5, user: null, flag: null }] },
    { id: '3', moods: [{ mood: 'calm', auto: 0.5, user: null, flag }] },
  ];
  assert.deepEqual(flaggedEntries(entries, 'sad').map((f) => f.entry.id), ['1']);
});

test('albumOf reads Artist / Album from the two parent folders', () => {
  assert.equal(albumOf('Z:\\Music\\Artist\\Album\\01 Song.flac'), 'Artist / Album');
  assert.equal(albumOf('Z:\\Song.flac'), '');
});

test('split name capitalises the top name suggestion', () => {
  assert.equal(splitName([{ word: 'pastoral', rho: 0.8 }]), 'Pastoral');
  assert.equal(splitName([]), '');
});

test('a flag targets high for Definitely, Understandable, at least, and higher', () => {
  const f = (kind, band, bound) => ({ kind, band, bound });
  assert.equal(targetIsHigh(f('band', 'definitely', 'in')), true);
  assert.equal(targetIsHigh(f('band', 'understandable', 'at_least')), true);
  assert.equal(targetIsHigh(f('band', 'stretch', 'at_least')), true);
  assert.equal(targetIsHigh(f('higher', null, 'in')), true);
  assert.equal(targetIsHigh(f('band', 'stretch', 'in')), false);
  assert.equal(targetIsHigh(f('band', 'stretch', 'at_most')), false);
  assert.equal(targetIsHigh(f('band', 'no', 'in')), false);
  assert.equal(targetIsHigh(f('lower', null, 'in')), false);
});

test('a hand-set level counts as high from 0.75, and a flag decides over a level', () => {
  const flag = { kind: 'band', band: 'stretch', bound: 'in' };
  assert.equal(reviewTargetIsHigh(null, 1), true);
  assert.equal(reviewTargetIsHigh(null, 0.75), true);
  assert.equal(reviewTargetIsHigh(null, 0.5), false);
  assert.equal(reviewTargetIsHigh(null, null), false);
  assert.equal(reviewTargetIsHigh(flag, 1), false);                   // the flag decides
});

test('versionProblem checks versions against the analyzer limits', () => {
  assert.equal(versionProblem(['a\nb', 'c']), null);
  const seventeen = Array.from({ length: 17 }, (_, i) => `line ${i}`).join('\n');
  assert.equal(versionProblem(['ok', seventeen]), 'Version 2 has 17 lines; a version can have at most 16.');
  assert.equal(
    versionProblem([`short\n${'x'.repeat(301)}`]),
    'Version 1 has a line over 300 characters; shorten it.',
  );
  // Blank lines do not count.
  assert.equal(versionProblem([`${'a\n'.repeat(16)}\n\n\n`]), null);
});

test('tune target ids are the songs with a flag or a hand-set level, once each', () => {
  const flag = { kind: 'band', band: 'no', bound: 'in', barAtFlag: null, calibratedAt: null, stale: false };
  const entries = [
    { id: '1', moods: [{ mood: 'sad', auto: 0.5, user: null, flag }] },
    { id: '2', moods: [{ mood: 'sad', auto: 0.5, user: 0.75, flag: null }] },
    { id: '3', moods: [{ mood: 'sad', auto: 0.5, user: 1, flag }] },
    { id: '4', moods: [{ mood: 'sad', auto: 0.5, user: null, flag: null }] },
  ];
  assert.deepEqual([...tuneTargetIds(entries, 'sad')], ['1', '2', '3']);
  assert.equal(tuneTargetIds(entries, 'grim').size, 0);
});
