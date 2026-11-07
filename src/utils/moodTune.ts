// Pure view-model helpers for the Tune dialog (MoodTuneDialog.vue): what the
// review screen says about one set of proposed lines.
import type {
  LibraryEntry,
  MoodFlag,
  TuneFlagOutcome,
  TuneLeaveOneOut,
  TuneNameIdea,
  TuneOverlap,
  TuneReview,
} from '@generated/index';

/// Below this many flags, the dialog warns that the result may be shaky.
export const MIN_FLAGS = 5;

/// An overlap with another mood that grows by at least this much is noted.
export const OVERLAP_RISE = 0.1;

type FlagOutcomes = { flags: Pick<TuneFlagOutcome, 'holds' | 'fresh'>[] };

/// Fresh flags (made under the current wording) that end outside their band.
/// Only these ask for the "Accept anyway" confirm; broken stale flags get
/// keep / update / delete instead.
export function brokenFresh(report: FlagOutcomes): number {
  return report.flags.filter((f) => f.fresh && !f.holds).length;
}

/// The Accept button's label: plain "Accept", or the explicit confirm wording
/// when fresh flags would leave their band.
export function acceptLabel(report: FlagOutcomes): string {
  return anywayLabel(brokenFresh(report), 'Accept');
}

function anywayLabel(n: number, plain: string): string {
  if (n === 0) return plain;
  return `Accept anyway: ${n} flagged song${n === 1 ? '' : 's'} leave${n === 1 ? 's' : ''} their band`;
}

/// Broken fresh flags over both halves of a split: like a proposal's, they
/// ask for the "Accept anyway" confirm.
export function splitBrokenFresh(split: { keep: FlagOutcomes; new: FlagOutcomes }): number {
  return brokenFresh(split.keep) + brokenFresh(split.new);
}

/// The split's accept button: "Accept split", or the confirm wording over
/// both halves.
export function splitAcceptLabel(split: { keep: FlagOutcomes; new: FlagOutcomes }): string {
  return anywayLabel(splitBrokenFresh(split), 'Accept split');
}

/// The warning shown beside the flag count when there are few flags.
export function shakyNote(flagCount: number): string | null {
  if (flagCount >= MIN_FLAGS) return null;
  return `Only ${flagCount} flag${flagCount === 1 ? '' : 's'}: the result may be shaky`;
}

/// Noted when the word that best follows the lines' ranking is not the
/// mood's own name. Never renames anything.
export function nameNote(currentName: string, names: Pick<TuneNameIdea, 'word'>[]): string | null {
  const best = names[0]?.word;
  if (best === undefined || best.trim().toLowerCase() === currentName.trim().toLowerCase()) return null;
  return `These words now read more like ${best} than ${currentName}`;
}

/// The leave-one-out result in words, or null for the baseline review (which
/// has no leave-one-out run: 0 of 0).
export function looNote(loo: TuneLeaveOneOut): string | null {
  if (loo.total === 0) return null;
  return `${loo.held} of ${loo.total} flags hold even when not used for tuning`;
}

/// Entry ids of the fresh flags that hold under `report`: accepting it
/// refreshes these to the new wording; every other flag reads as stale.
export function holdingEntryIds(report: Pick<TuneReview, 'flags'>): string[] {
  return report.flags.filter((f) => f.fresh && f.holds).map((f) => f.song.entryId);
}

/// The flagged songs in the order the review shows them: broken fresh flags
/// first (the most important thing on the screen), then broken stale flags,
/// then the ones that hold.
export function orderedFlags(report: Pick<TuneReview, 'flags'>): TuneFlagOutcome[] {
  const rank = (f: TuneFlagOutcome): number => (f.holds ? 2 : f.fresh ? 0 : 1);
  return [...report.flags].sort((a, b) => rank(a) - rank(b));
}

/// Overlaps with other moods that rise noticeably under the proposal.
export function overlapRises(overlap: TuneOverlap[]): TuneOverlap[] {
  // A small epsilon so 0.5 → 0.6 counts despite floating-point error.
  return overlap.filter((o) => o.after - o.before >= OVERLAP_RISE - 1e-9);
}

/// Which songs joined the top list and which dropped out of it.
export function topChanges(report: Pick<TuneReview, 'top' | 'topBefore'>): { joined: Set<string>; dropped: Set<string> } {
  const now = new Set(report.top.map((t) => t.song.entryId));
  const before = new Set(report.topBefore.map((t) => t.song.entryId));
  return {
    joined: new Set([...now].filter((id) => !before.has(id))),
    dropped: new Set([...before].filter((id) => !now.has(id))),
  };
}

/// "Extra words to try", split on commas and new lines, without blanks or
/// repeats.
export function parseExtraWords(text: string): string[] {
  const words = text.split(/[,\n]/).map((w) => w.trim()).filter((w) => w.length > 0);
  return [...new Set(words)];
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

/// What Tune grades a mood against: songs with a flag on it, plus songs with
/// a hand-set level and no flag (the backend reads those as band flags).
export function tuneTargetCount(entries: Pick<LibraryEntry, 'moods'>[], moodId: string): number {
  return entries.filter((e) => e.moods.some((m) => m.mood === moodId && (m.flag !== null || m.user !== null))).length;
}

/// Whether a flag asks for a high bar (Definitely, Understandable, any
/// "at least", or "higher"): such a song dropping out of the top list
/// matters.
export function targetIsHigh(flag: Pick<MoodFlag, 'kind' | 'band' | 'bound'>): boolean {
  if (flag.kind === 'higher') return true;
  if (flag.kind === 'lower') return false;
  return flag.bound === 'at_least' || (flag.bound === 'in' && (flag.band === 'definitely' || flag.band === 'understandable'));
}

/// Each top-list song's 1-based rank before and after (null when outside
/// that list).
export function topRanks(
  report: Pick<TuneReview, 'top' | 'topBefore'>,
): Map<string, { before: number | null; after: number | null }> {
  const ranks = new Map<string, { before: number | null; after: number | null }>();
  report.topBefore.forEach((t, i) => ranks.set(t.song.entryId, { before: i + 1, after: null }));
  report.top.forEach((t, i) => {
    const r = ranks.get(t.song.entryId);
    if (r) r.after = i + 1;
    else ranks.set(t.song.entryId, { before: null, after: i + 1 });
  });
  return ranks;
}

/// A rank change in words: "#12 → #2", "new", "drops out" or "same".
export function rankLabel(rank: { before: number | null; after: number | null } | undefined): string {
  if (!rank) return '';
  if (rank.before === null) return 'new';
  if (rank.after === null) return 'drops out';
  return rank.before === rank.after ? 'same' : `#${rank.before} → #${rank.after}`;
}

/// "Artist / Album" from the file's two parent folders, as the Moods panel
/// previews and Tune's songs show it.
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

/// Whether an accept was refused because the mood's lines changed after
/// Tune ran (the answer is to run Tune again).
export function isChangedSinceTune(error: string): boolean {
  return error.includes('changed since Tune ran');
}
