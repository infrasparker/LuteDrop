// Unit tests for the Moods page's pure view-model helpers (src/utils/moodPage.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  bandWord,
  breaksLabel,
  cellState,
  columnChips,
  confirmLine,
  confirmTitle,
  flagBounds,
  lineDiff,
  mergeTop,
  minusLinesLabel,
  movedMost,
  neverSuggestWords,
  nextVersionName,
  parseLines,
  reachedBars,
  regressions,
  sortAndCollapse,
  strongestOtherMood,
  useLabel,
  verdictText,
} from '../src/utils/moodPage.ts';

const flag = (over) => ({
  kind: 'band', band: 'understandable', bound: 'at_least', barAtFlag: null, calibratedAt: null, stale: false, staleReason: null,
  ...over,
});
const song = (id, name = id) => ({ entryId: id, name, album: '' });
const outcome = (id, before, after, holds, over = {}) => ({ song: song(id), before, after, holds, fresh: true, focus: false, ...over });

// --- Band words ---

test('band words: at least the top bands read as the band itself', () => {
  assert.equal(bandWord(flag({ band: 'definitely', bound: 'at_least' }), null), 'Definitely');
  assert.equal(bandWord(flag({ band: 'understandable', bound: 'at_least' }), null), 'Understandable');
  assert.equal(bandWord(flag({ band: 'stretch', bound: 'in' }), null), 'Stretch');
  assert.equal(bandWord(flag({ band: 'no', bound: 'in' }), null), 'No');
  assert.equal(bandWord(flag({ band: 'stretch', bound: 'at_least' }), null), 'Stretch or more');
  assert.equal(bandWord(flag({ band: 'stretch', bound: 'at_most' }), null), 'Stretch or less');
  assert.equal(bandWord(flag({ kind: 'higher', band: null }), null), 'Higher');
  assert.equal(bandWord(flag({ kind: 'lower', band: null }), null), 'Lower');
  assert.equal(bandWord(null, 0.75), 'Set by you');
});

test('band words abbreviate for narrow layouts', () => {
  assert.equal(bandWord(flag({ band: 'definitely' }), null, true), 'Def');
  assert.equal(bandWord(flag({ band: 'understandable' }), null, true), 'Und');
  assert.equal(bandWord(flag({ band: 'stretch', bound: 'in' }), null, true), 'Str');
  assert.equal(bandWord(flag({ band: 'stretch', bound: 'at_most' }), null, true), '≤Str');
  assert.equal(bandWord(flag({ band: 'stretch', bound: 'at_least' }), null, true), '≥Str');
  assert.equal(bandWord(flag({ band: 'no', bound: 'in' }), null, true), 'No');
});

// --- Bounds and cell state ---

test('flag bounds mirror the tuner: bands by bound, directions past the bar, levels as band flags', () => {
  assert.deepEqual(flagBounds(flag({ band: 'understandable', bound: 'at_least' }), null, 0.5), { min: 0.75, max: null });
  assert.deepEqual(flagBounds(flag({ band: 'stretch', bound: 'in' }), null, 0.5), { min: 0.5, max: 0.75 });
  assert.deepEqual(flagBounds(flag({ band: 'no', bound: 'in' }), null, 0.5), { min: null, max: 0.5 });
  const up = flagBounds(flag({ kind: 'higher', band: null, barAtFlag: 0.4 }), null, 0.6);
  assert.ok(Math.abs(up.min - 0.5) < 1e-9 && up.max === null);
  const stale = flagBounds(flag({ kind: 'lower', band: null, barAtFlag: 0.9, stale: true, staleReason: 'calibration' }), null, 0.6);
  assert.ok(Math.abs(stale.max - 0.5) < 1e-9);
  assert.deepEqual(flagBounds(null, 1, 0.5), { min: 0.9, max: null });
  assert.deepEqual(flagBounds(null, 0.25, 0.5), { min: null, max: 0.5 });
  assert.equal(flagBounds(null, null, 0.5), null);
});

test('cell state: holds, closer, worse, and a newly broken flag is worse', () => {
  const b = { min: 0.75, max: null };
  assert.equal(cellState({ holds: true, after: 0.8 }, { holds: false, after: 0.6 }, b), 'holds');
  assert.equal(cellState({ holds: false, after: 0.7 }, { holds: false, after: 0.6 }, b), 'closer');
  assert.equal(cellState({ holds: false, after: 0.5 }, { holds: false, after: 0.6 }, b), 'worse');
  assert.equal(cellState({ holds: false, after: 0.74 }, { holds: true, after: 0.8 }, b), 'worse');
  assert.equal(cellState({ holds: false, after: 0.6 }, { holds: false, after: 0.6 }, b), 'worse');
  // The Current column itself never alarms.
  assert.equal(cellState({ holds: false, after: 0.6 }, null, b), 'plain');
  assert.equal(cellState({ holds: true, after: 0.8 }, null, b), 'holds');
});

