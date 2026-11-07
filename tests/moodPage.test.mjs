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
  outRankLabel,
  topTitle,
  useShortBands,
  draftStale,
  splitRegressions,
  rescoreProblem,
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

test('cell state: holds, closer, worse, broken now; a newly broken flag is worse', () => {
  const b = { min: 0.75, max: null };
  assert.equal(cellState({ holds: true, after: 0.8 }, { holds: false, after: 0.6 }, b), 'holds');
  assert.equal(cellState({ holds: false, after: 0.7 }, { holds: false, after: 0.6 }, b), 'closer');
  assert.equal(cellState({ holds: false, after: 0.5 }, { holds: false, after: 0.6 }, b), 'worse');
  assert.equal(cellState({ holds: false, after: 0.74 }, { holds: true, after: 0.8 }, b), 'worse');
  // Broken under Current and no farther: still broken, but no worse.
  assert.equal(cellState({ holds: false, after: 0.6 }, { holds: false, after: 0.6 }, b), 'broken');
  // The Current column shows a flag broken now (never as "worse").
  assert.equal(cellState({ holds: false, after: 0.6 }, null, b), 'broken');
  assert.equal(cellState({ holds: true, after: 0.8 }, null, b), 'holds');
});

// --- Regressions and the confirm ---

test('regressions: flags that hold under Current and break, plus held high songs that leave the top', () => {
  const base = {
    flags: [
      outcome('a', 0.8, 0.8, true),
      outcome('b', 0.6, 0.6, false),
      outcome('c', 0.9, 0.9, true),
      outcome('w', 0.69, 0.69, false),
      outcome('s', 0.8, 0.8, true, { fresh: false }),
    ],
  };
  const v = {
    flags: [
      outcome('a', 0.8, 0.72, false),
      outcome('b', 0.6, 0.55, false),
      outcome('c', 0.9, 0.91, true),
      outcome('w', 0.69, 0.54, false),
      outcome('s', 0.8, 0.4, false, { fresh: false }),
    ],
    top: [{ song: song('x'), bar: 0.9 }],
    topBefore: [{ song: song('c'), bar: 0.9 }, { song: song('w'), bar: 0.85 }, { song: song('x'), bar: 0.8 }],
  };
  const high = new Set(['c', 'w']);
  // w was already broken under Current: it never counts, not even for leaving the top.
  const r = regressions(v, base, high);
  assert.deepEqual(r.map((x) => [x.outcome.song.entryId, x.leftTop]), [['a', false], ['c', true]]);
  assert.deepEqual(regressions(v, base, new Set()).map((x) => x.outcome.song.entryId), ['a']);
});

test('confirm copy', () => {
  assert.equal(confirmTitle(1), 'Break 1 flag?');
  assert.equal(confirmTitle(3), 'Break 3 flags?');
  assert.equal(confirmLine('White Orchard', 0.81, 0.72, 'Understandable'), 'White Orchard: 0.81 → 0.72 (Understandable)');
  assert.equal(
    confirmLine('Burial Isle', 0.78, 0.74, 'Understandable', true),
    'Burial Isle: 0.78 → 0.74 (Understandable, out of top 15)',
  );
  assert.equal(useLabel(0), 'Use');
  assert.equal(useLabel(2), 'Use anyway');
  assert.equal(breaksLabel(1), '1 break');
  assert.equal(breaksLabel(2), '2 breaks');
});

// --- Verdict and chips ---

test('verdict: "hold" with songs and change; Current says how many are broken now', () => {
  const r = { flags: [outcome('a', 0, 0, true), outcome('b', 0, 0, false)], firesBefore: 35, firesAfter: 38 };
  assert.equal(verdictText(r, false), '1/2 hold · 38 songs (+3)');
  assert.equal(verdictText({ ...r, firesAfter: 33 }, false), '1/2 hold · 33 songs (−2)');
  assert.equal(verdictText({ ...r, firesAfter: 35 }, false), '1/2 hold · 35 songs (±0)');
  // Current: held / total and how many are broken now (its songs are in the header).
  assert.equal(verdictText(r, true), '1/2 hold · 1 broken');
  assert.equal(verdictText({ ...r, flags: [outcome('a', 0, 0, true)] }, true), '1/1 hold');
  assert.equal(verdictText({ flags: [], firesBefore: 4, firesAfter: 4 }, true), '');
  assert.equal(verdictText({ flags: [], firesBefore: 4, firesAfter: 5 }, false), '5 songs (+1)');
  assert.equal(verdictText({ flags: [outcome('a', 0, 0, true)], firesBefore: 1, firesAfter: 1 }, false), '1/1 hold · 1 song (±0)');
});

