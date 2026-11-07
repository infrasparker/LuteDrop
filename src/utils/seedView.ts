// Seed view: a radial layout of the Scry map around the seed tracks.
//
// The 2D map (t-SNE) keeps neighbourhoods only roughly, so a seed's best
// matches can land far from it. The seed view fixes the one thing the list
// measures exactly: each track's DISTANCE from the centre comes from its
// similarity to the seed centroid (the same scores as the results list), and
// only its ANGLE comes from the map (its direction from the seeds there,
// partly evened out so one-sided maps still fill the disc; see
// ANGLE_EVENNESS), so clusters keep a rough sense of place.
//
// Positions are in a unit disc: centre (0, 0), y up, radius at most 1. The map
// scales the disc uniformly to the canvas, so rings stay circles and distance
// compares the same way in every direction.

export interface MapPoint {
  entryId: string;
  x: number;
  y: number;
}

export interface DiscPoint {
  x: number;
  y: number;
}

export interface SeedRing {
  radius: number;
  // The similarity a track on this ring has.
  score: number;
}

export interface SeedViewLayout {
  positions: Map<string, DiscPoint>;
  // Guide rings, inner to outer, each at a round similarity value.
  rings: SeedRing[];
  // Radius of the rim that holds tracks without a score (outside the
  // analyzer's result cap), or null when every track has one.
  unrankedRadius: number | null;
}

// Seeds sit on a small circle this far from the centre (multi-seed only).
export const SEED_RADIUS = 0.07;
// The most similar track sits here, just outside the seeds...
export const INNER_RADIUS = 0.15;
// ...and the least similar scored track here.
export const OUTER_RADIUS = 0.9;
// Tracks with no score: all less similar than every scored one (the analyzer
// returns the best N), so they share a rim outside the scored tracks.
export const UNRANKED_RADIUS = 0.97;
// Smallest gap, in disc units along the circle, between tracks on one ring.
export const MIN_ARC_GAP = 0.03;
// How far angles are evened out (0 = the exact map direction, 1 = evenly
// spaced in map-direction order). Seeds often sit near the edge of the map, so
// every other track lies in one narrow wedge from them; pure map directions
// would pile 100+ dots into a quarter of the disc. Blending towards even
// spacing keeps the order around the circle (and so which tracks are
// neighbours in direction) while using the whole disc.
export const ANGLE_EVENNESS = 0.6;
// Fractional ranks at which guide rings are drawn.
const RING_QUANTILES = [0.25, 0.5, 0.75, 1];

const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));

// Radius for a fractional rank q in [0, 1] (0 = most similar).
//
// Why rank, not a min-max of the raw similarity: MuQ cosine scores bunch up
// (most tracks sit in a narrow band) with a few outliers stretching the range,
// so a linear map of the score crowds most dots onto one ring and packs the
// best matches into the tiny centre. Rank keeps the order exact (higher
// similarity is always strictly closer; equal similarity, equal radius) and
// spreads tracks evenly. The square root makes the dot density even over the
// disc's AREA (the count within radius r grows with r squared), so the best
// matches near the centre get as much room as the rest. The guide rings carry
// the actual similarity values, so distance still reads as similarity.
export function radiusForRank(q: number): number {
  const clamped = Math.min(1, Math.max(0, q));
  return INNER_RADIUS + (OUTER_RADIUS - INNER_RADIUS) * Math.sqrt(clamped);
}

// Fractional rank in [0, 1] for a similarity, interpolated between the sorted
// (best first) scores; outside their range it clamps to the ends.
function rankForScore(sorted: number[], score: number): number {
  const n = sorted.length;
  if (n < 2) return 0;
  if (score >= sorted[0]) return 0;
  if (score <= sorted[n - 1]) return 1;
  for (let i = 0; i < n - 1; i++) {
    const a = sorted[i];
    const b = sorted[i + 1];
    if (score <= a && score > b) return (i + (a - score) / (a - b)) / (n - 1);
  }
  return 1;
}

// Spread tracks that share a ring so none sits on top of another: in angle
// order, each is pushed at least MIN_ARC_GAP (along the arc) past the one
// before. When the ring is too crowded for that, they are spaced evenly.
function spreadOnRing(angles: number[], radius: number): number[] {
  const n = angles.length;
  if (n < 2) return angles;
  const gap = MIN_ARC_GAP / Math.max(radius, 1e-6);
  const order = angles.map((a, i) => ({ a, i })).sort((p, q) => p.a - q.a || p.i - q.i);
  const out = angles.slice();
  if (gap * n >= Math.PI * 2) {
    order.forEach((p, k) => {
      out[p.i] = order[0].a + (Math.PI * 2 * k) / n;
    });
    return out;
  }
  let prev = -Infinity;
  for (const p of order) {
    const a = Math.max(p.a, prev + gap);
    out[p.i] = a;
    prev = a;
  }
  return out;
}

// Even out a set of directions (see ANGLE_EVENNESS) without changing their
// order around the circle. The widest empty gap becomes the seam; the evenly
// spaced set is centred on the middle of the occupied arc, so the tracks still
// face the way they do on the map.
export function evenAngles(angles: readonly number[], evenness = ANGLE_EVENNESS): number[] {
  const n = angles.length;
  if (n < 2 || evenness <= 0) return angles.slice();
  const TAU = Math.PI * 2;
  const norm = (a: number) => ((a % TAU) + TAU) % TAU;
  const order = angles.map((a, i) => ({ a: norm(a), i })).sort((p, q) => p.a - q.a || p.i - q.i);
  // The widest gap between neighbours (the last one wraps around).
  let seam = 0;
  let widest = -1;
  for (let k = 0; k < n; k++) {
    const gap = k === n - 1 ? order[0].a + TAU - order[k].a : order[k + 1].a - order[k].a;
    if (gap > widest) {
      widest = gap;
      seam = (k + 1) % n;
    }
  }
  // Unwrapped, increasing angles starting just after the seam.
  const run = order.map((_, k) => {
    const p = order[(seam + k) % n];
    return { i: p.i, a: p.a + (seam + k >= n ? TAU : 0) };
  });
  const mid = (run[0].a + run[n - 1].a) / 2;
  const out = new Array<number>(n);
  run.forEach((p, k) => {
    const even = mid + (TAU * (k - (n - 1) / 2)) / n;
    out[p.i] = (1 - evenness) * p.a + evenness * even;
  });
  return out;
}