// --- Regressions and the confirm ---

test('regressions: fresh flags that hold under Current and break, plus flagged high songs that leave the top', () => {
  const base = { flags: [outcome('a', 0.8, 0.8, true), outcome('b', 0.6, 0.6, false), outcome('c', 0.9, 0.9, true), outcome('s', 0.8, 0.8, true, { fresh: false })] };
  const v = {
    flags: [outcome('a', 0.8, 0.72, false), outcome('b', 0.6, 0.55, false), outcome('c', 0.9, 0.91, true), outcome('s', 0.8, 0.4, false, { fresh: false })],
    top: [{ song: song('x'), bar: 0.9 }],
    topBefore: [{ song: song('c'), bar: 0.9 }, { song: song('x'), bar: 0.8 }],
  };
  const high = new Set(['c']);
  assert.deepEqual(regressions(v, base, high).map((o) => o.song.entryId), ['a', 'c']);
  assert.deepEqual(regressions(v, base, new Set()).map((o) => o.song.entryId), ['a']);
});

test('confirm copy', () => {
  assert.equal(confirmTitle(1), 'Break 1 flag?');
  assert.equal(confirmTitle(3), 'Break 3 flags?');
  assert.equal(confirmLine('White Orchard', 0.81, 0.72, 'Understandable'), 'White Orchard: 0.81 → 0.72 (Understandable)');
  assert.equal(useLabel(0), 'Use');
  assert.equal(useLabel(2), 'Use anyway');
  assert.equal(breaksLabel(1), '1 break');
  assert.equal(breaksLabel(2), '2 breaks');
});

// --- Verdict and chips ---

test('verdict: held / total and songs with the change', () => {
  const r = { flags: [outcome('a', 0, 0, true), outcome('b', 0, 0, false)], firesBefore: 35, firesAfter: 38 };
  assert.equal(verdictText(r, false), '1/2 · 38 songs (+3)');
  assert.equal(verdictText({ ...r, firesAfter: 33 }, false), '1/2 · 33 songs (−2)');
  assert.equal(verdictText({ ...r, firesAfter: 35 }, false), '1/2 · 35 songs (±0)');
  assert.equal(verdictText(r, true), '1/2 · 35 songs');
  assert.equal(verdictText({ flags: [], firesBefore: 4, firesAfter: 5 }, false), '5 songs (+1)');
  assert.equal(verdictText({ flags: [outcome('a', 0, 0, true)], firesBefore: 1, firesAfter: 1 }, false), '1/1 · 1 song (±0)');
});

test('chips: shaky under 5 flags or weak leave-one-out, overlap rises, reads like only for generated lines', () => {
  const base = { flags: Array.from({ length: 6 }, (_, i) => outcome(`f${i}`, 0, 0, true)), loo: { held: 0, total: 0 }, overlap: [], names: [] };
  assert.equal(columnChips(base, 'Sorrowful', 'typed').shaky, false);
  assert.equal(columnChips({ ...base, flags: base.flags.slice(0, 4) }, 'Sorrowful', 'typed').shaky, true);
  assert.equal(columnChips({ ...base, loo: { held: 4, total: 6 } }, 'Sorrowful', 'suggested').shaky, true);
  assert.equal(columnChips({ ...base, loo: { held: 5, total: 6 } }, 'Sorrowful', 'suggested').shaky, false);
  const ov = columnChips({ ...base, overlap: [{ moodId: 'grim', before: 0.2, after: 0.35 }, { moodId: 'x', before: 0.2, after: 0.25 }] }, 'Sorrowful', 'typed');
  assert.deepEqual(ov.overlaps.map((o) => o.moodId), ['grim']);
  const names = [{ word: 'slow', rho: 0.9 }, { word: 'calm', rho: 0.8 }, { word: 'soft', rho: 0.7 }];
  assert.equal(columnChips({ ...base, names }, 'Sorrowful', 'suggested').readsLike, 'slow');
  assert.equal(columnChips({ ...base, names }, 'Sorrowful', 'typed').readsLike, null);
  assert.equal(columnChips({ ...base, names: [...names.slice(0, 2), { word: 'sorrowful', rho: 0.6 }] }, 'Sorrowful', 'arranged').readsLike, null);
});

// --- Flags table: sort and collapse ---

