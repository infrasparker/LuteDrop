// Pure view-model helpers for the Moods page (MoodsView.vue and the
// components under components/moods/): band words, the flags table's cell
// states and order, the Top 15 merge, column verdicts and chips, and the
// confirm's copy.
import type {
  MoodFlag,
  MoodValue,
  TuneFlagOutcome,
  TuneLeaveOneOut,
  TuneMovedSong,
  TuneNameIdea,
  TuneOutRank,
  TuneOverlap,
  TuneReview,
  TuneSplitReview,
  TuneTopSong,
} from '@generated/index';
import { MIN_FLAGS, overlapRises } from './moodTune.ts';

type FlagLike = Pick<MoodFlag, 'kind' | 'band' | 'bound' | 'barAtFlag' | 'stale' | 'staleReason'>;

/// Most version columns a mood can have open (besides Current and Before).
export const MAX_PAGE_VERSIONS = 6;

/// Leave-one-out below this share of flags held reads as shaky.
export const SHAKY_LOO = 0.7;

/// How a version column's lines came about.
export type VersionSource = 'typed' | 'arranged' | 'suggested' | 'before';

/// What a version column shows (MoodVersionColumn.vue).
export interface ColumnModel {
  key: number | 'current';
  name: string;
  kind: 'current' | 'version' | 'before' | 'split';
  source: VersionSource | null;
  text: string;
  review: TuneReview | null;
  /// The verdict was made from other lines (or another Current): greyed.
  stale: boolean;
  evaluating: boolean;
  running: 'arrange' | 'suggest' | null;
  error: string | null;
  regressionCount: number;
  /// A suggested column whose search found a split.
  splitOffer: boolean;
  split: TuneSplitReview | null;
}

// --- Band words ---

const LONG: Record<string, string> = { definitely: 'Definitely', understandable: 'Understandable', stretch: 'Stretch', no: 'No' };
const SHORT: Record<string, string> = { definitely: 'Def', understandable: 'Und', stretch: 'Str', no: 'No' };

/// What a flag (else a hand-set level) asks, in the band words: "Definitely",
/// "Understandable", "Stretch", "No", "Stretch or more", "Stretch or less",
/// "Higher", "Lower", or "Set by you" for a level set by hand. At least
/// Definitely / Understandable read as just the band. `short` gives the
/// narrow layout's abbreviations (Def, Und, Str, ≤Str, ≥Str).
export function bandWord(flag: Pick<MoodFlag, 'kind' | 'band' | 'bound'> | null, level: number | null, short = false): string {
  if (!flag) return level === null ? '' : short ? 'Set' : 'Set by you';
  if (flag.kind === 'higher') return 'Higher';
  if (flag.kind === 'lower') return 'Lower';
  const band = flag.band ?? '';
  const word = (short ? SHORT : LONG)[band] ?? '';
  const top = band === 'definitely' || band === 'understandable';
  if (flag.bound === 'at_least' && !top) return short ? `≥${word}` : `${word} or more`;
  if (flag.bound === 'at_most' && band !== 'no') return short ? `≤${word}` : `${word} or less`;
  return word;
}

// --- What a flag asks of the bar ---

const FLOOR: Record<string, number | null> = { definitely: 0.9, understandable: 0.75, stretch: 0.5, no: null };
const CEILING: Record<string, number | null> = { definitely: null, understandable: 0.9, stretch: 0.75, no: 0.5 };
const DIRECTION_MARGIN = 0.1;

export interface Bounds {
  min: number | null;
  max: number | null;
}

/// The band a hand-set level stands for, as the tuner reads it.
function levelBand(level: number): string {
  if (level >= 1) return 'definitely';
  if (level >= 0.75) return 'understandable';
  if (level >= 0.5) return 'stretch';
  return 'no';
}

