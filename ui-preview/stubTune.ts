// A stand-in for the analyzer's /tune in the UI preview: compare reviews any
// lines (so the Moods page evaluates whatever is typed), and Arrange /
// Suggest make a plausible proposal for moods with no recorded run. Bars
// are the library's suggestions nudged by a hash of the lines: deterministic,
// the same lines always read the same, and Current's lines read as they are.
import type { AppState, LibraryEntry, TuneFlagOutcome, TuneMode, TuneResult, TuneReview } from '@generated/index';
import { bandDistance, flagBounds } from '../src/utils/moodPage';

function hash01(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return ((h >>> 0) % 10007) / 10007;
}

const clamp = (v: number): number => Math.max(0, Math.min(1, v));
const words = (lines: string[]): Set<string> =>
  new Set(lines.flatMap((l) => l.toLowerCase().split(/[\s,]+/)).filter((w) => w.length > 2));

interface Target {
  entry: LibraryEntry;
  before: number;
  flagged: boolean;
  bounds: ReturnType<typeof flagBounds>;
  fresh: boolean;
}

export function stubTune(
  state: AppState,
  moodId: string,
  mode: TuneMode,
  candidates: string[][],
  extraWords: string[],
  bank: string[],
  exclude: string[],
  focusEntryId: string | null,
  recorded: TuneResult | undefined,
): TuneResult {
  const mood = state.moodVocabulary.moods.find((m) => m.id === moodId);
  if (!mood) throw `There is no mood "${moodId}" to tune`;
  const current = mood.prompts;
  const songs: Target[] = [];
  for (const entry of state.libraryEntries) {
    const v = entry.moods.find((m) => m.mood === moodId);
    if (entry.entryType !== 'music' || !v || v.auto === null) continue;
    const flagged = v.flag !== null || v.user !== null;
    songs.push({
      entry,
      before: v.auto,
      flagged,
      bounds: flagged ? flagBounds(v.flag, v.flag ? null : v.user, v.auto) : null,
      fresh: !(v.flag?.stale ?? false),
    });
  }
  const targets = songs.filter((s) => s.flagged && s.bounds);
  const focus =
    focusEntryId === '' ? null : (focusEntryId ?? [...targets].sort((a, b) => b.entry.name.localeCompare(a.entry.name))[0]?.entry.id ?? null);

  const currentWords = words(current);
  const newWords = (lines: string[]) => new Set([...words(lines)].filter((w) => !currentWords.has(w)));
  const otherWords = new Set(
    state.moodVocabulary.moods.filter((m) => m.id !== moodId).flatMap((m) => [...words(m.prompts)]),
  );
  const bar = (s: Target, lines: string[]): number => {
    const cur = new Set(current);
    const changed =
      (lines.filter((l) => !cur.has(l)).length + current.filter((l) => !lines.includes(l)).length) /
      Math.max(1, lines.length + current.length);
    if (changed === 0) return s.before;
    let delta = (hash01(`${s.entry.id}|${lines.join('\n')}`) - 0.5) * 0.36 * changed;
    if (s.bounds) {
      const pull = 0.22 * changed * (0.4 + hash01(`${lines.join('|')}#${s.entry.id}`));
      if (s.bounds.min !== null && s.before < s.bounds.min + 0.03) delta += pull;
      if (s.bounds.max !== null && s.before > s.bounds.max - 0.03) delta -= pull;
    }
    // Another mood's words pull this mood's high songs down (so a version
    // can break flags in the preview).
    if (s.before >= 0.6 && [...newWords(lines)].some((w) => otherWords.has(w))) {
      delta -= 0.12 + 0.1 * hash01(`${s.entry.id}~${lines.join()}`);
    }
    return clamp(s.before + delta);
  };

  const review = (lines: string[], source: string, proposed: boolean): TuneReview => {
    const after = new Map(songs.map((s) => [s.entry.id, bar(s, lines)]));
    const flags: TuneFlagOutcome[] = targets.map((s) => {
      const a = after.get(s.entry.id)!;
      return {
        song: { entryId: s.entry.id, name: s.entry.name, album: '' },
        before: s.before,
        after: a,
        holds: bandDistance(a, s.bounds!) === 0,
        fresh: s.fresh,
        focus: s.entry.id === focus,
      };
    });
    const top = (key: (s: Target) => number) =>
      [...songs].sort((x, y) => key(y) - key(x)).slice(0, 15).map((s) => ({ song: { entryId: s.entry.id, name: s.entry.name, album: '' }, bar: key(s) }));
    const moves = songs
      .filter((s) => !s.flagged)
      .map((s) => ({ song: { entryId: s.entry.id, name: s.entry.name, album: '' }, before: s.before, after: after.get(s.entry.id)! }))
      .filter((m) => Math.abs(m.after - m.before) > 0.005);
    const curWords = words(current);
    const mine = new Set([...words(lines)].filter((w) => !curWords.has(w)));
    const overlap = state.moodVocabulary.moods
      .filter((m) => m.id !== moodId)
      .map((m) => {
        const before = 0.15 + 0.5 * hash01(`${moodId}~${m.id}`);
        const shares = [...words(m.prompts)].some((w) => mine.has(w));
        return { moodId: m.id, before, after: before + (shares ? 0.16 : (hash01(`${m.id}${lines.join()}`) - 0.5) * 0.04) };
      });
    const fires = (k: (s: Target) => number) => songs.filter((s) => k(s) >= 0.5).length;
    const n = flags.length;
    return {
      lines,
      lineSources: lines.map(() => [source]),
      loss: 0,
      stability: 0,
      flags,
      top: top((s) => after.get(s.entry.id)!),
      topBefore: top((s) => s.before),
      risers: moves.filter((m) => m.after > m.before).sort((a, b) => b.after - b.before - (a.after - a.before)).slice(0, 10),
      fallers: moves.filter((m) => m.after < m.before).sort((a, b) => a.after - a.before - (b.after - b.before)).slice(0, 10),
      firesBefore: fires((s) => s.before),
      firesAfter: fires((s) => after.get(s.entry.id)!),
      overlap,
      loo: proposed ? { held: Math.max(0, n - Math.round(hash01(lines.join()) * 2)), total: n } : { held: 0, total: 0 },
      names: proposed
        ? [...mine].slice(0, 3).map((word, i) => ({ word, rho: 0.9 - i * 0.07 }))
        : [],
      unsatisfied: flags.filter((f) => f.fresh && !f.holds).map((f) => f.song),
    };
  };

  // A recorded run's proposal for exactly these lines reads as recorded.
  const recordedFor = (lines: string[]): TuneReview | undefined =>
    recorded?.proposals.find((p) => p.lines.join('\n') === lines.join('\n'));

  let proposals: TuneReview[];
  if (mode === 'compare') {
    proposals = candidates.map((c) => recordedFor(c) ?? review(c, 'your version', false));
  } else if (recorded) {
    proposals = recorded.proposals;
  } else {
    const out = new Set(exclude.map((w) => w.toLowerCase()));
    const keep = (l: string) => ![...out].some((w) => l.toLowerCase().includes(w));
    let lines: string[];
    if (mode === 'arrange') {
      lines = [...new Set([...extraWords, ...current])].filter(keep).slice(0, 6);
      lines = lines.length > 1 ? [lines[lines.length - 1], ...lines.slice(0, -1)] : lines;
    } else {
      const picks = bank.filter(keep).sort((a, b) => hash01(moodId + a) - hash01(moodId + b)).slice(0, 2);
      lines = [...current.filter(keep).slice(0, 3), ...picks];
    }
    proposals = lines.length > 0 ? [review(lines, mode === 'arrange' ? 'your words' : 'word bank', true)] : [];
  }
  return {
    baseline: review(current, 'current lines', false),
    proposals,
    split: mode === 'compare' ? null : (recorded?.split ?? null),
    skipped: [],
    linesTunedFrom: current,
    skippedFlags: 0,
    focusEntryId: focus,
    mode,
  };
}
