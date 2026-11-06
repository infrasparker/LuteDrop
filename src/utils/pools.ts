import type { LibraryEntry, Pool, Role } from '@generated/index';

/// One mood pool: its id (as stored), the hedged name shown everywhere, and
/// its place in the vocabulary order (buttons, chips and menus follow it).
export interface PoolInfo {
  id: Pool;
  name: string;
  order: number;
}

/// The fixed pool vocabulary, in order. The analyzer can suggest every one.
export const POOLS: readonly PoolInfo[] = [
  { id: 'peaceful', name: 'Peaceful', order: 0 },
  { id: 'cheerful', name: 'Cheerful', order: 1 },
  { id: 'quirky', name: 'Quirky', order: 2 },
  { id: 'somber', name: 'Somber', order: 3 },
  { id: 'awe', name: 'Wonder', order: 4 },
  { id: 'mysterious', name: 'Mysterious', order: 5 },
  { id: 'heroic', name: 'Heroic', order: 6 },
  { id: 'menacing', name: 'Menacing', order: 7 },
];

const BY_ID = new Map<Pool, PoolInfo>(POOLS.map((p) => [p.id, p]));

/// The pool's display name, or null for no pool.
export const poolName = (pool: Pool | null): string | null => (pool === null ? null : (BY_ID.get(pool)?.name ?? pool));

/// The pool shown for an entry: the confirmed one, else the suggestion.
export const effectivePool = (entry: LibraryEntry): Pool | null => entry.poolUser ?? entry.poolAuto;

/// The analyzer's suggestion as one line, e.g. "Heroic 0.84 · next: Wonder".
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

/// Sort keys for the filter bar. `match` keeps the incoming order (Scry's
/// ranking); `confidence` is the analyzer's pool suggestion probability.
export type SortKey = 'match' | 'name' | 'confidence';

export const SORT_LABELS: Record<SortKey, string> = {
  match: 'Best match',
  name: 'Name',
  confidence: 'Suggestion confidence',
};

/// Menu subtitles.
export const SORT_HINTS: Partial<Record<SortKey, string>> = {
  confidence: 'Least sure first',
};

/// Orders two entries by the suggestion's probability, least sure first (as
/// in Review); entries without a suggestion last.
function compareConfidence(a: LibraryEntry, b: LibraryEntry): number {
  const pa = a.poolAutoP;
  const pb = b.poolAutoP;
  if (pa === null || pb === null) return pa === pb ? 0 : pa === null ? 1 : -1;
  return pa - pb;
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
    out.sort((x, y) => compareConfidence(entryOf(x), entryOf(y)) || byName(entryOf(x), entryOf(y)));
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