/// The floor and ceiling a flag (else a hand-set level) puts on the bar,
/// as the tuner reads it (`tune.rs` `flag_target`): a direction flag aims
/// 0.1 past the bar it was made at, or past `bar` (the current one) when it
/// is stale by calibration. Null when nothing is asked.
export function flagBounds(flag: FlagLike | null, level: number | null, bar: number | null): Bounds | null {
  if (!flag) {
    if (level === null) return null;
    const band = levelBand(level);
    const atLeast = band === 'definitely' || band === 'understandable';
    return { min: FLOOR[band], max: atLeast ? null : CEILING[band] };
  }
  if (flag.kind === 'higher' || flag.kind === 'lower') {
    const from = flag.staleReason === 'calibration' ? (bar ?? flag.barAtFlag) : flag.barAtFlag;
    if (from === null) return null;
    return flag.kind === 'higher'
      ? { min: Math.min(1, from + DIRECTION_MARGIN), max: null }
      : { min: null, max: Math.max(0, from - DIRECTION_MARGIN) };
  }
  const band = flag.band ?? '';
  if (flag.bound === 'at_least') return { min: FLOOR[band] ?? null, max: null };
  if (flag.bound === 'at_most') return { min: null, max: CEILING[band] ?? null };
  return { min: FLOOR[band] ?? null, max: CEILING[band] ?? null };
}

/// How far `v` is outside the bounds (0 inside).
export function bandDistance(v: number, b: Bounds): number {
  return Math.max(0, (b.min ?? -Infinity) - v) + Math.max(0, v - (b.max ?? Infinity));
}

/// One cell of the flags table. `holds`: inside the band. `closer`:
/// outside, but nearer than under Current. `worse`: outside and no nearer,
/// or newly broken (Current held); `plain` when broken under Current
/// already and no farther. The Current column (`current` null) is
/// `holds` or `plain`, never an alarm.
export type CellState = 'holds' | 'closer' | 'worse' | 'broken';

export function cellState(
  cell: { holds: boolean; after: number },
  current: { holds: boolean; after: number } | null,
  bounds: Bounds | null,
): CellState {
  if (cell.holds) return 'holds';
  if (current === null) return 'broken';
  if (current.holds) return 'worse';
  if (!bounds) return 'worse';
  const now = bandDistance(cell.after, bounds);
  const before = bandDistance(current.after, bounds);
  if (now < before - 1e-9) return 'closer';
  // Broken under Current already and no farther: still broken, no worse.
  return now > before + 1e-9 ? 'worse' : 'broken';
}

// --- Regressions ---

type Outcomes = { flags: TuneFlagOutcome[] };

/// One thing Use would break: a flag that holds now and not under the
/// version, or (`leftTop`) a flagged song that still holds its band but
/// leaves the top list while asking for a high bar.
export interface Regression {
  outcome: TuneFlagOutcome;
  leftTop: boolean;
}

/// What Use would break, against Current (`baseline`): fresh flags that hold
/// under Current and not under `review`, then held songs that ask for a
/// high bar (`highIds`) and leave the top list. A song whose flag is already
/// broken under Current never counts. Each song once.
export function regressions(
  review: Outcomes & Partial<Pick<TuneReview, 'top' | 'topBefore'>>,
  baseline: Outcomes,
  highIds: Set<string>,
): Regression[] {
  const heldBefore = new Set(baseline.flags.filter((f) => f.holds).map((f) => f.song.entryId));
  const out: Regression[] = review.flags
    .filter((f) => f.fresh && !f.holds && heldBefore.has(f.song.entryId))
    .map((outcome) => ({ outcome, leftTop: false }));
  const seen = new Set(out.map((r) => r.outcome.song.entryId));
  if (review.top && review.topBefore) {
    const { out: left } = mergeTop(review.top, review.topBefore);
    for (const o of left) {
      if (!highIds.has(o.entryId) || seen.has(o.entryId) || !heldBefore.has(o.entryId)) continue;
      const f = review.flags.find((x) => x.song.entryId === o.entryId);
      if (f) {
        out.push({ outcome: f, leftTop: true });
        seen.add(o.entryId);
      }
    }
  }
  return out;
}

/// The bar each fresh flag that holds under `review` reaches, by entry id
/// (Use sends it so held higher / lower flags become band flags).
export function reachedBars(review: Outcomes): Record<string, number> {
  return Object.fromEntries(review.flags.filter((f) => f.fresh && f.holds).map((f) => [f.song.entryId, f.after]));
}

export const confirmTitle = (n: number): string => `Break ${n} flag${n === 1 ? '' : 's'}?`;

const fmt = (v: number): string => v.toFixed(2);

export const confirmLine = (name: string, before: number, after: number, band: string, leftTop = false): string =>
  `${name}: ${fmt(before)} → ${fmt(after)} (${band}${leftTop ? ', out of top 15' : ''})`;

export const useLabel = (regressionCount: number): string => (regressionCount > 0 ? 'Use anyway' : 'Use');

export const breaksLabel = (n: number): string => `${n} break${n === 1 ? '' : 's'}`;