test('rows sort focus, regressed, broken, changed, holding; unchanged holding rows collapse', () => {
  const cell = (state, value, regression = false) => ({ state, value, regression });
  const rows = [
    { entryId: 'hold', focus: false, cells: [cell('holds', 0.8), cell('holds', 0.8)] },
    { entryId: 'changed', focus: false, cells: [cell('holds', 0.8), cell('holds', 0.85)] },
    { entryId: 'broken', focus: false, cells: [cell('plain', 0.6), cell('closer', 0.7)] },
    { entryId: 'reg', focus: false, cells: [cell('holds', 0.8), cell('worse', 0.7, true)] },
    { entryId: 'focus', focus: true, cells: [cell('holds', 0.9), cell('holds', 0.9)] },
    { entryId: 'hold2', focus: false, cells: [cell('holds', 0.3), cell('holds', 0.302)] },
  ];
  const { shown, collapsed } = sortAndCollapse(rows, false);
  assert.deepEqual(shown.map((r) => r.entryId), ['focus', 'reg', 'broken', 'changed']);
  assert.deepEqual(collapsed.map((r) => r.entryId), ['hold', 'hold2']);
  const open = sortAndCollapse(rows, true);
  assert.deepEqual(open.shown.map((r) => r.entryId), ['focus', 'reg', 'broken', 'changed', 'hold', 'hold2']);
  // A row with no value anywhere is not "changed".
  const none = sortAndCollapse([{ entryId: 'n', focus: false, cells: [cell('holds', null), cell('holds', null)] }], false);
  assert.equal(none.collapsed.length, 1);
});

// --- Top 15 ---

test('top merge: rank, move, new, out with ranks, same as current', () => {
  const t = (id, bar) => ({ song: song(id), bar });
  const before = [t('a', 0.9), t('b', 0.8), t('c', 0.7)];
  const after = [t('b', 0.91), t('a', 0.85), t('d', 0.8)];
  const m = mergeTop(after, before);
  assert.deepEqual(m.rows.map((r) => [r.entryId, r.rank, r.move, r.isNew]), [['b', 1, 1, false], ['a', 2, -1, false], ['d', 3, null, true]]);
  assert.deepEqual(m.out.map((o) => [o.entryId, o.oldRank]), [['c', 3]]);
  assert.equal(m.same, false);
  assert.equal(mergeTop(before, before).same, true);
});

test('moved most: five each way', () => {
  const mv = (id, b, a) => ({ song: song(id), before: b, after: a });
  const r = { risers: Array.from({ length: 8 }, (_, i) => mv(`u${i}`, 0.1, 0.5)), fallers: [mv('d', 0.5, 0.2)] };
  const m = movedMost(r);
  assert.equal(m.up.length, 5);
  assert.deepEqual(m.down.map((x) => x.song.entryId), ['d']);
});

test('strongest other mood from the shown values', () => {
  const moods = [
    { mood: 'sorrowful', auto: 0.9, user: null },
    { mood: 'mysterious', auto: 0.6, user: null },
    { mood: 'grim', auto: 0.2, user: 0.75 },
    { mood: 'energy', auto: 0.99, user: null },
  ];
  assert.equal(strongestOtherMood(moods, 'sorrowful'), 'grim');
  assert.equal(strongestOtherMood([{ mood: 'sorrowful', auto: 0.9, user: null }], 'sorrowful'), null);
});

// --- Lines ---

test('lines: parse, diff against current, missing label', () => {
  assert.deepEqual(parseLines(' grief \n\n a sad farewell\n'), ['grief', 'a sad farewell']);
  const d = lineDiff(['grief', 'melancholic'], ['a sad farewell', 'grief', 'remembrance']);
  assert.deepEqual(d.added, [false, true]);
  assert.deepEqual(d.missing, ['a sad farewell', 'remembrance']);
  assert.equal(minusLinesLabel(1), '−1 line');
  assert.equal(minusLinesLabel(2), '−2 lines');
});

test('never suggest takes the comma parts of a line', () => {
  assert.deepEqual(neverSuggestWords('memory, sad,  lonely'), ['memory', 'sad', 'lonely']);
  assert.deepEqual(neverSuggestWords('a sad farewell'), ['a sad farewell']);
  assert.deepEqual(neverSuggestWords(' , '), []);
});

test('next version name skips names in use', () => {
  assert.equal(nextVersionName([]), 'v2');
  assert.equal(nextVersionName(['v2', 'v3']), 'v4');
  assert.equal(nextVersionName(['v3']), 'v2');
});

test('reached bars: the bar each holding flag reaches', () => {
  const r = { flags: [outcome('a', 0.5, 0.8, true), outcome('b', 0.5, 0.4, false), outcome('s', 0.5, 0.9, true, { fresh: false })] };
  assert.deepEqual(reachedBars(r), { a: 0.8 });
});
