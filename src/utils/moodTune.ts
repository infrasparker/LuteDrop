// Pure view-model helpers for Tune on the Moods page (MoodsView.vue): what a
// review screen says about one set of proposed lines.
import type {
  LibraryEntry,
  MoodFlag,
  TuneNameIdea,
  TuneOverlap,
  TuneReview,
} from '@generated/index';

/// Below this many flags, a column reads as shaky.
export const MIN_FLAGS = 5;

/// An overlap with another mood that grows by at least this much is noted.
export const OVERLAP_RISE = 0.1;

/// Entry ids of the fresh flags that hold under `report`: accepting it
/// refreshes these to the new wording; every other flag reads as stale.
export function holdingEntryIds(report: Pick<TuneReview, 'flags'>): string[] {
  return report.flags.filter((f) => f.fresh && f.holds).map((f) => f.song.entryId);
}

/// Overlaps with other moods that rise noticeably under the proposal.
export function overlapRises(overlap: TuneOverlap[]): TuneOverlap[] {
  // A small epsilon so 0.5 → 0.6 counts despite floating-point error.
  return overlap.filter((o) => o.after - o.before >= OVERLAP_RISE - 1e-9);
}

/// Most lines in one version, and characters in one line (the analyzer's limits,
/// as for a mood's prompts).
export const MAX_VERSION_LINES = 16;
export const MAX_LINE_CHARS = 300;

/// What keeps the versions from being compared, in plain words, or null.
export function versionProblem(texts: string[]): string | null {
  for (const [i, lines] of texts.map((t) => t.split('\n').map((l) => l.trim()).filter((l) => l.length > 0)).entries()) {
    if (lines.length > MAX_VERSION_LINES) {
      return `Version ${i + 1} has ${lines.length} lines; a version can have at most ${MAX_VERSION_LINES}.`;
    }
    if (lines.some((l) => l.length > MAX_LINE_CHARS)) {
      return `Version ${i + 1} has a line over ${MAX_LINE_CHARS} characters; shorten it.`;
    }
  }
  return null;
}

/// The songs with a flag on `moodId`, for the Flags list and the flag count.
export function flaggedEntries<E extends Pick<LibraryEntry, 'id' | 'moods'>>(
  entries: E[],
  moodId: string,
): { entry: E; flag: MoodFlag }[] {
  const out: { entry: E; flag: MoodFlag }[] = [];
  for (const entry of entries) {
    const flag = entry.moods.find((m) => m.mood === moodId)?.flag;
    if (flag) out.push({ entry, flag });
  }
  return out;
}

/// The ids of the songs `tuneTargetCount` counts.
export function tuneTargetIds(entries: Pick<LibraryEntry, 'id' | 'moods'>[], moodId: string): Set<string> {
  return new Set(
    entries
      .filter((e) => e.moods.some((m) => m.mood === moodId && (m.flag !== null || m.user !== null)))
      .map((e) => e.id),
  );
}

/// Whether a song's flag, else its hand-set level, asks for a high bar. Tune
/// reads a level of 0.75 or 1 as at least Understandable / Definitely.
export function reviewTargetIsHigh(flag: Pick<MoodFlag, 'kind' | 'band' | 'bound'> | null, level: number | null): boolean {
  if (flag) return targetIsHigh(flag);
  return level !== null && level >= 0.75;
}

/// Whether a flag asks for a high bar (Definitely, Understandable, any
/// "at least", or "higher"): such a song dropping out of the top list
/// matters.
export function targetIsHigh(flag: Pick<MoodFlag, 'kind' | 'band' | 'bound'>): boolean {
  if (flag.kind === 'higher') return true;
  if (flag.kind === 'lower') return false;
  return flag.bound === 'at_least' || (flag.bound === 'in' && (flag.band === 'definitely' || flag.band === 'understandable'));
}

/// "Artist / Album" from the file's two parent folders, as the Moods page's
/// song tooltips show it.
export function albumOf(filePath: string): string {
  const parts = filePath.split(/[\\/]/).filter((p) => p.length > 0 && !/^[A-Za-z]:$/.test(p));
  return [parts[parts.length - 3], parts[parts.length - 2]].filter(Boolean).join(' / ');
}

/// The name a split's new mood starts with: its top name suggestion,
/// capitalised like the shipped mood names.
export function splitName(names: Pick<TuneNameIdea, 'word'>[]): string {
  const word = names[0]?.word.trim() ?? '';
  return word.charAt(0).toUpperCase() + word.slice(1);
}