// --- Verdict and chips ---

/// A column's verdict: "{held}/{total} hold · {n} songs ({±d})"; with no
/// flags only the songs show. Current shows "{held}/{total} hold · {b}
/// broken" (its song count is in the page header), or nothing without flags.
export function verdictText(r: Outcomes & Pick<TuneReview, 'firesBefore' | 'firesAfter'>, isCurrent: boolean): string {
  const holding = r.flags.filter((f) => f.holds).length;
  const held = r.flags.length === 0 ? '' : `${holding}/${r.flags.length} hold`;
  if (isCurrent) {
    const broken = r.flags.length - holding;
    return broken > 0 ? `${held} · ${broken} broken` : held;
  }
  const n = r.firesAfter;
  const d = r.firesAfter - r.firesBefore;
  const songs = `${n} song${n === 1 ? '' : 's'} (${d > 0 ? `+${d}` : d < 0 ? `−${-d}` : '±0'})`;
  return held ? `${held} · ${songs}` : songs;
}

export interface ColumnChips {
  shaky: boolean;
  overlaps: TuneOverlap[];
  readsLike: string | null;
}

/// The chips under a column's verdict: Shaky (1 to MIN_FLAGS - 1 flags, or
/// leave-one-out under SHAKY_LOO when known; never with no flags), Overlaps
/// (another mood's overlap rose by OVERLAP_RISE or more), Reads like (the
/// top name idea is another mood's name: that mood, as named).
export function columnChips(
  r: Outcomes & { loo: TuneLeaveOneOut; overlap: TuneOverlap[]; names: TuneNameIdea[] },
  moodName: string,
  otherMoodNames: string[],
): ColumnChips {
  const looWeak = r.loo.total > 0 && r.loo.held / r.loo.total < SHAKY_LOO;
  const n = r.flags.length;
  const top = r.names[0]?.word.trim().toLowerCase() ?? '';
  const other = top && top !== moodName.trim().toLowerCase()
    ? (otherMoodNames.find((m) => m.trim().toLowerCase() === top) ?? null)
    : null;
  return {
    shaky: n > 0 && (n < MIN_FLAGS || looWeak),
    overlaps: overlapRises(r.overlap),
    readsLike: other,
  };
}

/// The flags table abbreviates band words when narrow or with three or
/// more version columns.
export const useShortBands = (narrow: boolean, versionCount: number): boolean => narrow || versionCount >= 3;

// --- Flags table ---

export interface CellModel {
  state: CellState;
  value: number | null;
  regression: boolean;
}

export interface RowModel {
  entryId: string;
  focus: boolean;
  /// Current first, then each version column.
  cells: CellModel[];
}

/// Values closer than this count as unchanged.
const CHANGE_EPS = 0.005;

function rowRank(r: RowModel): number {
  if (r.focus) return 0;
  if (r.cells.some((c) => c.regression)) return 1;
  if (r.cells.some((c) => c.state !== 'holds')) return 2;
  const now = r.cells[0]?.value ?? null;
  const changed = r.cells.slice(1).some((c) => c.value !== null && now !== null && Math.abs(c.value - now) >= CHANGE_EPS);
  return changed ? 3 : 4;
}

/// The flags table's order: the focus flag, then regressed, broken,
/// changed and holding rows (stable within each). Rows that hold and do not
/// change in any column collapse (`collapsed`) unless `expanded`.
export function sortAndCollapse<R extends RowModel>(rows: R[], expanded: boolean): { shown: R[]; collapsed: R[] } {
  const ranked = rows.map((r, i) => ({ r, i, k: rowRank(r) })).sort((a, b) => a.k - b.k || a.i - b.i);
  // With Current the only column there is nothing to compare: show every row.
  if (rows.every((r) => r.cells.length <= 1)) return { shown: ranked.map((x) => x.r), collapsed: [] };
  const shown = ranked.filter((x) => expanded || x.k < 4).map((x) => x.r);
  const collapsed = ranked.filter((x) => x.k === 4).map((x) => x.r);
  return { shown, collapsed };
}

// --- Top 15 ---

export interface TopRow {
  entryId: string;
  name: string;
  rank: number;
  bar: number;
  /// Places moved up (negative: down); null for a song new to the list.
  move: number | null;
  isNew: boolean;
}

export interface OutRow {
  entryId: string;
  name: string;
  oldRank: number;
  /// Its rank in the selected column's whole ranking, when the review says.
  newRank: number | null;
  bar: number;
}

