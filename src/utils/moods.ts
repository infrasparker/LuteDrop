import { shallowRef } from 'vue';
import type { LibraryEntry, MoodCalibrationInfo, MoodId, MoodVocabulary, Role } from '@generated/index';

/// One row of the mood profile: its id (as stored) and the name shown.
export interface MoodInfo {
  id: MoodId;
  name: string;
}

export const ENERGY_ID = 'energy';
const ENERGY: MoodInfo = { id: ENERGY_ID, name: 'Energy' };

/// The app's mood vocabulary in its order: the user's moods, then Energy.
/// It comes from the app state (`AppState.moodVocabulary`, set by the store
/// on every hydrate); nothing here hard-codes a mood. Lens chips, profile
/// rows and `LibraryEntry.moods` all follow this order.
const vocabulary = shallowRef<readonly MoodInfo[]>([ENERGY]);

export function setMoodVocabulary(v: MoodVocabulary): void {
  vocabulary.value = [...v.moods.map((m) => ({ id: m.id, name: m.name })), ENERGY];
}

/// Every row of the profile: the moods, then Energy. Reactive.
export const moodList = (): readonly MoodInfo[] => vocabulary.value;

/// The moods without Energy (for "the two highest moods").
export const moodsOnly = (): readonly MoodInfo[] => vocabulary.value.filter((m) => m.id !== ENERGY_ID);

/// The five levels a user can set (a 1–5 rating mapped onto 0..1).
export const MOOD_LEVELS: readonly number[] = [0, 0.25, 0.5, 0.75, 1];

export const moodName = (id: MoodId): string => vocabulary.value.find((m) => m.id === id)?.name ?? id;

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

// --- Lenses and filters ---

/// A tag chip's state. Tag chips cycle off → include → exclude → off. (Mood
/// and Energy chips are plain on/off toggles; see `MoodFilters`.)
export type ChipState = 'off' | 'include' | 'exclude';

export function nextChipState(state: ChipState): ChipState {
  return state === 'off' ? 'include' : state === 'include' ? 'exclude' : 'off';
}

/// Included and excluded tag chips.
export interface ChipSelection {
  include: string[];
  exclude: string[];
}

export const emptyChipSelection = (): ChipSelection => ({ include: [], exclude: [] });

export function chipState(sel: ChipSelection, id: string): ChipState {
  if (sel.include.includes(id)) return 'include';
  if (sel.exclude.includes(id)) return 'exclude';
  return 'off';
}

/// The selection with `id` moved to its next state.
export function cycleChip(sel: ChipSelection, id: string): ChipSelection {
  const next = nextChipState(chipState(sel, id));
  const include = sel.include.filter((x) => x !== id);
  const exclude = sel.exclude.filter((x) => x !== id);
  if (next === 'include') include.push(id);
  if (next === 'exclude') exclude.push(id);
  return { include, exclude };
}

/// `list` with `id` added at the end (when absent) or removed (when present),
/// for the on/off chips.
export function toggleIn<T>(list: readonly T[], id: T): T[] {
  return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
}

export type RoleChoice = 'all' | Role;

/// The three Energy bands of the lens row. Each is an on/off chip; a track
/// passes when its Energy is in any band that is on.
export type EnergyBand = 'low' | 'mid' | 'high';

/// Low is below this; Mid runs from here to `ENERGY_HIGH_ABOVE`, both ends
/// included; High is above that. The edges sit halfway between the five
/// levels, so 0 and 0.25 are Low, 0.5 is Mid, 0.75 and 1 are High.
export const ENERGY_LOW_BELOW = 0.375;
export const ENERGY_HIGH_ABOVE = 0.625;

export const ENERGY_BANDS: readonly { id: EnergyBand; name: string; hint: string }[] = [
  { id: 'low', name: 'Low', hint: `Energy below ${ENERGY_LOW_BELOW}` },
  { id: 'mid', name: 'Mid', hint: `Energy ${ENERGY_LOW_BELOW} to ${ENERGY_HIGH_ABOVE}` },
  { id: 'high', name: 'High', hint: `Energy above ${ENERGY_HIGH_ABOVE}` },
];

/// The band an Energy value falls in.
export function energyBand(value: number): EnergyBand {
  if (value < ENERGY_LOW_BELOW) return 'low';
  if (value > ENERGY_HIGH_ABOVE) return 'high';
  return 'mid';
}

export interface MoodFilters {
  /// Mood lenses that are on, in the order they were picked. They rank the
  /// list by the weakest of their scores.
  lenses: MoodId[];
  /// Energy bands that are on. None on means no Energy filter.
  bands: EnergyBand[];
  role: RoleChoice;
}

