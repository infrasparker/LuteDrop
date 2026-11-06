import type { LibraryEntry, MoodScale, MoodStep, Pool, Role } from '@generated/index';
import { effectiveStep, scaleValues } from './mood';

/// One mood pool: its id (as stored), the hedged name shown everywhere, and
/// its place in the vocabulary order (buttons, chips and menus follow it).
export interface PoolInfo {
  id: Pool;
  name: string;
  order: number;
}

/// The fixed pool vocabulary. Tension is never suggested by the analyzer; it
/// exists so tracks can be moved into it by hand.
export const POOLS: readonly PoolInfo[] = [
  { id: 'peaceful', name: 'Peaceful', order: 0 },
  { id: 'lighthearted', name: 'Lighthearted', order: 1 },
  { id: 'somber', name: 'Somber', order: 2 },
  { id: 'awe', name: 'Wonder', order: 3 },
  { id: 'mysterious', name: 'Mysterious', order: 4 },
  { id: 'epic', name: 'Epic', order: 5 },
  { id: 'tension', name: 'Tense', order: 6 },
];

const BY_ID = new Map<Pool, PoolInfo>(POOLS.map((p) => [p.id, p]));

/// The pool's display name, or null for no pool.
export const poolName = (pool: Pool | null): string | null => (pool === null ? null : (BY_ID.get(pool)?.name ?? pool));

/// The pool shown for an entry: the confirmed one, else the suggestion.
export const effectivePool = (entry: LibraryEntry): Pool | null => entry.poolUser ?? entry.poolAuto;

/// The analyzer's suggestion as one line, e.g. "Epic 0.84 · next: Wonder".
export function suggestionText(entry: LibraryEntry): string | null {
  if (entry.poolAuto === null) return null;
  const p = entry.poolAutoP === null ? '' : ` ${entry.poolAutoP.toFixed(2)}`;
  const next = entry.poolAutoRunner === null ? '' : ` · next: ${poolName(entry.poolAutoRunner)}`;
  return `${poolName(entry.poolAuto)}${p}${next}`;
}

// --- Filters ---

/// A pool chip in the filter bar: a pool, or music with no pool shown.
export type PoolChoice = Pool | 'unsorted';

export type RoleChoice = 'all' | Role | 'unset';

export interface PoolFilters {
  /// Selected pool chips; empty means any pool.
  pools: PoolChoice[];
  role: RoleChoice;
}

export function emptyPoolFilters(): PoolFilters {
  return { pools: [], role: 'all' };
}

/// The chip an entry counts under, or null for ambiance (which has no pool).
function poolChoiceOf(entry: LibraryEntry): PoolChoice | null {
  if (entry.entryType !== 'music') return null;
  return effectivePool(entry) ?? 'unsorted';
}

/// True when the entry passes the pool and role filters. With pool chips
/// selected, an entry passes when its shown pool (or Unsorted) is one of them,
/// so ambiance never passes. "Unset" role means music with no role shown.
export function matchesPoolFilters(entry: LibraryEntry, f: PoolFilters): boolean {
  if (f.pools.length > 0) {
    const choice = poolChoiceOf(entry);
    if (choice === null || !f.pools.includes(choice)) return false;
  }
  if (f.role === 'all') return true;
  if (f.role === 'unset') return entry.entryType === 'music' && entry.role === null;
  return entry.role === f.role;
}

/// How many music entries sit under each chip (every pool and Unsorted).
export function countByPool(entries: LibraryEntry[]): Map<PoolChoice, number> {
  const counts = new Map<PoolChoice, number>();
  for (const entry of entries) {
    const choice = poolChoiceOf(entry);
    if (choice !== null) counts.set(choice, (counts.get(choice) ?? 0) + 1);
  }
  return counts;
}

// --- Sorting ---

/// Sort keys for the filter bar. `match` keeps the incoming order (Scry's ranking).
export type SortKey = 'match' | 'name' | MoodScale;

export const SORT_LABELS: Record<SortKey, string> = {
  match: 'Best match',
  name: 'Name',
  energy: 'Energy',
  bright: 'Brightness',
  grand: 'Size',
};

/// Menu subtitles: which end of each scale comes first.
export const SORT_HINTS: Partial<Record<SortKey, string>> = {
  energy: 'Energetic first',
  bright: 'Bright first',
  grand: 'Grand first',
};

const STEP_RANK: Record<MoodStep, number> = { low: 0, mid: 1, high: 2 };

/// Orders two entries on one scale: highest shown step first (Energetic,
/// Bright, Grand), then highest z-score, entries without a value last.
function compareScale(a: LibraryEntry, b: LibraryEntry, scale: MoodScale): number {
  const va = scaleValues(a, scale);
  const vb = scaleValues(b, scale);
  const sa = effectiveStep(va.z, va.user);
  const sb = effectiveStep(vb.z, vb.user);
  if (sa === null || sb === null) return sa === sb ? 0 : sa === null ? 1 : -1;
  if (sa !== sb) return STEP_RANK[sb] - STEP_RANK[sa];
  return (vb.z ?? -Infinity) - (va.z ?? -Infinity);
}

const byName = (a: LibraryEntry, b: LibraryEntry): number => a.name.localeCompare(b.name);

/// A sorted copy of `items` (stable). `entryOf` picks the library entry from
/// an item, so Scry's rows can be sorted the same way as Library entries.
export function sortByKey<T>(items: T[], key: SortKey, entryOf: (item: T) => LibraryEntry): T[] {
  if (key === 'match') return items;
  const out = [...items];
  if (key === 'name') {
    out.sort((x, y) => byName(entryOf(x), entryOf(y)));
  } else {
    out.sort((x, y) => compareScale(entryOf(x), entryOf(y), key) || byName(entryOf(x), entryOf(y)));
  }
  return out;
}

// --- Review queue ---

/// Music that has a pool to look at (suggested or set) and has not been
/// reviewed yet. This is what the Review tab's badge counts.
export const needsReview = (entry: LibraryEntry): boolean =>
  entry.entryType === 'music' && entry.poolReviewedAt === null && effectivePool(entry) !== null;

const DAY_MS = 24 * 60 * 60 * 1000;
const createdMs = (e: LibraryEntry): number => Number(e.createdAt) || 0;

/// Review order: newest import day first; within a day, the least sure
/// suggestion (lowest probability) first; then newest, then name.
export function compareReview(a: LibraryEntry, b: LibraryEntry): number {
  const dayA = Math.floor(createdMs(a) / DAY_MS);
  const dayB = Math.floor(createdMs(b) / DAY_MS);
  if (dayA !== dayB) return dayB - dayA;
  const pa = a.poolAutoP ?? Infinity;
  const pb = b.poolAutoP ?? Infinity;
  if (pa !== pb) return pa - pb;
  return createdMs(b) - createdMs(a) || byName(a, b);
}
