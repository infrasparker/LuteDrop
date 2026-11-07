// Unit tests for the pure view-model helpers in src/utils/moodTune.ts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  acceptLabel,
  albumOf,
  brokenFresh,
  flaggedEntries,
  flagsLinkLabel,
  handSetCount,
  holdingEntryIds,
  isChangedSinceTune,
  looNote,
  nameNote,
  orderedFlags,
  overlapRises,
  parseExtraWords,
  shakyNote,
  rankLabel,
  reviewTargetIsHigh,
  splitAcceptLabel,
  splitBrokenFresh,
  splitName,
  targetIsHigh,
  topChanges,
  topRanks,
  tuneTargetCount,
  tuneTargetIds,
} from '../src/utils/moodTune.ts';

const song = (entryId) => ({ entryId, name: entryId, album: '' });

test('accept label counts only fresh broken flags', () => {
  const r = { flags: [{ holds: false, fresh: true }, { holds: false, fresh: false }, { holds: true, fresh: true }] };
  assert.equal(brokenFresh(r), 1);
  assert.equal(acceptLabel(r), 'Accept anyway: 1 flagged song leaves its band');
});
test('accept label is plain Accept when no fresh flag breaks, and counts plurals', () => {
  assert.equal(acceptLabel({ flags: [{ holds: false, fresh: false }] }), 'Accept');
  const two = { flags: [{ holds: false, fresh: true }, { holds: false, fresh: true }] };
  assert.equal(acceptLabel(two), 'Accept anyway: 2 flagged songs leave their band');
});
test('shaky note under five flags', () => { assert.equal(shakyNote(3), 'Only 3 flags: the result may be shaky'); assert.equal(shakyNote(5), null); });
test('shaky note says flag for one', () => assert.equal(shakyNote(1), 'Only 1 flag: the result may be shaky'));
test('name note only when the name is not among the top three words', () => {
  assert.equal(nameNote('Sad', [{ word: 'sad', rho: 0.9 }]), null);
  const names = (...words) => words.map((word) => ({ word, rho: 0.5 }));
  assert.equal(nameNote('Sad', names('slow', 'melancholic', 'sad')), null);
  assert.equal(nameNote('Sad', names('slow', 'melancholic', 'dark', 'sad')), 'These words now read more like slow than Sad');
  assert.equal(nameNote('Wondrous', names('reverent')), 'These words now read more like reverent than Wondrous');
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

test('split broken-fresh counts both halves and labels the split accept', () => {
  const half = (flags) => ({ flags });
  const split = {
    keep: half([{ holds: false, fresh: true }, { holds: false, fresh: false }]),
    new: half([{ holds: false, fresh: true }, { holds: true, fresh: true }]),
  };
  assert.equal(splitAcceptLabel({ keep: half([{ holds: false, fresh: true }]), new: half([]) }),
    'Accept anyway: 1 flagged song leaves its band');
  assert.equal(splitBrokenFresh(split), 2);
  assert.equal(splitAcceptLabel(split), 'Accept anyway: 2 flagged songs leave their band');
  assert.equal(splitAcceptLabel({ keep: half([]), new: half([{ holds: true, fresh: true }]) }), 'Accept split');
});

test('tune targets count flags and hand-set levels without a flag, once per song', () => {
  const flag = { kind: 'band', band: 'no', bound: 'in', barAtFlag: null, calibratedAt: null, stale: false };
  const entries = [
    { id: '1', moods: [{ mood: 'sad', auto: 0.5, user: null, flag }] },
    { id: '2', moods: [{ mood: 'sad', auto: 0.5, user: 0.75, flag: null }] },
    { id: '3', moods: [{ mood: 'sad', auto: 0.5, user: 1, flag }] },
    { id: '4', moods: [{ mood: 'sad', auto: 0.5, user: null, flag: null }] },
    { id: '5', moods: [{ mood: 'calm', auto: 0.5, user: 1, flag: null }] },
  ];
  assert.equal(tuneTargetCount(entries, 'sad'), 3);
  assert.equal(tuneTargetCount(entries, 'calm'), 1);
  assert.equal(tuneTargetCount(entries, 'grim'), 0);
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

test('top ranks and their labels', () => {
  const r = {
    top: [{ song: song('a'), bar: 0.9 }, { song: song('n'), bar: 0.8 }, { song: song('b'), bar: 0.7 }],
    topBefore: [{ song: song('b'), bar: 0.9 }, { song: song('a'), bar: 0.85 }, { song: song('d'), bar: 0.8 }],
  };
  const ranks = topRanks(r);
  assert.deepEqual(ranks.get('a'), { before: 2, after: 1 });
  assert.deepEqual(ranks.get('d'), { before: 3, after: null });
  assert.equal(rankLabel(ranks.get('a')), '#2 → #1');
  assert.equal(rankLabel(ranks.get('n')), 'new');
  assert.equal(rankLabel(ranks.get('d')), 'drops out');
  assert.equal(rankLabel({ before: 4, after: 4 }), 'same');
});

test('the flags link counts flags and hand-set levels apart', () => {
  const flag = { kind: 'band', band: 'no', bound: 'in', barAtFlag: null, calibratedAt: null, stale: false, staleReason: null };
  const entries = [
    { id: '1', moods: [{ mood: 'sad', auto: 0.5, user: null, flag }] },
    { id: '2', moods: [{ mood: 'sad', auto: 0.5, user: 0.75, flag: null }] },
    { id: '3', moods: [{ mood: 'sad', auto: 0.5, user: 1, flag }] },
    { id: '4', moods: [{ mood: 'sad', auto: 0.5, user: 0, flag: null }] },
  ];
  assert.equal(handSetCount(entries, 'sad'), 2);
  assert.deepEqual([...tuneTargetIds(entries, 'sad')].sort(), ['1', '2', '3', '4']);
  assert.equal(flagsLinkLabel(2, 2), '2 flags · 2 hand-set');
  assert.equal(flagsLinkLabel(1, 0), '1 flag');
  assert.equal(flagsLinkLabel(0, 0), '0 flags');
});

test('a hand-set level counts as high from 0.75, and a flag decides over a level', () => {
  const flag = { kind: 'band', band: 'stretch', bound: 'in' };
  assert.equal(reviewTargetIsHigh(null, 1), true);
  assert.equal(reviewTargetIsHigh(null, 0.75), true);
  assert.equal(reviewTargetIsHigh(null, 0.5), false);
  assert.equal(reviewTargetIsHigh(null, null), false);
  assert.equal(reviewTargetIsHigh(flag, 1), false);                   // the flag decides
});
