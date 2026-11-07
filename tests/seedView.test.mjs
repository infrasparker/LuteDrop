// Unit tests for the Scry seed view layout (src/utils/seedView.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  INNER_RADIUS,
  MIN_ARC_GAP,
  OUTER_RADIUS,
  SEED_RADIUS,
  UNRANKED_RADIUS,
  evenAngles,
  radiusForRank,
  seedViewLayout,
} from '../src/utils/seedView.ts';

const radius = (p) => Math.hypot(p.x, p.y);
const angle = (p) => Math.atan2(p.y, p.x);
// Smallest difference between two angles, in [0, pi].
const angleGap = (a, b) => Math.abs(Math.atan2(Math.sin(a - b), Math.cos(a - b)));

// A deterministic pseudo-random sequence, so the tests are repeatable.
function rng(seed) {
  let s = seed;
  return () => {
    s = (s * 1664525 + 1013904223) % 4294967296;
    return s / 4294967296;
  };
}

test('radius order follows similarity order exactly', () => {
  const rand = rng(7);
  const points = [{ entryId: 'seed', x: 0.5, y: 0.5 }];
  const scores = new Map();
  for (let i = 0; i < 129; i++) {
    const id = `t${i}`;
    // Map position unrelated to the score, like a t-SNE that misplaces tracks.
    points.push({ entryId: id, x: rand(), y: rand() });
    scores.set(id, 0.4 + 0.55 * rand());
  }
  const layout = seedViewLayout(points, ['seed'], scores);
  const ids = [...scores.keys()].sort((a, b) => scores.get(b) - scores.get(a));
  for (let i = 1; i < ids.length; i++) {
    const closer = radius(layout.positions.get(ids[i - 1]));
    const farther = radius(layout.positions.get(ids[i]));
    assert.ok(closer < farther, `${ids[i - 1]} (${scores.get(ids[i - 1])}) should be closer than ${ids[i]}`);
  }
  assert.ok(Math.abs(radius(layout.positions.get(ids[0])) - INNER_RADIUS) < 1e-9);
  assert.ok(Math.abs(radius(layout.positions.get(ids.at(-1))) - OUTER_RADIUS) < 1e-9);
  assert.equal(layout.unrankedRadius, null);
});

test('angles follow the direction from the seed on the map', () => {
  // Already evenly spread, so evening out leaves the directions exact.
  const points = [
    { entryId: 'seed', x: 0.5, y: 0.5 },
    { entryId: 'east', x: 0.9, y: 0.5 },
    { entryId: 'north', x: 0.5, y: 0.6 },
    { entryId: 'west', x: 0.1, y: 0.5 },
    { entryId: 'south', x: 0.5, y: 0.2 },
  ];
  const scores = new Map([['east', 0.9], ['north', 0.5], ['west', 0.7], ['south', 0.6]]);
  const { positions } = seedViewLayout(points, ['seed'], scores);
  assert.ok(angleGap(angle(positions.get('east')), 0) < 1e-9);
  assert.ok(angleGap(angle(positions.get('north')), Math.PI / 2) < 1e-9);
  assert.ok(angleGap(angle(positions.get('west')), Math.PI) < 1e-9);
  assert.ok(angleGap(angle(positions.get('south')), -Math.PI / 2) < 1e-9);
  // Distance comes from the score, not from the map: far-off "east" is the
  // closest and nearby "north" the farthest.
  const order = ['east', 'west', 'south', 'north'].map((id) => radius(positions.get(id)));
  for (let i = 1; i < order.length; i++) assert.ok(order[i - 1] < order[i]);
  assert.deepEqual(positions.get('seed'), { x: 0, y: 0 });
});