test('chips: shaky under 5 flags (not with none) or weak leave-one-out, overlap rises, reads like another mood', () => {
  const base = { flags: Array.from({ length: 6 }, (_, i) => outcome(`f${i}`, 0, 0, true)), loo: { held: 0, total: 0 }, overlap: [], names: [] };
  const others = ['Serene', 'Horror'];
  assert.equal(columnChips(base, 'Sorrowful', others).shaky, false);
  assert.equal(columnChips({ ...base, flags: base.flags.slice(0, 4) }, 'Sorrowful', others).shaky, true);
  assert.equal(columnChips({ ...base, flags: [] }, 'Sorrowful', others).shaky, false, 'no flags: nothing to be shaky about');
  assert.equal(columnChips({ ...base, loo: { held: 4, total: 6 } }, 'Sorrowful', others).shaky, true);
  assert.equal(columnChips({ ...base, loo: { held: 5, total: 6 } }, 'Sorrowful', others).shaky, false);
  const ov = columnChips({ ...base, overlap: [{ moodId: 'grim', before: 0.2, after: 0.35 }, { moodId: 'x', before: 0.2, after: 0.25 }] }, 'Sorrowful', others);
  assert.deepEqual(ov.overlaps.map((o) => o.moodId), ['grim']);
  // Only when the top name idea is another mood's name.
  assert.equal(columnChips({ ...base, names: [{ word: 'serene', rho: 0.9 }] }, 'Sorrowful', others).readsLike, 'Serene');
  assert.equal(columnChips({ ...base, names: [{ word: 'slow', rho: 0.9 }, { word: 'serene', rho: 0.8 }] }, 'Sorrowful', others).readsLike, null);
  assert.equal(columnChips({ ...base, names: [{ word: 'sorrowful', rho: 0.9 }] }, 'Sorrowful', others).readsLike, null);
});

test('short band words with three or more versions, or narrow', () => {
  assert.equal(useShortBands(false, 2), false);
  assert.equal(useShortBands(false, 3), true);
  assert.equal(useShortBands(true, 0), true);
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
  // With Current the only column nothing collapses.
  const alone = sortAndCollapse([{ entryId: 'h', focus: false, cells: [cell('holds', 0.8)] }], false);
  assert.deepEqual([alone.shown.length, alone.collapsed.length], [1, 0]);
});

// --- Top 15 ---

test('top merge: rank, move, new, out with ranks, same as current', () => {
  const t = (id, bar) => ({ song: song(id), bar });
  const before = [t('a', 0.9), t('b', 0.8), t('c', 0.7)];
  const after = [t('b', 0.91), t('a', 0.85), t('d', 0.8)];
  const m = mergeTop(after, before);
  assert.deepEqual(m.rows.map((r) => [r.entryId, r.rank, r.move, r.isNew]), [['b', 1, 1, false], ['a', 2, -1, false], ['d', 3, null, true]]);
  assert.deepEqual(m.out.map((o) => [o.entryId, o.oldRank, o.newRank]), [['c', 3, null]]);
  assert.equal(m.same, false);
  assert.equal(mergeTop(before, before).same, true);
  // The new rank of a song that left, from the review's out ranks.
  const ranked = mergeTop(after, before, [{ song: song('c'), rank: 22 }]);
  assert.equal(ranked.out[0].newRank, 22);
  assert.equal(outRankLabel(ranked.out[0]), '3 → 22');
  assert.equal(outRankLabel(m.out[0]), '3 → out');
  assert.equal(topTitle('v4', ranked, false), 'Top 15 · v4 · 1 new · 1 out');
  assert.equal(topTitle('v4', mergeTop(before, before), false), 'Top 15 · v4');
  assert.equal(topTitle('Current', mergeTop(before, before), true), 'Top 15 · Current');
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

// --- Review freshness, split regressions, rescore problems ---

test('a draft review is stale when its lines, Current or the flags changed since, or while it evaluates', () => {
  const d = { review: {}, reviewedLines: 'a\nb', reviewedAgainst: 'cur', reviewedFlags: 'f1', evaluating: false };
  const now = { text: ' a\n\nb ', current: 'cur', flags: 'f1' };
  assert.equal(draftStale(d, now), false);
  assert.equal(draftStale(d, { ...now, text: 'a' }), true);
  assert.equal(draftStale(d, { ...now, current: 'other' }), true);
  assert.equal(draftStale(d, { ...now, flags: 'f2' }), true, 'a flag changed after the review');
  assert.equal(draftStale({ ...d, evaluating: true }, now), true);
  assert.equal(draftStale({ ...d, review: null }, now), false, 'no review: nothing to be stale');
});

test('split regressions: fresh flags broken in either half that held under Current', () => {
  const base = { flags: [outcome('a', 0.8, 0.8, true), outcome('w', 0.6, 0.6, false), outcome('n', 0.3, 0.3, true)] };
  const split = {
    keep: { flags: [outcome('a', 0.8, 0.7, false), outcome('w', 0.6, 0.5, false)] },
    new: { flags: [outcome('n', 0.3, 0.6, false), outcome('s', 0.3, 0.6, false, { fresh: false })] },
  };
  assert.deepEqual(splitRegressions(split, base).map((r) => r.outcome.song.entryId), ['a', 'n']);
});

test('a stale flag leaving the top list is not a regression', () => {
  const base = { flags: [outcome('c', 0.9, 0.9, true, { fresh: false })] };
  const v = { flags: [outcome('c', 0.9, 0.91, true, { fresh: false })], top: [], topBefore: [{ song: song('c'), bar: 0.9 }] };
  assert.deepEqual(regressions(v, base, new Set(['c'])), []);
});

test('rescore problem: why the bars were not updated', () => {
  const ok = { error: null, needsRecalibration: false, linesAtStart: 'a', linesNow: 'a' };
  assert.equal(rescoreProblem(ok), null);
  assert.equal(
    rescoreProblem({ ...ok, error: 'Turn on auto-tagging first: the analyzer rescores the moods' }),
    'Bars not updated: auto-tagging is off',
  );
  assert.equal(rescoreProblem({ ...ok, needsRecalibration: true, linesNow: 'b' }), 'Bars not updated: words changed, run again');
  assert.equal(rescoreProblem({ ...ok, needsRecalibration: true }), 'Bars not updated: not enough analyzed tracks');
  assert.equal(rescoreProblem({ ...ok, error: 'analyzer down' }), 'Bars not updated: analyzer down');
});