export function emptyMoodFilters(): MoodFilters {
  return { lenses: [], bands: [], role: 'all' };
}

/// True while at least one mood lens is on (the list is ranked).
export const lensOn = (f: Pick<MoodFilters, 'lenses'>): boolean => f.lenses.length > 0;

/// The entry's score under the lenses that are on: the weakest of them, so a
/// track ranks high only when it is high in every one. Null when no lens is
/// on, for ambiance, and for music missing a value for one of the lenses.
/// Energy plays no part.
export function lensScore(entry: LibraryEntry, f: Pick<MoodFilters, 'lenses'>): number | null {
  const ids = f.lenses;
  if (ids.length === 0 || entry.entryType !== 'music') return null;
  const values = ids.map((id) => moodValue(entry, id));
  if (values.some((v) => v === null)) return null;
  return Math.min(...(values as number[]));
}

/// True when the entry's Energy (set by you, else suggested) is in one of
/// `bands`. With no band on, everything passes; with any band on, an entry
/// with no Energy value does not.
export function matchesEnergyBands(entry: LibraryEntry, bands: readonly EnergyBand[]): boolean {
  if (bands.length === 0) return true;
  const v = moodValue(entry, ENERGY_ID);
  return v !== null && bands.includes(energyBand(v));
}

/// True when the entry passes the lens, Energy and role filters. With any
/// lens or band on only music shows (ambiance has no moods). Music not scored
/// in a lens's mood stays, ranked last. A role choice shows music with that
/// role (ambiance has none).
export function matchesMoodFilters(entry: LibraryEntry, f: MoodFilters): boolean {
  const anyOn = f.lenses.length > 0 || f.bands.length > 0;
  if (anyOn && entry.entryType !== 'music') return false;
  if (!matchesEnergyBands(entry, f.bands)) return false;
  if (f.role === 'all') return true;
  return entry.role === f.role;
}

/// The title of the column that shows the lens score, in the Library and Scry.
export const LENS_COLUMN_TITLE = 'Match';

/// Hover text for the lens column: what the score is, e.g. "Heroic" or
/// "weakest of Heroic, Grim".
export function lensLabel(f: Pick<MoodFilters, 'lenses'>): string {
  const names = f.lenses.map(moodName);
  return names.length > 1 ? `weakest of ${names.join(', ')}` : (names[0] ?? '');
}

/// True when an entry with `tags` passes the tag chips: it has at least one
/// included tag (when any is included) and none of the excluded ones.
export function matchesTagSelection(tags: string[], sel: ChipSelection): boolean {
  if (sel.include.length > 0 && !sel.include.some((t) => tags.includes(t))) return false;
  return !sel.exclude.some((t) => tags.includes(t));
}

// --- Sorting ---

/// Sort keys, picked from the list's column headers. `match` keeps the
/// incoming order (Scry's search ranking, its "Search" column); `lens` ranks
/// by the lens score (the "Match" column) and is there while a lens is on. `status`, `entryType`, `role` and
/// `duration` are the Library's other sortable columns.
export type SortKey = 'match' | 'lens' | 'name' | 'energy' | 'status' | 'entryType' | 'role' | 'duration';
export type SortDir = 'asc' | 'desc';

/// The direction a key starts in when picked: scores high first, names A–Z,
/// missing files first (status), shortest first (duration).
export const DEFAULT_DIR: Record<SortKey, SortDir> = {
  match: 'desc',
  lens: 'desc',
  name: 'asc',
  energy: 'desc',
  status: 'desc',
  entryType: 'asc',
  role: 'asc',
  duration: 'asc',
};

/// False for a header that does not sort in the current state: Energy while
/// a mood lens is on, so the ranking never depends on Energy then.
export function canSortBy(key: SortKey, f: Pick<MoodFilters, 'lenses'>): boolean {
  return !(key === 'energy' && lensOn(f));
}

/// The sort after a header click: a new key starts in its default
/// direction, the current key flips.
export function clickSort(current: { key: SortKey; dir: SortDir }, key: SortKey): { key: SortKey; dir: SortDir } {
  if (current.key !== key) return { key, dir: DEFAULT_DIR[key] };
  return { key, dir: current.dir === 'asc' ? 'desc' : 'asc' };
}