test('a seed at the edge of the map: order around the circle is kept, the wedge is widened', () => {
  const rand = rng(3);
  // Seed in a corner: every other track lies within a quarter turn of it.
  const points = [{ entryId: 'seed', x: 0, y: 0 }];
  const scores = new Map();
  for (let i = 0; i < 60; i++) {
    points.push({ entryId: `t${i}`, x: 0.05 + rand(), y: 0.05 + rand() });
    scores.set(`t${i}`, rand());
  }
  const { positions } = seedViewLayout(points, ['seed'], scores);
  const ids = [...scores.keys()];
  const mapAngle = (id) => {
    const p = points.find((q) => q.entryId === id);
    return Math.atan2(p.y, p.x);
  };
  const byMap = [...ids].sort((a, b) => mapAngle(a) - mapAngle(b));
  // Unwrap the laid-out angles in map order: they must keep increasing.
  let prev = -Infinity;
  let first = null;
  for (const id of byMap) {
    let a = angle(positions.get(id));
    if (first === null) first = a;
    while (a < prev) a += Math.PI * 2;
    assert.ok(a > prev, 'map-direction order is kept');
    prev = a;
  }
  // The quarter-turn wedge now spans well over half the circle.
  assert.ok(prev - first > Math.PI * 1.2);
  // ...still centred on the same direction (north-east, 45 degrees).
  const mx = ids.reduce((s, id) => s + Math.cos(angle(positions.get(id))), 0);
  const my = ids.reduce((s, id) => s + Math.sin(angle(positions.get(id))), 0);
  assert.ok(angleGap(Math.atan2(my, mx), Math.PI / 4) < 0.35);
});

test('evenAngles: no evenness keeps the angles, full evenness spaces them evenly', () => {
  const input = [0.1, 0.2, 0.25, 1.0];
  assert.deepEqual(evenAngles(input, 0), input);
  const even = evenAngles(input, 1);
  const sorted = [...even].sort((a, b) => a - b);
  for (let i = 1; i < sorted.length; i++) assert.ok(Math.abs(sorted[i] - sorted[i - 1] - Math.PI / 2) < 1e-9);
  // Same order as the input.
  for (let i = 1; i < input.length; i++) assert.ok(even[i] > even[i - 1]);
  assert.deepEqual(evenAngles([2], 1), [2]);
  assert.deepEqual(evenAngles([], 1), []);
});

test('several seeds: centred on their centroid, spread around the middle', () => {
  const points = [
    { entryId: 'a', x: 0.2, y: 0.2 },
    { entryId: 'b', x: 0.4, y: 0.2 },
    { entryId: 'c', x: 0.3, y: 0.5 },
    // Due east of the seed centroid (0.3, 0.3).
    { entryId: 'x', x: 0.9, y: 0.3 },
    // Due north of it.
    { entryId: 'y', x: 0.3, y: 0.9 },
  ];
  const scores = new Map([['x', 0.8], ['y', 0.6]]);
  const { positions } = seedViewLayout(points, ['a', 'b', 'c'], scores);
  const seeds = ['a', 'b', 'c'].map((id) => positions.get(id));
  for (const s of seeds) assert.ok(Math.abs(radius(s) - SEED_RADIUS) < 1e-9);
  const mx = seeds.reduce((s, p) => s + p.x, 0) / 3;
  const my = seeds.reduce((s, p) => s + p.y, 0) / 3;
  assert.ok(Math.hypot(mx, my) < 1e-9, 'seeds average to the centre');
  // Evenly spaced, so each stays clickable.
  for (let i = 0; i < 3; i++) {
    for (let j = i + 1; j < 3; j++) assert.ok(Math.hypot(seeds[i].x - seeds[j].x, seeds[i].y - seeds[j].y) > 0.1);
  }
  // East stays east-ish, north stays north-ish (evened out a little).
  assert.ok(positions.get('x').x > 0 && angle(positions.get('x')) < angle(positions.get('y')));
  assert.ok(positions.get('y').y > 0);
  // Every seed is inside every other track.
  assert.ok(radius(positions.get('x')) > SEED_RADIUS);
});

test('one other track sits at the inner radius', () => {
  const layout = seedViewLayout(
    [{ entryId: 's', x: 0, y: 0 }, { entryId: 't', x: 1, y: 1 }],
    ['s'],
    new Map([['t', 0.3]]),
  );
  assert.ok(Math.abs(radius(layout.positions.get('t')) - INNER_RADIUS) < 1e-9);
  assert.equal(layout.rings.length, 1);
  assert.equal(layout.rings[0].score, 0.3);
});