/// The selected column's top list against Current's: each song's rank and
/// move, the songs that left (`out`), and whether the two are the same.
export function mergeTop(
  top: TuneTopSong[],
  topBefore: TuneTopSong[],
  outRanks: TuneOutRank[] = [],
): { rows: TopRow[]; out: OutRow[]; same: boolean } {
  const newRank = new Map(outRanks.map((o) => [o.song.entryId, o.rank]));
  const before = new Map(topBefore.map((t, i) => [t.song.entryId, i + 1]));
  const now = new Set(top.map((t) => t.song.entryId));
  const rows = top.map((t, i) => {
    const old = before.get(t.song.entryId);
    return {
      entryId: t.song.entryId,
      name: t.song.name,
      rank: i + 1,
      bar: t.bar,
      move: old === undefined ? null : old - (i + 1),
      isNew: old === undefined,
    };
  });
  const out = topBefore
    .map((t, i) => ({ entryId: t.song.entryId, name: t.song.name, oldRank: i + 1, newRank: newRank.get(t.song.entryId) ?? null, bar: t.bar }))
    .filter((o) => !now.has(o.entryId));
  const same = top.length === topBefore.length && top.every((t, i) => t.song.entryId === topBefore[i].song.entryId);
  return { rows, out, same };
}

/// "3 → 22", or "3 → out" when the new rank is not known.
export const outRankLabel = (o: Pick<OutRow, 'oldRank' | 'newRank'>): string =>
  `${o.oldRank} → ${o.newRank ?? 'out'}`;

/// The Top panel's title: "Top 15 · v4 · 1 new · 1 out" (counts only when
/// not Current and not zero).
export function topTitle(name: string, merged: { rows: TopRow[]; out: OutRow[] }, isCurrent: boolean): string {
  const parts = [`Top 15`, name];
  if (!isCurrent) {
    const added = merged.rows.filter((r) => r.isNew).length;
    if (added > 0) parts.push(`${added} new`);
    if (merged.out.length > 0) parts.push(`${merged.out.length} out`);
  }
  return parts.join(' · ');
}

/// "↑2", "↓3", or '' for no move.
export function rankMove(move: number | null): string {
  if (move === null || move === 0) return '';
  return move > 0 ? `↑${move}` : `↓${-move}`;
}

/// The unflagged songs that moved most: five up, five down.
export function movedMost(r: Pick<TuneReview, 'risers' | 'fallers'>, n = 5): { up: TuneMovedSong[]; down: TuneMovedSong[] } {
  return { up: r.risers.slice(0, n), down: r.fallers.slice(0, n) };
}

/// The song's strongest mood other than `moodId` (and Energy), by the value
/// shown (the level set by hand, else the suggestion); null when none.
export function strongestOtherMood(moods: Pick<MoodValue, 'mood' | 'auto' | 'user'>[], moodId: string): string | null {
  let best: { id: string; v: number } | null = null;
  for (const m of moods) {
    if (m.mood === moodId || m.mood === 'energy') continue;
    const v = m.user ?? m.auto;
    if (v !== null && (best === null || v > best.v)) best = { id: m.mood, v };
  }
  return best?.id ?? null;
}

// --- Lines ---

/// One prompt per line, trimmed, blanks dropped.
export function parseLines(text: string): string[] {
  return text.split('\n').map((l) => l.trim()).filter((l) => l.length > 0);
}

/// Which of `lines` are not in Current, and Current's lines missing from them.
export function lineDiff(lines: string[], current: string[]): { added: boolean[]; missing: string[] } {
  const have = new Set(lines);
  const cur = new Set(current);
  return { added: lines.map((l) => !cur.has(l)), missing: current.filter((l) => !have.has(l)) };
}

export const minusLinesLabel = (n: number): string => `−${n} line${n === 1 ? '' : 's'}`;

/// What "Never suggest" adds for a line: each comma-separated part, as a
/// word or phrase (the analyzer leaves out any unit holding one whole).
export function neverSuggestWords(line: string): string[] {
  return line.split(',').map((w) => w.trim()).filter((w) => w.length > 0);
}

/// The next free version name: v2, v3, … (Current is the first version).
export function nextVersionName(taken: string[]): string {
  const used = new Set(taken);
  let i = 2;
  while (used.has(`v${i}`)) i++;
  return `v${i}`;
}
