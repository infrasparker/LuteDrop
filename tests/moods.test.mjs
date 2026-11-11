// Unit tests for the pure lens/filter/sort functions in src/utils/moods.ts.
// Run with `npm run test:frontend` (Node strips the TypeScript types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ENERGY_HIGH_ABOVE,
  ENERGY_LOW_BELOW,
  calibrationLabel,
  canSortBy,
  chipState,
  cycleChip,
  cycleMood,
  lensOn,
  emptyChipSelection,
  emptyMoodFilters,
  energyBand,
  lensLabel,
  lensScore,
  matchesEnergyBands,
  matchesMoodFilters,
  matchesTagSelection,
  setMoodVocabulary,
  sortByKey,
  sortForLens,
  toggleIn,
} from '../src/utils/moods.ts';

setMoodVocabulary({
  moods: [
    { id: 'heroic', name: 'Heroic', prompts: [] },
    { id: 'grim', name: 'Grim', prompts: [] },
  ],
  energy: { positive: [], negative: [] },
});

/// A library entry with only the fields these functions read.
function entry(name, { energy = null, energyUser = null, heroic = null, heroicUser = null, grim = null, type = 'music', role = 'nonCombat' } = {}) {
  const moods = [];
  if (heroic !== null || heroicUser !== null) moods.push({ mood: 'heroic', auto: heroic, user: heroicUser });
  if (grim !== null) moods.push({ mood: 'grim', auto: grim, user: null });
  if (energy !== null || energyUser !== null) moods.push({ mood: 'energy', auto: energy, user: energyUser });
  return { id: name, name, entryType: type, role: type === 'music' ? role : null, moods, duration: 60 };
}

const filters = (over = {}) => ({ ...emptyMoodFilters(), ...over });

test('energy band edges: 0.375 is Mid, 0.625 is Mid, just outside is Low/High', () => {
  assert.equal(ENERGY_LOW_BELOW, 0.375);
  assert.equal(ENERGY_HIGH_ABOVE, 0.625);
  assert.equal(energyBand(0.375), 'mid');
  assert.equal(energyBand(0.625), 'mid');
  assert.equal(energyBand(0.3749), 'low');
  assert.equal(energyBand(0.6251), 'high');
});

test('energy band of the five levels', () => {
  assert.deepEqual([0, 0.25, 0.5, 0.75, 1].map(energyBand), ['low', 'low', 'mid', 'high', 'high']);
});

test('bands filter by the union of the bands that are on', () => {
  const levels = [0, 0.25, 0.5, 0.75, 1].map((v) => entry(`e${v}`, { energy: v }));
  const pass = (bands) => levels.filter((e) => matchesEnergyBands(e, bands)).map((e) => e.name);
  assert.deepEqual(pass([]), ['e0', 'e0.25', 'e0.5', 'e0.75', 'e1']);
  assert.deepEqual(pass(['low']), ['e0', 'e0.25']);
  assert.deepEqual(pass(['mid']), ['e0.5']);
  assert.deepEqual(pass(['high']), ['e0.75', 'e1']);
  assert.deepEqual(pass(['low', 'high']), ['e0', 'e0.25', 'e0.75', 'e1']);
  assert.deepEqual(pass(['high', 'mid', 'low']), ['e0', 'e0.25', 'e0.5', 'e0.75', 'e1']);
});

test('the level set by the user wins over the suggestion for the band', () => {
  const e = entry('x', { energy: 0.9, energyUser: 0.25 });
  assert.equal(matchesEnergyBands(e, ['low']), true);
  assert.equal(matchesEnergyBands(e, ['high']), false);
});

test('a track with no Energy passes only when no band is on', () => {
  const e = entry('unscored');
  assert.equal(matchesMoodFilters(e, filters()), true);
  assert.equal(matchesMoodFilters(e, filters({ bands: ['low'] })), false);
  assert.equal(matchesMoodFilters(e, filters({ bands: ['low', 'mid', 'high'] })), false);
});

test('ambiance shows with nothing on and hides while a lens or band is on (as before)', () => {
  const amb = entry('rain', { type: 'ambiance' });
  assert.equal(matchesMoodFilters(amb, filters()), true);
  assert.equal(matchesMoodFilters(amb, filters({ lenses: ['heroic'] })), false);
  assert.equal(matchesMoodFilters(amb, filters({ bands: ['mid'] })), false);
});

test('role filter still applies with bands', () => {
  const e = entry('fight', { energy: 0.75, role: 'combat' });
  assert.equal(matchesMoodFilters(e, filters({ bands: ['high'], role: 'combat' })), true);
  assert.equal(matchesMoodFilters(e, filters({ bands: ['high'], role: 'nonCombat' })), false);
});

