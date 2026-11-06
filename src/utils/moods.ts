import type { LibraryEntry, MoodId, Role } from '@generated/index';

/// One row of the mood profile: its id (as stored) and the name shown.
export interface MoodInfo {
  id: MoodId;
  name: string;
}

/// The fixed vocabulary in its order: nine moods, then Energy. Lens buttons,
/// profile rows and `LibraryEntry.moods` all follow this order.
export const MOODS: readonly MoodInfo[] = [
  { id: 'peaceful', name: 'Peaceful' },
  { id: 'sorrowful', name: 'Sorrowful' },
  { id: 'wondrous', name: 'Wondrous' },
  { id: 'cheerful', name: 'Cheerful' },
  { id: 'quirky', name: 'Quirky' },
  { id: 'mysterious', name: 'Mysterious' },
  { id: 'grim', name: 'Grim' },
  { id: 'heroic', name: 'Heroic' },
  { id: 'menacing', name: 'Menacing' },
  { id: 'energy', name: 'Energy' },
];

/// The nine moods without Energy (for "the two highest moods").
export const MOODS_ONLY: readonly MoodInfo[] = MOODS.filter((m) => m.id !== 'energy');

/// The five levels a user can set (a 1–5 rating mapped onto 0..1).
export const MOOD_LEVELS: readonly number[] = [0, 0.25, 0.5, 0.75, 1];

const NAMES = new Map<MoodId, string>(MOODS.map((m) => [m.id, m.name]));

export const moodName = (id: MoodId): string => NAMES.get(id) ?? id;

/// Both layers of one mood on one entry (null/null when the entry has no row).
export function moodLayers(entry: LibraryEntry, id: MoodId): { auto: number | null; user: number | null } {
  const v = entry.moods.find((m) => m.mood === id);
  return { auto: v?.auto ?? null, user: v?.user ?? null };
}

/// The value shown for a mood: the level the user set, else the suggestion.
export function moodValue(entry: LibraryEntry, id: MoodId): number | null {
  const { auto, user } = moodLayers(entry, id);
  return user ?? auto;
}

/// The level nearest to `value`.
export function snapLevel(value: number): number {
  let best = MOOD_LEVELS[0];
  for (const level of MOOD_LEVELS) {
    if (Math.abs(level - value) < Math.abs(best - value)) best = level;
  }
  return best;
}

/// The next level above (`step` 1) or below (`step` -1) `value`. A value
/// between two levels moves to the neighbouring level in that direction.
export function stepLevel(value: number | null, step: 1 | -1): number {
  if (value === null) return step > 0 ? MOOD_LEVELS[1] : MOOD_LEVELS[0];
  const eps = 1e-9;
  if (step > 0) return MOOD_LEVELS.find((l) => l > value + eps) ?? MOOD_LEVELS[MOOD_LEVELS.length - 1];
  return [...MOOD_LEVELS].reverse().find((l) => l < value - eps) ?? MOOD_LEVELS[0];
}

/// The entry's highest moods (Energy left out), best first, e.g. for the
/// Review card's readout "Mysterious 0.87 · Menacing 0.74".
export function topMoods(entry: LibraryEntry, n: number): { id: MoodId; name: string; value: number }[] {
  const out: { id: MoodId; name: string; value: number }[] = [];
  for (const m of MOODS_ONLY) {
    const value = moodValue(entry, m.id);
    if (value !== null) out.push({ id: m.id, name: m.name, value });
  }
  return out.sort((a, b) => b.value - a.value).slice(0, n);
}

export const topMoodText = (entry: LibraryEntry, n = 2): string =>
  topMoods(entry, n)
    .map((m) => `${m.name} ${m.value.toFixed(2)}`)
    .join(' · ');

// --- Lenses and filters ---

/// How several selected lenses (or tag chips) combine: `any` ranks by a
/// track's best score among them, `all` by its weakest.
export type MatchMode = 'any' | 'all';

export type RoleChoice = 'all' | Role | 'unset';

export interface MoodFilters {
  /// Selected lenses, in the order they were picked; empty means no lens.
  lenses: MoodId[];
  mode: MatchMode;
  role: RoleChoice;
}

export function emptyMoodFilters(): MoodFilters {
  return { lenses: [], mode: 'any', role: 'all' };
}

/// The entry's score under the selected lenses: its best (any) or weakest
/// (all) value among them. Null when no lens is selected, for ambiance, and
/// for music with no value for a selected mood (`any` uses those it has).
export function lensScore(entry: LibraryEntry, f: Pick<MoodFilters, 'lenses' | 'mode'>): number | null {
  if (f.lenses.length === 0 || entry.entryType !== 'music') return null;
  const values = f.lenses.map((id) => moodValue(entry, id));
  if (f.mode === 'all') {
    if (values.some((v) => v === null)) return null;
    return Math.min(...(values as number[]));
  }
  const present = values.filter((v): v is number => v !== null);
  return present.length === 0 ? null : Math.max(...present);
}