/// The sort after the lenses changed: turning on the first lens ranks by it
/// (highest first); dropping the last one while ranking by it goes back to
/// the view's own default (`fallback`: Name in the Library, Match in Scry).
export function sortForLens(
  current: { key: SortKey; dir: SortDir },
  wasOn: boolean,
  isOn: boolean,
  fallback: SortKey,
): { key: SortKey; dir: SortDir } {
  if (isOn && !wasOn) return { key: 'lens', dir: 'desc' };
  if (!isOn && current.key === 'lens') return { key: fallback, dir: DEFAULT_DIR[fallback] };
  return current;
}

const byName = (a: LibraryEntry, b: LibraryEntry): number => a.name.localeCompare(b.name);

/// Compares two optional numbers in `dir`, with nulls always last.
function compareNullable(a: number | null, b: number | null, dir: SortDir): number {
  if (a === null || b === null) return a === b ? 0 : a === null ? 1 : -1;
  return dir === 'asc' ? a - b : b - a;
}

/// The number a key sorts by (null sorts last in either direction), for the
/// keys that are not text. `isMissing` tells a missing file (status: 1).
function sortValue(
  e: LibraryEntry,
  key: Exclude<SortKey, 'match' | 'name'>,
  filters: Pick<MoodFilters, 'lenses'>,
  isMissing: (e: LibraryEntry) => boolean,
): number | null {
  switch (key) {
    case 'lens':
      return lensScore(e, filters);
    case 'energy':
      return moodValue(e, 'energy');
    case 'status':
      return isMissing(e) ? 1 : 0;
    case 'entryType':
      return e.entryType === 'music' ? 0 : 1;
    case 'role':
      return e.role === null ? null : e.role === 'combat' ? 0 : 1;
    case 'duration':
      return e.duration;
  }
}

/// The suggestion behind a lens or Energy value: the weakest suggested score
/// over the lenses that are on, or the suggested Energy. Levels set by hand are
/// coarse (five steps), so tracks set to the same level tie; this orders them
/// within the level by what the analyzer thinks, without changing the value
/// shown. For a track with no level set it equals the value itself.
function suggestionBehind(e: LibraryEntry, key: SortKey, filters: Pick<MoodFilters, 'lenses'>): number | null {
  const ids = key === 'lens' ? filters.lenses : key === 'energy' ? [ENERGY_ID] : [];
  if (ids.length === 0) return null;
  const autos = ids.map((id) => moodLayers(e, id).auto);
  if (autos.some((a) => a === null)) return null;
  return Math.min(...(autos as number[]));
}

/// A sorted copy of `items` (stable). `entryOf` picks the library entry from
/// an item, so Scry's rows sort the same way as Library entries. `match`
/// keeps the incoming order (reversed for ascending). Lens and Energy ties go
/// by the suggestion behind the value (`suggestionBehind`), then every tie
/// goes by name. Energy is not a sort while a lens is on (`canSortBy`); asked
/// for then, it ranks by the lens score instead.
export function sortByKey<T>(
  items: T[],
  key: SortKey,
  dir: SortDir,
  filters: Pick<MoodFilters, 'lenses'>,
  entryOf: (item: T) => LibraryEntry,
  isMissing: (e: LibraryEntry) => boolean = () => false,
): T[] {
  if (!canSortBy(key, filters)) key = 'lens';
  if (key === 'match') return dir === 'desc' ? items : [...items].reverse();
  const out = [...items];
  if (key === 'name') {
    out.sort((x, y) => (dir === 'asc' ? 1 : -1) * byName(entryOf(x), entryOf(y)));
  } else {
    const score = (e: LibraryEntry): number | null => sortValue(e, key, filters, isMissing);
    const behind = (e: LibraryEntry): number | null => suggestionBehind(e, key, filters);
    out.sort((x, y) => {
      const a = entryOf(x);
      const b = entryOf(y);
      return compareNullable(score(a), score(b), dir) || compareNullable(behind(a), behind(b), dir) || byName(a, b);
    });
  }
  return out;
}

/** The line under "Calibration" in Settings → Moods: which statistics the
 * bars are measured against. */
export function calibrationLabel(info: MoodCalibrationInfo | null): string {
  if (!info) return "Not calibrated yet. The next analysis run calibrates, against the analyzer's reference set, or once it covers 50 music tracks of your library.";
  if (info.source === 'reference') {
    // The "(Kevin MacLeod, CC BY)" attribution is for the shipped set
    // incompetech-300-v1. Change it together with the set if a new one ships.
    return `Calibrated against the reference set: ${info.tracks} soundtrack tracks (Kevin MacLeod, CC BY), ${info.referenceSet}`;
  }
  const date = new Date(Number(info.calibratedAt)).toLocaleString();
  return `Calibrated against your library: ${date}, ${info.tracks} tracks`;
}
