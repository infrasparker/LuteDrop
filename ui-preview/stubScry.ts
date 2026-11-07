// Stand-in for the analyzer's Scry endpoints in the UI preview. The fixtures
// hold no embeddings, so each analyzed music track gets a made-up one: a
// cluster centre plus noise, from a hash of its id (stable across reloads).
// The map is a noisy 2D projection of it, so, like the real t-SNE map, it
// keeps neighbours only roughly. Only for looking at the UI.
import type { AppState, ScryHit, ScryPoint } from '@generated/index';

const DIM = 12;
const CLUSTERS = 6;

function hashSeed(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  return h >>> 0;
}

function rng(seed: number): () => number {
  let s = seed || 1;
  return () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

const centres = Array.from({ length: CLUSTERS }, (_, c) => {
  const r = rng(1000 + c);
  return Array.from({ length: DIM }, () => r() * 2 - 1);
});

let cache: { ids: string[]; vecs: Map<string, number[]>; map: ScryPoint[] } | null = null;

function build(state: AppState) {
  const ids = state.libraryEntries
    .filter((e) => e.entryType === 'music' && e.analysis?.status === 'embedded')
    .map((e) => e.id);
  if (cache && cache.ids.join() === ids.join()) return cache;
  const vecs = new Map<string, number[]>();
  const raw: { id: string; x: number; y: number }[] = [];
  for (const id of ids) {
    const r = rng(hashSeed(id));
    const centre = centres[Math.floor(r() * CLUSTERS)];
    const v = centre.map((c) => c + (r() * 2 - 1) * 0.7);
    vecs.set(id, v);
    raw.push({ id, x: v[0] + v[2] * 0.4 + (r() - 0.5) * 0.9, y: v[1] - v[3] * 0.4 + (r() - 0.5) * 0.9 });
  }
  const xs = raw.map((p) => p.x);
  const ys = raw.map((p) => p.y);
  const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const map = raw.map((p) => ({ entryId: p.id, x: (p.x - x0) / (x1 - x0 || 1), y: (p.y - y0) / (y1 - y0 || 1) }));
  cache = { ids, vecs, map };
  return cache;
}

export function stubScryMap(state: AppState): ScryPoint[] {
  return build(state).map;
}

const unit = (v: number[]) => {
  const n = Math.hypot(...v) || 1;
  return v.map((x) => x / n);
};

export function stubSimilar(state: AppState, entryIds: string[], limit: number): ScryHit[] {
  const { ids, vecs } = build(state);
  const query = entryIds.map((id) => vecs.get(id)).filter((v): v is number[] => !!v).map(unit);
  if (query.length === 0) throw new Error('None of the selected tracks have been analyzed yet');
  const centroid = unit(query[0].map((_, d) => query.reduce((s, q) => s + q[d], 0) / query.length));
  const skip = new Set(entryIds);
  return ids
    .filter((id) => !skip.has(id))
    .map((id) => ({ entryId: id, score: unit(vecs.get(id)!).reduce((s, x, d) => s + x * centroid[d], 0) }))
    .sort((a, b) => b.score - a.score)
    .slice(0, Math.min(Math.max(limit, 1), 500));
}