test('a lone seed with nothing else is laid out at the centre', () => {
  const layout = seedViewLayout([{ entryId: 's', x: 0.3, y: 0.7 }], ['s'], new Map());
  assert.deepEqual(layout.positions.get('s'), { x: 0, y: 0 });
  assert.deepEqual(layout.rings, []);
});

test('equal similarity: one shared radius, and no two tracks overlap', () => {
  const points = [{ entryId: 's', x: 0.5, y: 0.5 }];
  const scores = new Map();
  // Ten tracks in the same spot (identical tracks), all with the same score.
  for (let i = 0; i < 10; i++) {
    points.push({ entryId: `t${i}`, x: 0.8, y: 0.5 });
    scores.set(`t${i}`, 0.75);
  }
  const { positions } = seedViewLayout(points, ['s'], scores);
  const rs = [...scores.keys()].map((id) => radius(positions.get(id)));
  for (const r of rs) assert.ok(Math.abs(r - rs[0]) < 1e-9);
  const pts = [...scores.keys()].map((id) => positions.get(id));
  for (let i = 0; i < pts.length; i++) {
    for (let j = i + 1; j < pts.length; j++) {
      assert.ok(Math.hypot(pts[i].x - pts[j].x, pts[i].y - pts[j].y) > MIN_ARC_GAP * 0.9);
    }
  }
});

test('a track on top of the seed still gets an angle', () => {
  const { positions } = seedViewLayout(
    [{ entryId: 's', x: 0.5, y: 0.5 }, { entryId: 'twin', x: 0.5, y: 0.5 }],
    ['s'],
    new Map([['twin', 0.99]]),
  );
  const p = positions.get('twin');
  assert.ok(Number.isFinite(p.x) && Number.isFinite(p.y));
  assert.ok(Math.abs(radius(p) - INNER_RADIUS) < 1e-9);
});

test('tracks without a score go on the outer rim', () => {
  const points = [
    { entryId: 's', x: 0.5, y: 0.5 },
    { entryId: 'scored', x: 0.6, y: 0.5 },
    { entryId: 'capped', x: 0.5, y: 0.9 },
  ];
  const layout = seedViewLayout(points, ['s'], new Map([['scored', 0.2]]));
  assert.ok(Math.abs(radius(layout.positions.get('capped')) - UNRANKED_RADIUS) < 1e-9);
  assert.equal(layout.unrankedRadius, UNRANKED_RADIUS);
});

test('no seed on the map: no layout', () => {
  assert.equal(seedViewLayout([{ entryId: 'a', x: 0, y: 0 }], ['gone'], new Map([['a', 1]])), null);
  assert.equal(seedViewLayout([], [], new Map()), null);
});

test('guide rings sit where tracks with that score sit', () => {
  const points = [{ entryId: 's', x: 0, y: 0 }];
  const scores = new Map();
  for (let i = 0; i < 21; i++) {
    points.push({ entryId: `t${i}`, x: Math.cos(i), y: Math.sin(i) });
    scores.set(`t${i}`, 0.5 + i * 0.02);
  }
  const layout = seedViewLayout(points, ['s'], scores);
  assert.ok(layout.rings.length >= 3);
  for (let i = 1; i < layout.rings.length; i++) {
    assert.ok(layout.rings[i].radius > layout.rings[i - 1].radius);
    assert.ok(layout.rings[i].score < layout.rings[i - 1].score);
  }
  for (const ring of layout.rings) {
    // A track whose score equals the ring value lies on the ring.
    for (const [id, s] of scores) {
      if (Math.abs(s - ring.score) < 1e-9) assert.ok(Math.abs(radius(layout.positions.get(id)) - ring.radius) < 1e-9);
    }
  }
});

test('radiusForRank is increasing and clamped', () => {
  assert.equal(radiusForRank(0), INNER_RADIUS);
  assert.equal(radiusForRank(1), OUTER_RADIUS);
  assert.equal(radiusForRank(-1), INNER_RADIUS);
  assert.equal(radiusForRank(2), OUTER_RADIUS);
  assert.ok(radiusForRank(0.3) < radiusForRank(0.31));
});
