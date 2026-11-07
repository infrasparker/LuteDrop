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
  TuneOverlap,
  TuneReview,
  TuneTopSong,
} from '@generated/index';
import { MIN_FLAGS, nameNote, overlapRises } from './moodTune.ts';

type FlagLike = Pick<MoodFlag, 'kind' | 'band' | 'bound' | 'barAtFlag' | 'stale' | 'staleReason'>;

/// Most version columns a mood can have open (besides Current and Before).
export const MAX_PAGE_VERSIONS = 6;

/// Leave-one-out below this share of flags held reads as shaky.
export const SHAKY_LOO = 0.7;

/// How a version column's lines came about.
export type VersionSource = 'typed' | 'arranged' | 'suggested' | 'before';

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
/// or newly broken (Current held). The Current column (`current` null) is
/// `holds` or `plain`, never an alarm.
export type CellState = 'holds' | 'closer' | 'worse' | 'plain';

export function cellState(
  cell: { holds: boolean; after: number },
  current: { holds: boolean; after: number } | null,
  bounds: Bounds | null,
): CellState {
  if (cell.holds) return 'holds';
  if (current === null) return 'plain';
  if (current.holds) return 'worse';
  if (bounds && bandDistance(cell.after, bounds) < bandDistance(current.after, bounds) - 1e-9) return 'closer';
  return 'worse';
}

// --- Regressions ---

type Outcomes = { flags: TuneFlagOutcome[] };

/// What Use would break, against Current (`baseline`): fresh flags that hold
/// under Current and not under `review`, then flagged songs that ask for a
/// high bar (`highIds`) and leave the top list. Flags already broken under
/// Current never count. Each song once, as its flag outcome in `review`.
export function regressions(
  review: Outcomes & Partial<Pick<TuneReview, 'top' | 'topBefore'>>,
  baseline: Outcomes,
  highIds: Set<string>,
): TuneFlagOutcome[] {
  const heldBefore = new Set(baseline.flags.filter((f) => f.holds).map((f) => f.song.entryId));
  const out: TuneFlagOutcome[] = review.flags.filter((f) => f.fresh && !f.holds && heldBefore.has(f.song.entryId));
  const seen = new Set(out.map((f) => f.song.entryId));
  if (review.top && review.topBefore) {
    const { out: left } = mergeTop(review.top, review.topBefore);
    for (const o of left) {
      if (!highIds.has(o.entryId) || seen.has(o.entryId)) continue;
      const f = review.flags.find((x) => x.song.entryId === o.entryId);
      if (f) {
        out.push(f);
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

export const confirmLine = (name: string, before: number, after: number, band: string): string =>
  `${name}: ${fmt(before)} → ${fmt(after)} (${band})`;

export const useLabel = (regressionCount: number): string => (regressionCount > 0 ? 'Use anyway' : 'Use');

export const breaksLabel = (n: number): string => `${n} break${n === 1 ? '' : 's'}`;

// --- Verdict and chips ---

/// A column's verdict: "{held}/{total} · {n} songs ({±d})". Current shows
/// its own song count with no change; with no flags only the songs show.
export function verdictText(r: Outcomes & Pick<TuneReview, 'firesBefore' | 'firesAfter'>, isCurrent: boolean): string {
  const n = isCurrent ? r.firesBefore : r.firesAfter;
  let songs = `${n} song${n === 1 ? '' : 's'}`;
  if (!isCurrent) {
    const d = r.firesAfter - r.firesBefore;
    songs += ` (${d > 0 ? `+${d}` : d < 0 ? `−${-d}` : '±0'})`;
  }
  if (r.flags.length === 0) return songs;
  return `${r.flags.filter((f) => f.holds).length}/${r.flags.length} · ${songs}`;
}

export interface ColumnChips {
  shaky: boolean;
  overlaps: TuneOverlap[];
  readsLike: string | null;
}

/// The chips under a column's verdict: Shaky (fewer than MIN_FLAGS flags,
/// or leave-one-out under SHAKY_LOO when known), Overlaps (another mood's
/// overlap rose by OVERLAP_RISE or more), Reads like (arranged or suggested
/// lines whose name ideas leave out the mood's name).
export function columnChips(
  r: Outcomes & { loo: TuneLeaveOneOut; overlap: TuneOverlap[]; names: TuneNameIdea[] },
  moodName: string,
  source: VersionSource,
): ColumnChips {
  const looWeak = r.loo.total > 0 && r.loo.held / r.loo.total < SHAKY_LOO;
  const generated = source === 'arranged' || source === 'suggested';
  return {
    shaky: r.flags.length < MIN_FLAGS || looWeak,
    overlaps: overlapRises(r.overlap),
    readsLike: generated && nameNote(moodName, r.names) !== null ? r.names[0].word : null,
  };
}

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
  bar: number;
}

/// The selected column's top list against Current's: each song's rank and
/// move, the songs that left (`out`), and whether the two are the same.
export function mergeTop(top: TuneTopSong[], topBefore: TuneTopSong[]): { rows: TopRow[]; out: OutRow[]; same: boolean } {
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
    .map((t, i) => ({ entryId: t.song.entryId, name: t.song.name, oldRank: i + 1, bar: t.bar }))
    .filter((o) => !now.has(o.entryId));
  const same = top.length === topBefore.length && top.every((t, i) => t.song.entryId === topBefore[i].song.entryId);
  return { rows, out, same };
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