test('mood chips are on/off: toggling twice turns a mood off again', () => {
  assert.deepEqual(toggleIn([], 'heroic'), ['heroic']);
  assert.deepEqual(toggleIn(['heroic', 'grim'], 'heroic'), ['grim']);
  assert.deepEqual(toggleIn(toggleIn(['grim'], 'heroic'), 'heroic'), ['grim']);
});

test('several moods rank by their geometric mean; a mood lens never filters', () => {
  const f = filters({ lenses: ['heroic', 'grim'] });
  assert.ok(Math.abs(lensScore(entry('a', { heroic: 0.9, grim: 0.3 }), f) - Math.sqrt(.9 * .3)) < 1e-9);
  assert.equal(lensScore(entry('b', { heroic: 0.2 }), f), null);
  assert.equal(matchesMoodFilters(entry('c', { heroic: 0.9, grim: 0.9 }), f), true);
  assert.equal(lensLabel(f), 'combined High Heroic, High Grim');
  assert.equal(lensLabel(filters({ lenses: ['grim'] })), 'High Grim');
});

test('lens ranking does not depend on Energy, within the bands that are on', () => {
  const items = [
    entry('calm-heroic', { heroic: 0.9, energy: 0.6 }),
    entry('loud-meh', { heroic: 0.3, energy: 1 }),
    entry('mid-ok', { heroic: 0.6, energy: 0.5 }),
    entry('quiet', { heroic: 1, energy: 0 }),
  ];
  const f = filters({ lenses: ['heroic'], bands: ['mid', 'high'] });
  const shown = items.filter((e) => matchesMoodFilters(e, f));
  const names = (key) => sortByKey(shown, key, 'desc', f, (e) => e).map((e) => e.name);
  assert.deepEqual(names('lens'), ['calm-heroic', 'mid-ok', 'loud-meh']);
  // Energy is not a sort while a lens is on; asked for anyway, it ranks by the lens.
  assert.equal(canSortBy('energy', f), false);
  assert.deepEqual(names('energy'), ['calm-heroic', 'mid-ok', 'loud-meh']);
});

test('Energy sorts with no lens on', () => {
  const items = [entry('a', { energy: 0.2 }), entry('b', { energy: 0.9 }), entry('c')];
  const f = filters();
  assert.equal(canSortBy('energy', f), true);
  assert.deepEqual(sortByKey(items, 'energy', 'desc', f, (e) => e).map((e) => e.name), ['b', 'a', 'c']);
  assert.equal(canSortBy('name', filters({ lenses: ['heroic'] })), true);
});

test('clearing the last mood returns to the view default sort', () => {
  assert.deepEqual(sortForLens({ key: 'name', dir: 'asc' }, false, true, 'name'), { key: 'lens', dir: 'desc' });
  assert.deepEqual(sortForLens({ key: 'lens', dir: 'desc' }, true, false, 'name'), { key: 'name', dir: 'asc' });
  assert.deepEqual(sortForLens({ key: 'lens', dir: 'desc' }, true, false, 'match'), { key: 'match', dir: 'desc' });
});

test('tag chips keep three states: off, include, exclude', () => {
  let sel = emptyChipSelection();
  assert.equal(chipState(sel, 'boss'), 'off');
  sel = cycleChip(sel, 'boss');
  assert.equal(chipState(sel, 'boss'), 'include');
  sel = cycleChip(sel, 'boss');
  assert.equal(chipState(sel, 'boss'), 'exclude');
  assert.equal(matchesTagSelection(['boss'], sel), false);
  assert.equal(matchesTagSelection(['tavern'], sel), true);
  sel = cycleChip(sel, 'boss');
  assert.equal(chipState(sel, 'boss'), 'off');
  assert.deepEqual(sel, { include: [], exclude: [] });
});

test('tracks set to the same level rank within it by the suggestion behind the level', () => {
  const f = filters({ lenses: ['heroic'] });
  const items = [
    entry('setLow', { heroic: 0.95, heroicUser: 1 }),
    entry('setHigh', { heroic: 0.6, heroicUser: 1 }),
    entry('unset', { heroic: 0.9 }),
    entry('setBelow', { heroic: 0.99, heroicUser: 0.75 }),
  ];
  // Level first (1 > 0.9 > 0.75), then the suggestion inside the level (0.95 before 0.6).
  assert.deepEqual(sortByKey(items, 'lens', 'desc', f, (e) => e).map((e) => e.name), ['setLow', 'setHigh', 'unset', 'setBelow']);
  assert.deepEqual(sortByKey(items, 'lens', 'asc', f, (e) => e).map((e) => e.name), ['setBelow', 'unset', 'setHigh', 'setLow']);
});