// Round guide-ring values: the similarity at a few rank quantiles, rounded to
// two decimals, each placed at the radius that value really has.
function guideRings(sorted: number[]): SeedRing[] {
  if (sorted.length === 0) return [];
  const n = sorted.length;
  const rings: SeedRing[] = [];
  for (const q of RING_QUANTILES) {
    const pos = q * (n - 1);
    const lo = Math.floor(pos);
    const hi = Math.min(n - 1, lo + 1);
    const raw = sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo);
    const score = Math.round(raw * 100) / 100;
    const radius = radiusForRank(rankForScore(sorted, score));
    if (rings.some((r) => r.score === score || Math.abs(r.radius - radius) < 0.05)) continue;
    rings.push({ radius, score });
  }
  return rings.sort((a, b) => a.radius - b.radius);
}

/**
 * Lay the map out around the seeds. `points` are the t-SNE positions (any
 * scale), `scores` the similarity of tracks to the seed centroid. Seeds are
 * placed at the centre; every other point gets a radius from its similarity
 * rank and an angle from its direction from the seeds on the map. Returns null
 * when no seed is on the map (there is no centre to lay out around).
 */
export function seedViewLayout(
  points: readonly MapPoint[],
  seedIds: readonly string[],
  scores: ReadonlyMap<string, number>,
): SeedViewLayout | null {
  const seedSet = new Set(seedIds);
  const seeds = points.filter((p) => seedSet.has(p.entryId));
  if (seeds.length === 0) return null;
  const cx = seeds.reduce((s, p) => s + p.x, 0) / seeds.length;
  const cy = seeds.reduce((s, p) => s + p.y, 0) / seeds.length;

  // Direction from the seed centroid on the map; a point exactly on the
  // centroid has none, so it gets a golden-angle fallback.
  const angleOf = (p: MapPoint, i: number): number => {
    const dx = p.x - cx;
    const dy = p.y - cy;
    return Math.hypot(dx, dy) < 1e-12 ? i * GOLDEN_ANGLE : Math.atan2(dy, dx);
  };

  const positions = new Map<string, DiscPoint>();

  // Seeds: one sits at the centre; several share a small circle, evenly
  // spaced in the order of their directions on the map, so each stays
  // clickable and their mean is the centre.
  if (seeds.length === 1) {
    positions.set(seeds[0].entryId, { x: 0, y: 0 });
  } else {
    const ordered = seeds.map((p, i) => ({ p, a: angleOf(p, i) })).sort((u, v) => u.a - v.a);
    ordered.forEach(({ p }, k) => {
      const a = ordered[0].a + (Math.PI * 2 * k) / ordered.length;
      positions.set(p.entryId, { x: SEED_RADIUS * Math.cos(a), y: SEED_RADIUS * Math.sin(a) });
    });
  }

  const others = points
    .map((p, i) => ({ p, i }))
    .filter(({ p }) => !seedSet.has(p.entryId));
  const scored = others.filter(({ p }) => Number.isFinite(scores.get(p.entryId)));
  const unscored = others.filter(({ p }) => !Number.isFinite(scores.get(p.entryId)));

  // Average rank per distinct score (ties share it), best first.
  const sorted = scored.map(({ p }) => scores.get(p.entryId)!).sort((a, b) => b - a);
  const rankByScore = new Map<number, number>();
  for (let i = 0; i < sorted.length; ) {
    let j = i;
    while (j + 1 < sorted.length && sorted[j + 1] === sorted[i]) j++;
    rankByScore.set(sorted[i], (i + j) / 2);
    i = j + 1;
  }
  const n = sorted.length;

  // Group by radius so tracks on one ring can be spread apart.
  const byRadius = new Map<number, { id: string; angle: number }[]>();
  const place = (id: string, radius: number, angle: number) => {
    const group = byRadius.get(radius) ?? [];
    group.push({ id, angle });
    byRadius.set(radius, group);
  };
  const angles = evenAngles(others.map(({ p, i }) => angleOf(p, i)));
  const angleById = new Map(others.map(({ p }, k) => [p.entryId, angles[k]]));
  for (const { p } of scored) {
    const rank = rankByScore.get(scores.get(p.entryId)!)!;
    place(p.entryId, radiusForRank(n > 1 ? rank / (n - 1) : 0), angleById.get(p.entryId)!);
  }
  for (const { p } of unscored) place(p.entryId, UNRANKED_RADIUS, angleById.get(p.entryId)!);

  for (const [radius, group] of byRadius) {
    const angles = spreadOnRing(group.map((g) => g.angle), radius);
    group.forEach((g, k) => {
      positions.set(g.id, { x: radius * Math.cos(angles[k]), y: radius * Math.sin(angles[k]) });
    });
  }

  return {
    positions,
    rings: guideRings(sorted),
    unrankedRadius: unscored.length > 0 ? UNRANKED_RADIUS : null,
  };
}