/// True when the entry passes the lens and role filters. Under a lens only
/// music shows (ambiance has no moods); music not scored yet stays, ranked
/// last. "Unset" role means music with no role shown.
export function matchesMoodFilters(entry: LibraryEntry, f: MoodFilters): boolean {
  if (f.lenses.length > 0 && entry.entryType !== 'music') return false;
  if (f.role === 'all') return true;
  if (f.role === 'unset') return entry.entryType === 'music' && entry.role === null;
  return entry.role === f.role;
}

/// Short label for the lens column's header and hover text.
export function lensLabel(f: Pick<MoodFilters, 'lenses' | 'mode'>): string {
  if (f.lenses.length === 0) return '';
  if (f.lenses.length === 1) return moodName(f.lenses[0]);
  const names = f.lenses.map(moodName).join(f.mode === 'any' ? ' or ' : ' and ');
  return f.mode === 'any' ? `Best of ${names}` : `Weakest of ${names}`;
}

/// True when an entry with `tags` passes the selected tag chips: one of them
/// (any) or every one (all). No chip selected passes everything.
export function matchesTagSelection(tags: string[], selected: string[], mode: MatchMode): boolean {
  if (selected.length === 0) return true;
  return mode === 'all' ? selected.every((t) => tags.includes(t)) : selected.some((t) => tags.includes(t));
}

// --- Sorting ---

/// Sort keys for the filter bar. `match` keeps the incoming order (Scry's
/// ranking); `lens` ranks by the lens score, highest first, and is offered
/// while a lens is selected.
export type SortKey = 'match' | 'lens' | 'name' | 'energy';
export type SortDir = 'asc' | 'desc';

export const SORT_LABELS: Record<SortKey, string> = {
  match: 'Best match',
  lens: 'Lens score',
  name: 'Name',
  energy: 'Energy',
};

/// The direction a key starts in when picked: scores high first, names A–Z.
export const DEFAULT_DIR: Record<SortKey, SortDir> = {
  match: 'desc',
  lens: 'desc',
  name: 'asc',
  energy: 'desc',
};

const byName = (a: LibraryEntry, b: LibraryEntry): number => a.name.localeCompare(b.name);

/// Compares two optional numbers in `dir`, with nulls always last.
function compareNullable(a: number | null, b: number | null, dir: SortDir): number {
  if (a === null || b === null) return a === b ? 0 : a === null ? 1 : -1;
  return dir === 'asc' ? a - b : b - a;
}

/// A sorted copy of `items` (stable). `entryOf` picks the library entry from
/// an item, so Scry's rows sort the same way as Library entries. `match`
/// keeps the incoming order (reversed for ascending).
export function sortByKey<T>(
  items: T[],
  key: SortKey,
  dir: SortDir,
  filters: Pick<MoodFilters, 'lenses' | 'mode'>,
  entryOf: (item: T) => LibraryEntry,
): T[] {
  if (key === 'match') return dir === 'desc' ? items : [...items].reverse();
  const out = [...items];
  if (key === 'name') {
    out.sort((x, y) => (dir === 'asc' ? 1 : -1) * byName(entryOf(x), entryOf(y)));
  } else {
    const score = (e: LibraryEntry): number | null => (key === 'lens' ? lensScore(e, filters) : moodValue(e, 'energy'));
    out.sort((x, y) => compareNullable(score(entryOf(x)), score(entryOf(y)), dir) || byName(entryOf(x), entryOf(y)));
  }
  return out;
}

// --- Review queue ---

/// Music not reviewed yet (no mood level set and not approved). This is what
/// the Review tab's badge counts.
export const needsReview = (entry: LibraryEntry): boolean =>
  entry.entryType === 'music' && entry.moodReviewedAt === null;

const DAY_MS = 24 * 60 * 60 * 1000;
const createdMs = (e: LibraryEntry): number => Number(e.createdAt) || 0;
const topScore = (e: LibraryEntry): number => topMoods(e, 1)[0]?.value ?? Infinity;

/// Review order: newest import day first; within a day, the lowest top mood
/// score first (the track the analyzer is least sure fits anywhere), entries
/// with no scores after those; then newest, then name.
export function compareReview(a: LibraryEntry, b: LibraryEntry): number {
  const dayA = Math.floor(createdMs(a) / DAY_MS);
  const dayB = Math.floor(createdMs(b) / DAY_MS);
  if (dayA !== dayB) return dayB - dayA;
  const ta = topScore(a);
  const tb = topScore(b);
  if (ta !== tb) return ta - tb;
  return createdMs(b) - createdMs(a) || byName(a, b);
}
