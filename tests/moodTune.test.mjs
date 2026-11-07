// Unit tests for the pure view-model helpers in src/utils/moodTune.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  acceptLabel,
  albumOf,
  brokenFresh,
  flaggedEntries,
  holdingEntryIds,
  isChangedSinceTune,
  looNote,
  nameNote,
  orderedFlags,
  overlapRises,
  parseExtraWords,
  shakyNote,
  splitName,
  topChanges,
} from '../src/utils/moodTune.ts';

const song = (entryId) => ({ entryId, name: entryId, album: '' });

test('accept label counts only fresh broken flags', () => {
  const r = { flags: [{ holds: false, fresh: true }, { holds: false, fresh: false }, { holds: true, fresh: true }] };
  assert.equal(brokenFresh(r), 1);
  assert.equal(acceptLabel(r), 'Accept anyway: 1 flagged song leaves their band');
});
test('accept label is plain Accept when no fresh flag breaks, and counts plurals', () => {
  assert.equal(acceptLabel({ flags: [{ holds: false, fresh: false }] }), 'Accept');
  const two = { flags: [{ holds: false, fresh: true }, { holds: false, fresh: true }] };
  assert.equal(acceptLabel(two), 'Accept anyway: 2 flagged songs leave their band');
});
test('shaky note under five flags', () => { assert.equal(shakyNote(3), 'Only 3 flags: the result may be shaky'); assert.equal(shakyNote(5), null); });
test('shaky note says flag for one', () => assert.equal(shakyNote(1), 'Only 1 flag: the result may be shaky'));
test('name note only when the name is not the best word', () => {
  assert.equal(nameNote('Sad', [{ word: 'sad', rho: 0.9 }]), null);
  assert.equal(nameNote('Wondrous', [{ word: 'reverent', rho: 0.77 }]), 'These words now read more like reverent than Wondrous');
  assert.equal(nameNote('Sad', []), null);
});

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

test('ordered flags put broken fresh first, then broken stale, then holding', () => {
  const r = {
    flags: [
      { song: song('ok'), holds: true, fresh: true },
      { song: song('stale'), holds: false, fresh: false },
      { song: song('fresh'), holds: false, fresh: true },
    ],
  };
  assert.deepEqual(orderedFlags(r).map((f) => f.song.entryId), ['fresh', 'stale', 'ok']);
});

test('leave-one-out note is skipped for the baseline (0 of 0)', () => {
  assert.equal(looNote({ held: 0, total: 0 }), null);
  assert.equal(looNote({ held: 4, total: 6 }), '4 of 6 flags hold even when not used for tuning');
});

test('extra words split on commas and new lines, trimmed and de-duplicated', () => {
  assert.deepEqual(parseExtraWords(' grief, funeral\ncello,, grief '), ['grief', 'funeral', 'cello']);
  assert.deepEqual(parseExtraWords('  '), []);
});

test('overlap rises of at least 0.1 are reported', () => {
  const rises = overlapRises([
    { moodId: 'a', before: 0.3, after: 0.45 },
    { moodId: 'b', before: 0.3, after: 0.35 },
    { moodId: 'c', before: 0.5, after: 0.6 },
  ]);
  assert.deepEqual(rises.map((o) => o.moodId), ['a', 'c']);
});

test('top changes name what joined and what dropped out', () => {
  const r = {
    top: [{ song: song('a'), bar: 0.9 }, { song: song('n'), bar: 0.8 }],
    topBefore: [{ song: song('a'), bar: 0.9 }, { song: song('d'), bar: 0.85 }],
  };
  const { joined, dropped } = topChanges(r);
  assert.deepEqual([...joined], ['n']);
  assert.deepEqual([...dropped], ['d']);
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

test('changed since Tune ran is recognised in an accept error', () => {
  assert.equal(isChangedSinceTune('The lines of Sad changed since Tune ran; run Tune again'), true);
  assert.equal(isChangedSinceTune('Database is locked'), false);
});