test('Energy ties between set levels break by the suggested Energy, then by name', () => {
  const f = filters();
  const items = [
    entry('c', { energy: 0.3, energyUser: 0.5 }),
    entry('a', { energy: 0.3, energyUser: 0.5 }),
    entry('b', { energy: 0.55, energyUser: 0.5 }),
  ];
  assert.deepEqual(sortByKey(items, 'energy', 'desc', f, (e) => e).map((e) => e.name), ['b', 'a', 'c']);
});

test('calibrationLabel says where the calibration came from', () => {
  const lib = calibrationLabel({ calibratedAt: '1700000000000', tracks: 129, source: 'library', referenceSet: null });
  assert.ok(lib.includes('your library') && lib.includes('129 tracks'));
  const ref = calibrationLabel({ calibratedAt: '1700000000000', tracks: 300, source: 'reference', referenceSet: 'incompetech-300-v1' });
  assert.ok(ref.includes('reference set') && ref.includes('300') && ref.includes('incompetech-300-v1'));
  assert.equal(calibrationLabel(null), "Not calibrated yet. The next analysis run calibrates, against the analyzer's reference set, or once it covers 50 music tracks of your library.");
});


test('mood cycles support high and low preferences in either direction', () => {
  let f = filters();
  f = cycleMood(f, 'heroic');
  assert.deepEqual(f.lenses, ['heroic']);
  f = cycleMood(f, 'heroic');
  assert.deepEqual(f.lenses, []);
  assert.deepEqual(f.lowLenses, ['heroic']);
  assert.equal(lensOn(f), true);
  assert.equal(canSortBy('energy', f), false);
  f = cycleMood(f, 'heroic');
  assert.equal(lensOn(f), false);
  f = cycleMood(f, 'grim', -1);
  assert.deepEqual(f.lowLenses, ['grim']);
  f = cycleMood(f, 'grim', -1);
  assert.deepEqual(f.lenses, ['grim']);
  assert.deepEqual(f.lowLenses, []);
  f = cycleMood(f, 'grim', -1);
  assert.equal(lensOn(f), false);
  assert.equal(chipState(cycleChip(emptyChipSelection(), 'boss', -1), 'boss'), 'exclude');
});

test('mixed high/low moods combine desired directions without penalizing energy', () => {
  const f = filters({ lenses: ['heroic'], lowLenses: ['grim'] });
  const best = entry('balanced', { heroic: .8, grim: .1, energy: .2 });
  const bad = entry('too scary', { heroic: .99, grim: .9, energy: 1 });
  assert.ok(Math.abs(lensScore(best, f) - Math.sqrt(.8 * .9)) < 1e-9);
  assert.ok(Math.abs(lensScore(bad, f) - Math.sqrt(.99 * .1)) < 1e-9);
  assert.equal(lensScore(entry('unknown', { heroic: 1 }), f), null);
  assert.deepEqual(sortByKey([bad, best], 'lens', 'desc', f, e => e).map(e => e.name), ['balanced', 'too scary']);
  assert.equal(lensLabel(f), 'combined High Heroic, Low Grim');
  assert.equal(matchesMoodFilters(entry('rain', { type: 'ambiance' }), filters({ lowLenses: ['grim'] })), false);
});

test('low-only preferences respect manual levels and invert suggested tie breakers', () => {
  const f = filters({ lowLenses: ['heroic'] });
  const a = entry('a', { heroic: .9, heroicUser: .25 });
  const b = entry('b', { heroic: .2, heroicUser: .25 });
  assert.equal(lensScore(a, f), .75);
  assert.deepEqual(sortByKey([a, b, entry('unknown')], 'lens', 'desc', f, e => e).map(e => e.name), ['b', 'a', 'unknown']);
});


test('two high and two low requests use a normalized cross-product, including zero and unknown cases', () => {
  const f = filters({ lenses: ['heroic', 'festive'], lowLenses: ['grim', 'sorrowful'] });
  const e = entry('balanced', { heroic: .9, grim: .2 });
  e.moods.push({ mood: 'festive', auto: .8, user: null }, { mood: 'sorrowful', auto: .1, user: null });
  const score = lensScore(e, f);
  assert.ok(Math.abs(score - Math.pow(.9 * .8 * (1-.2) * (1-.1), .25)) < 1e-9);
  assert.equal(matchesMoodFilters(e, f), true, 'preference scores never exclude tracks');
  assert.ok(Math.abs(score - lensScore(e, filters({ lenses: ['festive', 'heroic'], lowLenses: ['sorrowful', 'grim'] }))) < 1e-9);
  e.moods.find(m => m.mood === 'grim').user = 1;
  assert.equal(lensScore(e, f), 0);
  assert.equal(matchesMoodFilters(e, f), true, 'zero fit still stays in the ranking');
  assert.equal(lensScore(entry('unknown'), f), null);
});
