<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open as openFile } from '@tauri-apps/plugin-dialog';
import { useDisplay } from 'vuetify';
import type { AcceptedLines, AppState, CaseImportReport, MoodDef, MoodFlagRow, MoodTunePrefs, MoodVocabulary, TuneResult } from '@generated/index';
import { useAppStore, type MoodDraft } from '../stores/appStore';
import { useToast } from '../composables/useToast';
import { useAutoTagEvents } from '../composables/useAutoTagEvents';
import {
  MAX_PAGE_VERSIONS,
  nextVersionName,
  parseLines,
  reachedBars,
  regressions,
  draftStale,
  rescoreProblem,
  splitRegressions,
  type ColumnModel,
  type Regression,
} from '../utils/moodPage';
import {
  flaggedEntries,
  holdingEntryIds,
  reviewTargetIsHigh,
  splitName,
  tuneTargetIds,
  versionProblem,
} from '../utils/moodTune';
import { newMoodId, WORD_BANK } from '../utils/moodWords';
import ConfirmDialog from '../components/ConfirmDialog.vue';
import PromptDialog from '../components/PromptDialog.vue';
import { feedbackPreference, feedbackDisagreement } from '../utils/moodFlags';
import { splitDescription, joinDescription } from '../utils/moodDescription';
import MoodFlagsPanel from '../components/moods/MoodFlagsPanel.vue';
import MoodTopPanel from '../components/moods/MoodTopPanel.vue';

/// Saved and proposed scoring versions share one signed editor. Edits fork a
/// draft; searches produce independent proposals. Only Apply changes saved data.
const store = useAppStore();
const toast = useToast();
const autotag = useAutoTagEvents();
const { width } = useDisplay();
const narrow = computed(() => width.value < 1200);

const ENERGY = 'energy';
const prefs = ref<Record<string, MoodTunePrefs>>({});
const moods = computed(() => store.moodVocabulary.moods);
const selectedId = computed<string>({
  get: () => {
    const id = store.moodsSelectedId;
    if (id === ENERGY || moods.value.some((m) => m.id === id)) return id as string;
    return moods.value[0]?.id ?? ENERGY;
  },
  set: (id) => {
    store.moodsSelectedId = id;
  },
});
const isEnergy = computed(() => selectedId.value === ENERGY);
const mood = computed<MoodDef | null>(() => moods.value.find((m) => m.id === selectedId.value) ?? null);
const currentText = computed(() => mood.value?.prompts.join('\n') ?? '');

const flagCount = (id: string): number => flaggedEntries(store.libraryEntries, id).length;
const songCount = computed(() =>
  store.libraryEntries.filter((e) => {
    const v = e.moods.find((m) => m.mood === selectedId.value);
    const shown = v ? (v.user ?? v.auto) : null;
    return e.entryType === 'music' && shown !== null && shown >= 0.5;
  }).length,
);
const flaggedIds = computed(() => tuneTargetIds(store.libraryEntries, selectedId.value));
const highIds = computed(() => {
  const out = new Set<string>();
  for (const e of store.libraryEntries) {
    const v = e.moods.find((m) => m.mood === selectedId.value);
    if (v && reviewTargetIsHigh(v.flag, v.flag ? null : v.user)) out.add(e.id);
  }
  return out;
});


// --- Drafts (version columns) ---

let nextKey = Math.max(0, ...Object.values(store.moodDrafts).flat().map(d => d.key)) + 1;
const drafts = computed<MoodDraft[]>(() => store.moodDrafts[selectedId.value] ?? []);
function draftList(moodId: string): MoodDraft[] {
  if (!store.moodDrafts[moodId]) store.moodDrafts[moodId] = [];
  return store.moodDrafts[moodId];
}
const versionCount = computed(() => drafts.value.filter((d) => d.source !== 'before' && d.source !== 'split').length);
const canAdd = computed(() => versionCount.value < MAX_PAGE_VERSIONS);

function addDraft(moodId: string, part: Partial<MoodDraft> & Pick<MoodDraft, 'source'>, at?: number): MoodDraft {
  const list = draftList(moodId);
  const d: MoodDraft = {
    key: nextKey++,
    name: nextVersionName(list.map((x) => x.name)),
    text: '',
    review: null,
    reviewedLines: null,
    reviewedAgainst: null,
    reviewedFlags: null,
    evaluating: false,
    running: null,
    error: null,
    split: null,
    ...part,
  };
  if (at === undefined) list.push(d);
  else list.splice(at, 0, d);
  // The store's reactive copy, so later changes show.
  return list.find((x) => x.key === d.key)!;
}
function removeDraft(moodId: string, key: number): void {
  const list = draftList(moodId);
  const i = list.findIndex((d) => d.key === key);
  if (i >= 0) list.splice(i, 1);
  tokens.delete(key);
  clearTimer(key);
  if (selectedKeyByMood[moodId] === key) selectedKeyByMood[moodId] = 'current';
}

/// The selected column per mood: clicking a header selects; editing or
/// creating a version selects it. The Top panel follows it.
const selectedKeyByMood = reactive<Record<string, number | 'current'>>({});
const selectedKey = computed<number | 'current'>({
  get: () => {
    const k = selectedKeyByMood[selectedId.value] ?? 'current';
    return k === 'current' || drafts.value.some((d) => d.key === k) ? k : 'current';
  },
  set: (k) => {
    selectedKeyByMood[selectedId.value] = k;
  },
});
const selectedDraft = computed(() => drafts.value.find((d) => d.key === selectedKey.value) ?? null);

// --- Current's review (the baseline) and evaluation ---

const pinnedFocus = computed(() =>
  store.moodFocus && store.moodFocus.moodId === selectedId.value ? store.moodFocus.entryId : null,
);
const focusName = computed(() => (pinnedFocus.value ? (store.findLibraryEntry(pinnedFocus.value)?.name ?? null) : null));
function clearFocus(): void {
  store.moodFocus = null;
  refreshAll();
}
// The pinned focus goes with its flag.
watch(
  () => {
    const f = store.moodFocus;
    if (!f) return true;
    return store.findLibraryEntry(f.entryId)?.moods.some((m) => m.mood === f.moodId && m.flag !== null) ?? false;
  },
  (stillFlagged) => {
    if (!stillFlagged) store.moodFocus = null;
  },
);

const baseline = computed(() => {
  const b = store.moodBaselines[selectedId.value];
  return b && mood.value && b.lines.join('\n') === currentText.value ? b : null;
});
const baselineState = reactive({ loading: false, error: null as string | null });
const tokens = new Map<number | string, number>();
const timers = new Map<number, number>();
const bump = (k: number | string): number => {
  const t = (tokens.get(k) ?? 0) + 1;
  tokens.set(k, t);
  return t;
};
function clearTimer(key: number): void {
  const t = timers.get(key);
  if (t !== undefined) window.clearTimeout(t);
  timers.delete(key);
}

function compare(moodId: string, lines: string[], profile: import('@generated/index').LearnedMoodProfile | null = null): Promise<TuneResult> {
  const focus = store.moodFocus && store.moodFocus.moodId === moodId ? store.moodFocus.entryId : null;
  return invoke<TuneResult>('tune_mood', {
    moodId,
    mode: 'compare',
    extraWords: [],
    bank: [],
    focusEntryId: focus,
    excludeWords: [],
    candidates: [lines],
    profileVersions: [profile],
  });
}

function keepBaseline(moodId: string, r: TuneResult): void {
  store.moodBaselines[moodId] = { lines: r.linesTunedFrom, review: r.baseline, focusEntryId: r.focusEntryId };
}

async function evaluateCurrent(): Promise<void> {
  const m = mood.value;
  if (!m) return;
  const t = bump(`current:${m.id}`);
  baselineState.loading = true;
  baselineState.error = null;
  try {
    const r = await compare(m.id, m.prompts);
    if (tokens.get(`current:${m.id}`) !== t) return;
    keepBaseline(m.id, r);
  } catch (e) {
    if (tokens.get(`current:${m.id}`) === t) baselineState.error = String(e);
  } finally {
    if (tokens.get(`current:${m.id}`) === t) baselineState.loading = false;
  }
}

async function evaluate(moodId: string, d: MoodDraft): Promise<void> {
  clearTimer(d.key);
  if (d.source === 'split' || d.running || (!d.profile && d.source === 'learned')) return;
  const lines = parseLines(d.text);
  if (lines.length === 0) {
    d.review = null;
    d.error = null;
    return;
  }
  const problem = versionProblem([d.text]);
  if (problem) {
    d.error = problem.replace('Version 1', d.name);
    return;
  }
  d.error = null;
  d.evaluating = true;
  const t = bump(d.key);
  // The flags this review is made against (they may change meanwhile).
  const sig = flagSig.value;
  try {
    const r = await compare(moodId, lines, d.profile ?? null);
    if (tokens.get(d.key) !== t) return;
    d.review = r.proposals[0] ?? null;
    d.reviewedLines = lines.join('\n');
    d.reviewedAgainst = r.linesTunedFrom.join('\n');
    d.reviewedFlags = sig;
    keepBaseline(moodId, r);
  } catch (e) {
    if (tokens.get(d.key) === t) d.error = String(e);
  } finally {
    if (tokens.get(d.key) === t) d.evaluating = false;
  }
}


const isStale = (d: MoodDraft): boolean =>
  draftStale(d, { text: d.text, current: currentText.value, flags: flagSig.value });

/// The selected mood's flags and hand-set levels, so a change re-evaluates.
const flagSig = computed(() =>
  store.libraryEntries
    .map((e) => {
      const v = e.moods.find((m) => m.mood === selectedId.value);
      return v && (v.flag || v.user !== null) ? `${e.id}:${JSON.stringify(v.flag)}:${v.user}` : '';
    })
    .filter(Boolean)
    .join('|') + `|profile:${mood.value?.profile?.id ?? ''}|pairs:${JSON.stringify(prefs.value[selectedId.value]?.comparisons ?? [])}`,
);

/// Everything the selected mood shows, evaluated again: on opening a mood,
/// after its lines change, and after its flags change.
function refreshAll(): void {
  if (!mood.value) return;
  void evaluateCurrent();
  for (const d of drafts.value) {
    if (d.source !== 'split' && !d.running && parseLines(d.text).length > 0) void evaluate(selectedId.value, d);
  }
}

watch(
  () => [selectedId.value, currentText.value] as const,
  () => {
    if (!mood.value) return;
    if (!baseline.value) void evaluateCurrent();
    for (const d of drafts.value) {
      if (d.review === null || d.reviewedAgainst !== currentText.value) {
        if (d.source !== 'split' && !d.running && parseLines(d.text).length > 0) void evaluate(selectedId.value, d);
      }
    }
  },
  { immediate: true },
);

let flagTimer: number | undefined;
watch(flagSig, (now, before) => {
  if (before === undefined || now === before) return;
  window.clearTimeout(flagTimer);
  flagTimer = window.setTimeout(refreshAll, 300);
});

// --- Version models ---

const currentColumn = computed<ColumnModel>(() => ({
  profile: mood.value?.profile ?? null,
  key: 'current',
  name: 'Saved',
  kind: 'current',
  source: null,
  text: currentText.value,
  review: baseline.value?.review ?? null,
  stale: false,
  evaluating: baselineState.loading,
  running: null,
  error: baselineState.error,
  regressionCount: 0,
  splitOffer: false,
  split: null,
}));

function regressionsOf(d: MoodDraft): Regression[] {
  const base = baseline.value?.review;
  if (!d.review || !base) return [];
  return regressions(d.review, base, highIds.value);
}
/// Fresh flags a split breaks, over both halves.
const splitRegressionsOf = (d: MoodDraft): Regression[] =>
  d.split && baseline.value ? splitRegressions(d.split, baseline.value.review) : [];
const columns = computed<ColumnModel[]>(() => [
  currentColumn.value,
  ...drafts.value.map(
    (d): ColumnModel => ({
      profile: d.profile ?? null,
      key: d.key,
      name: d.name,
      kind: d.source === 'before' ? 'before' : d.source === 'split' ? 'split' : 'version',
      source: d.source === 'split' ? null : d.source,
      text: d.text,
      review: d.review,
      stale: isStale(d),
      evaluating: d.evaluating,
      running: d.running,
      error: d.error,
      regressionCount: d.source === 'split' ? splitRegressionsOf(d).length : regressionsOf(d).length,
      splitOffer: d.source === 'suggested' && d.split !== null,
      split: d.source === 'split' ? d.split : null,
    }),
  ),
]);

/// The columns the Flags panel shows a value for (not a split).
const flagColumns = computed(() =>
  columns.value
    .filter((c) => c.kind !== 'split' && (c.key === 'current' || c.key === selectedKey.value))
    .map((c) => {
      const d = drafts.value.find((x) => x.key === c.key);
      const ids = new Set(d ? regressionsOf(d).map((r) => r.outcome.song.entryId) : []);
      return { key: c.key, name: `${c.name}${c.profile && c.profile.version<6 ? ' · earlier' : ''}`, review: c.review, stale: c.stale, regressionIds: ids, comparable: (c.profile?.version ?? 0) === (mood.value?.profile?.version ?? 0) };
    }),
);
const selectedColumn = computed(() => columns.value.find((c) => c.key === selectedKey.value) ?? currentColumn.value);
const topReview = computed(() => {
  const c = selectedColumn.value;
  if (c.kind === 'split') return c.split?.keep ?? null;
  return c.review ?? baseline.value?.review ?? null;
});

const editorText = computed(() => selectedDraft.value?.text ?? currentText.value);
function editDescription(text: string): void {
  let d = selectedDraft.value;
  if (!d || d.source === 'before') {
    if (!canAdd.value) return;
    d = addDraft(selectedId.value, { source: 'learned', name: 'Draft', text });
    selectedKey.value = d.key;
  } else {
    bump(d.key);
    d.evaluating = false;
    d.source = 'learned';
    d.name = 'Draft';
    d.text = text;
    d.profile = null;
    d.review = null;
    d.error = null;
  }
}
const moreText = computed({ get: () => splitDescription(editorText.value).more, set: (text: string) => editDescription(joinDescription(text, lessText.value)) });
const lessText = computed({ get: () => splitDescription(editorText.value).less, set: (text: string) => editDescription(joinDescription(moreText.value, text)) });
const searchBusy = computed(() => drafts.value.some(d => d.running !== null));
const runningDraft = computed(() => drafts.value.find(d => d.running !== null) ?? null);
const canPreview = computed(() => canAdd.value || (selectedDraft.value?.source === 'learned' && !selectedDraft.value.profile));
const canApply = computed(() => !!selectedDraft.value?.profile && !!selectedDraft.value?.review && !isStale(selectedDraft.value));
const selectedFit = computed(() => {
  const review = selectedColumn.value.review;
  if (!review?.flags.length) return null;
  const distances = review.flags.flatMap(f => {
    const value = store.findLibraryEntry(f.song.entryId)?.moods.find(v => v.mood === selectedId.value);
    const preference = value ? feedbackPreference(value.flag, value.flag ? null : value.user, value.auto) : null;
    return preference === null ? [] : [{loss:feedbackDisagreement(f.after,preference),weight:preference.confidence}];
  });
  return { within: review.flags.filter(f => f.holds).length, total: review.flags.length, miss: distances.reduce((a,b) => a+b.loss*b.weight, 0)/Math.max(1, distances.reduce((a,b) => a+b.weight,0)) };
});
function discard(key: number): void { removeDraft(selectedId.value, key); }
function copySelected(): void {
  editDescription(editorText.value);
  document.getElementById('mood-more')?.focus();
}

// --- Independent wording proposals ---

/// Arrange and Suggest search against flags: with none they are off.
const canSearch = computed(() => canAdd.value && (flaggedIds.value.size > 0 || comparisons.value.length > 0));



async function search(kind: 'arrange' | 'suggest' | 'learn', rebuild = false): Promise<void> {
  const m = mood.value;
  if (!m || searchBusy.value || !(canAdd.value || (kind === 'learn' && selectedDraft.value?.source === 'learned' && !selectedDraft.value.profile))) return;
  const extra = parseLines(editorText.value);
  if (!extra.length) return;
  const reusable = kind === 'learn' && selectedDraft.value?.source === 'learned' && !selectedDraft.value.profile ? selectedDraft.value : null;
  const d = reusable ?? addDraft(m.id, { source: kind === 'learn' ? 'learned' : kind === 'arrange' ? 'arranged' : 'suggested', name: `${kind === 'learn' ? 'Preview' : kind === 'arrange' ? 'Recombined' : 'Suggestion'} ${drafts.value.length + 1}`, text: extra.join('\n') });
  d.running = kind;
  if (rebuild) d.name = `Rebuild ${drafts.value.length}`;
  if (kind === 'learn') selectedKey.value = d.key;
  const t = bump(d.key);
  // Flags may change while the search runs; then its review is out of date.
  const sigAtStart = flagSig.value;
  try {
    const r = await invoke<TuneResult>('tune_mood', {
      moodId: m.id,
      mode: kind,
      rebuild,
      extraWords: extra,
      bank: kind === 'suggest' ? WORD_BANK : [],
      focusEntryId: pinnedFocus.value,
      excludeWords: prefs.value[m.id]?.neverSuggest ?? [],
      candidates: drafts.value.filter(x => x.key !== d.key && x.text.trim()).map(x => parseLines(x.text)),
    });
    if (tokens.get(d.key) !== t) return;
    const p = r.proposals[0];
    if (!p) {
      removeDraft(m.id, d.key);
      toast.showInfo('No new wording proposal');
      return;
    }
    if (d.name === 'Draft') d.name = `Preview ${d.key}`;
    d.text = p.lines.join('\n');
    d.review = p;
    d.profile = p.profile;
    d.reviewedLines = p.lines.join('\n');
    d.reviewedAgainst = r.linesTunedFrom.join('\n');
    d.reviewedFlags = sigAtStart;
    d.split = r.split;
    d.running = null;
    if (flagSig.value === sigAtStart) keepBaseline(m.id, r);
    else void evaluate(m.id, d);
  } catch (e) {
    if (tokens.get(d.key) !== t) return;
    d.running = null;
    d.error = String(e);
  }
}

function cancelSearch(key: number): void {
  removeDraft(selectedId.value, key);
}

// --- Split ---

// --- Use ---

const using = ref(false);

function askBreak(_tradeoffs: Regression[], action: () => void): void {
  action();
}

async function onUse(key: number | 'current'): Promise<void> {
  const d = drafts.value.find((x) => x.key === key);
  const m = mood.value;
  if (!d || !m || using.value) return;
  if (d.source === 'split') {
    if (!d.split) return;
    askBreak(splitRegressionsOf(d), () => {
      splitDraft.value = d;
      askSplitNameOpen.value = true;
    });
    return;
  }
  if (!d.review || isStale(d)) await evaluate(m.id, d);
  // Typed into (or Current changed) while it evaluated: not what was asked.
  if (!d.review || d.error || isStale(d)) return;
  askBreak(regressionsOf(d), () => void useLines(m.id, d));
}

async function useLines(moodId: string, d: MoodDraft): Promise<void> {
  const m = moods.value.find((x) => x.id === moodId);
  const review = d.review;
  if (!m || !review) return;
  const oldLines = [...m.prompts];
  const oldProfile = m.profile ?? null;
  const freshBefore = (store.moodBaselines[moodId]?.review.flags ?? []).filter((f) => f.fresh).map((f) => f.song.entryId);
  using.value = true;
  let converted: MoodFlagRow[] = [];
  try {
    const out = await invoke<AcceptedLines>('accept_tuned_lines', {
      moodId,
      linesTunedFrom: oldLines,
      lines: review.lines,
      holdingEntryIds: holdingEntryIds(review),
      reachedBars: reachedBars(review),
      profile: review.profile,
      reviewSignature: review.targetSignature,
    });
    store.hydrate(out.state);
    converted = out.converted;
  } catch (e) {
    toast.showError(String(e));
    return;
  } finally {
    using.value = false;
  }
  const used = { name: d.name, source: d.source, text: d.text, profile: d.profile };
  const at = draftList(moodId).findIndex((x) => x.key === d.key);
  removeDraft(moodId, d.key);
  for (const b of draftList(moodId).filter((x) => x.source === 'before')) removeDraft(moodId, b.key);
  addDraft(moodId, { source: 'before', name: 'Before', text: oldLines.join('\n'), profile: oldProfile }, 0);
  selectedKeyByMood[moodId] = 'current';
  toast.showSuccess(`${m.name} updated`, {
    label: 'Undo',
    run: () => void undo(moodId, review.lines, oldLines, freshBefore, converted, used, at, oldProfile, review.profile?.id ?? null),
  });
  void rescore(moodId);
}

/// Undo of Use: the old lines back, the flags the ratchet converted back as
/// they were, and the used column back where it was.
async function undo(
  moodId: string,
  newLines: string[],
  oldLines: string[],
  freshBefore: string[],
  converted: MoodFlagRow[],
  used: Pick<MoodDraft, 'name' | 'source' | 'text' | 'profile'>,
  at: number,
  oldProfile: import('@generated/index').LearnedMoodProfile | null,
  expectedProfileId: string | null,
): Promise<void> {
  try {
    // One transaction: the lines go back and the converted flags with them.
    store.hydrate(
      await invoke<AppState>('undo_tuned_lines', {
        moodId,
        linesTunedFrom: newLines,
        lines: oldLines,
        holdingEntryIds: freshBefore,
        restore: converted,
        profile: oldProfile,
        expectedProfileId,
      }),
    );
  } catch (e) {
    toast.showError(String(e));
    return;
  }
  for (const b of draftList(moodId).filter((x) => x.source === 'before')) removeDraft(moodId, b.key);
  const back = addDraft(moodId, { ...used }, Math.max(0, Math.min(at, draftList(moodId).length)));
  if (selectedId.value === moodId) selectedKey.value = back.key;
  void rescore(moodId);
}

const splitDraft = ref<MoodDraft | null>(null);
const askSplitNameOpen = ref(false);
const splitNameInitial = computed(() => (splitDraft.value?.split ? splitName(splitDraft.value.split.new.names) : ''));

async function useSplit(newName: string): Promise<void> {
  const d = splitDraft.value;
  const m = mood.value;
  if (!d?.split || !m) return;
  const split = d.split;
  const taken = new Set(moods.value.map((x) => x.id));
  let newId = newMoodId();
  while (taken.has(newId)) newId = newMoodId();
  using.value = true;
  try {
    const state = await invoke<AppState>('accept_tuned_split', {
      moodId: m.id,
      linesTunedFrom: m.prompts,
      keepLines: split.keep.lines,
      newId,
      newName,
      newLines: split.new.lines,
      moveEntryIds: split.newSongs.map((s) => s.entryId),
      keepHoldingEntryIds: holdingEntryIds(split.keep),
      newHoldingEntryIds: holdingEntryIds(split.new),
    });
    store.hydrate(state);
  } catch (e) {
    toast.showError(String(e));
    return;
  } finally {
    using.value = false;
  }
  for (const x of draftList(m.id).filter((x) => x.source === 'split' || x.split === split)) removeDraft(m.id, x.key);
  toast.showSuccess(`${m.name} split`);
  void rescore(m.id);
}

// --- Rescoring ---

/// The mood whose bars are being rescored (after Use), and the one whose
/// rescore failed ("Bars not updated" in its header; never a toast, which
/// could replace one with an Undo).
const rescoringId = ref<string | null>(null);
/// Why the last rescore left a mood's bars as they were.
const notUpdated = ref<{ moodId: string; text: string } | null>(null);
let rescoreLines = '';
const linesOf = (id: string): string =>
  id === ENERGY
    ? JSON.stringify(store.moodVocabulary.energy)
    : (moods.value.find((m) => m.id === id)?.prompts.join('\n') ?? '');
function failRescore(moodId: string, error: string | null): void {
  rescoringId.value = null;
  const text = rescoreProblem({ error, needsRecalibration: true, linesAtStart: rescoreLines, linesNow: linesOf(moodId) });
  if (text) notUpdated.value = { moodId, text };
}
const updating = computed(() => rescoringId.value !== null && rescoringId.value === selectedId.value);
let sawProgress = false;
let updatingTimer: number | undefined;
async function rescore(moodId: string): Promise<void> {
  rescoringId.value = moodId;
  notUpdated.value = null;
  rescoreLines = linesOf(moodId);
  sawProgress = false;
  try {
    await invoke('rescore_moods');
  } catch (e) {
    failRescore(moodId, String(e));
    return;
  }
  // Without a run starting soon (no analyzer), stop saying so.
  window.clearTimeout(updatingTimer);
  updatingTimer = window.setTimeout(() => {
    if (!sawProgress) rescoringId.value = null;
  }, 5000);
}
watch(autotag.progress, (p) => {
  if (rescoringId.value === null) return;
  if (p?.phase === 'error') failRescore(rescoringId.value, p.message ?? 'the analysis failed');
  else if (p) sawProgress = true;
});
/// After the run: the state again, then whether the mood got its bars (the
/// run skips a mood whose words changed meanwhile, or that it cannot
/// calibrate yet).
watch(autotag.completedRuns, async () => {
  const id = rescoringId.value;
  if (id === null) return;
  try {
    store.hydrate(await invoke<AppState>('load_data'));
  } catch {
    // The events composable reloads too; judge on what the store has.
  }
  rescoringId.value = null;
  const text = rescoreProblem({
    error: null,
    needsRecalibration: store.moodsNeedRecalibration,
    linesAtStart: rescoreLines,
    linesNow: linesOf(id),
  });
  notUpdated.value = text ? { moodId: id, text } : null;
});

/// The full Recalibrate (⋯ menu): clears and rescores every mood.
async function recalibrateAll(): Promise<void> {
  await store.command('recalibrate_moods');
}

// --- Never suggest (per-mood prefs) ---


/// The preferences could be read; until then (or after a failure) Never
/// suggest is off, so nothing overwrites what is stored.
const prefsOk = ref(false);
onMounted(async () => {
  try {
    prefs.value = (await invoke<Record<string, MoodTunePrefs>>('load_mood_tune_prefs')) ?? {};
    prefsOk.value = true;
  } catch {
    prefsOk.value = false;
  }
});
const neverList = computed(() => prefs.value[selectedId.value]?.neverSuggest ?? []);
async function removeNeverSuggest(id: string, word: string): Promise<void> {
  try {
    prefs.value = await invoke<Record<string, MoodTunePrefs>>('remove_never_suggest', { mood: id, word });
  } catch (e) {
    toast.showError(String(e));
  }
}
const comparisons = computed(() => prefs.value[selectedId.value]?.comparisons ?? []);
const higherSong = ref<string | null>(null);
const lowerSong = ref<string | null>(null);
const comparisonSongs = computed(() => store.libraryEntries.filter(e => e.entryType === 'music').map(e => ({title: e.name, value:e.id})).sort((a,b) => a.title.localeCompare(b.title)));
const comparisonName = (id: string) => store.findLibraryEntry(id)?.name ?? 'Removed song';
async function saveComparisons(pairs: [string,string][]): Promise<void> {
  const id = selectedId.value;
  try {
    prefs.value = await invoke<Record<string, MoodTunePrefs>>('set_mood_comparisons', {mood:id,pairs});
    for (const d of drafts.value) d.review = null;
    delete store.moodBaselines[id];
    higherSong.value = lowerSong.value = null;
  } catch (e) { toast.showError(String(e)); }
}
const neverOpen = ref(false);

// --- The mood list and the ⋯ menu ---

async function saveVocabulary(moodsNext: MoodDef[], energy = store.moodVocabulary.energy): Promise<boolean> {
  const vocabulary: MoodVocabulary = { moods: moodsNext, energy };
  return store.command('set_mood_vocabulary', { vocabulary });
}

function nameProblem(name: string, exceptId: string | null): string | null {
  const n = name.trim().toLowerCase();
  if (!n) return 'Give it a name';
  if (n === 'energy' || moods.value.some((m) => m.id !== exceptId && m.name.trim().toLowerCase() === n)) {
    return 'That name is taken';
  }
  return null;
}

const renameOpen = ref(false);
const renameProblem = (name: string): string | null => nameProblem(name, mood.value?.id ?? null);
const addProblem = (name: string): string | null => nameProblem(name, null);
async function renameMood(name: string): Promise<void> {
  const m = mood.value;
  if (!m || nameProblem(name, m.id)) return;
  await saveVocabulary(moods.value.map((x) => (x.id === m.id ? { ...x, name: name.trim() } : x)));
}

async function moveMood(step: -1 | 1): Promise<void> {
  const list = [...moods.value];
  const i = list.findIndex((m) => m.id === selectedId.value);
  const to = i + step;
  if (i < 0 || to < 0 || to >= list.length) return;
  const [m] = list.splice(i, 1);
  list.splice(to, 0, m);
  await saveVocabulary(list);
}

const deleteOpen = ref(false);
const deleteFlags = computed(() => flagCount(selectedId.value));
const deleteMessage = computed(() => {
  const f = deleteFlags.value;
  const n = deleteLevels.value;
  return `Removes its bars, ${f} flag${f === 1 ? '' : 's'} and ${n} level${n === 1 ? '' : 's'} you set.`;
});
const deleteLevels = computed(
  () => store.libraryEntries.filter((e) => e.moods.some((m) => m.mood === selectedId.value && m.user !== null)).length,
);
async function deleteMood(): Promise<void> {
  const m = mood.value;
  if (!m || moods.value.length <= 1) return;
  const i = moods.value.findIndex((x) => x.id === m.id);
  const ok = await saveVocabulary(moods.value.filter((x) => x.id !== m.id));
  if (!ok) return;
  delete store.moodDrafts[m.id];
  delete store.moodBaselines[m.id];
  selectedId.value = moods.value[Math.max(0, i - 1)]?.id ?? ENERGY;
}

const addOpen = ref(false);
async function addMood(name: string): Promise<void> {
  if (nameProblem(name, null)) return;
  const taken = new Set(moods.value.map((m) => m.id));
  let id = newMoodId();
  while (taken.has(id)) id = newMoodId();
  const ok = await saveVocabulary([...moods.value, { id, name: name.trim(), prompts: [name.trim().toLowerCase()] }]);
  if (!ok) return;
  selectedId.value = id;
  void rescore(id);
}

const recalibrateOpen = ref(false);
const resetOpen = ref(false);
async function resetAll(): Promise<void> {
  const ok = await store.command('reset_mood_vocabulary');
  if (!ok) return;
  store.moodDrafts = {};
  store.moodBaselines = {};
  if (store.moodsNeedRecalibration) void rescore(selectedId.value);
}

const importing = ref(false);
const importReport = ref<CaseImportReport | null>(null);
const importReportOpen = ref(false);
async function importCaseStudies(): Promise<void> {
  const path = await openFile({ multiple: false, filters: [{ name: 'Case studies', extensions: ['json'] }] });
  if (!path || Array.isArray(path)) return;
  importing.value = true;
  try {
    importReport.value = await invoke<CaseImportReport>('import_case_studies', { path });
    importReportOpen.value = true;
    store.hydrate(await invoke<AppState>('load_data'));
  } catch (e) {
    toast.showError(String(e));
  } finally {
    importing.value = false;
  }
}

// --- Energy ---

const energyPositive = ref('');
const energyNegative = ref('');
function loadEnergy(): void {
  energyPositive.value = store.moodVocabulary.energy.positive.join('\n');
  energyNegative.value = store.moodVocabulary.energy.negative.join('\n');
}
watch(() => store.moodVocabulary.energy, loadEnergy, { immediate: true });
const energyChanged = computed(
  () =>
    parseLines(energyPositive.value).join('\n') !== store.moodVocabulary.energy.positive.join('\n') ||
    parseLines(energyNegative.value).join('\n') !== store.moodVocabulary.energy.negative.join('\n'),
);
const energyValid = computed(() => parseLines(energyPositive.value).length > 0 && parseLines(energyNegative.value).length > 0);
async function saveEnergy(): Promise<void> {
  const ok = await saveVocabulary(moods.value, {
    positive: parseLines(energyPositive.value),
    negative: parseLines(energyNegative.value),
  });
  if (ok) void rescore(ENERGY);
}

// --- Keyboard ---

const moodList = ref<HTMLElement | null>(null);
function onKeydown(e: KeyboardEvent): void {
  if (isEnergy.value) return;
  const inList = moodList.value?.contains(e.target as Node) ?? false;
  if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown') && inList && mood.value) {
    e.preventDefault();
    void moveMood(e.key === 'ArrowUp' ? -1 : 1);
    return;
  }
  if ((e.altKey || (e.target as HTMLElement)?.closest('[role="tablist"]')) && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
    e.preventDefault();
    const keys = columns.value.map((c) => c.key);
    const i = keys.indexOf(selectedKey.value);
    const next = keys[Math.max(0, Math.min(keys.length - 1, i + (e.key === 'ArrowLeft' ? -1 : 1)))];
    selectedKey.value = next;
    document.querySelector<HTMLElement>(`[data-version-key="${next}"]`)?.focus();
    return;
  }
  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && (e.key === 'd' || e.key === 'D')) {
    e.preventDefault();
    copySelected();
    return;
  }
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
    e.preventDefault();
    const d = selectedDraft.value;
    if (e.shiftKey) {
      if (d) void onUse(d.key);
    } else void search('learn');
  }
}
onBeforeUnmount(() => {
  for (const t of timers.values()) window.clearTimeout(t);
  window.clearTimeout(flagTimer);
  window.clearTimeout(updatingTimer);
});

const moodItems = computed(() => [
  ...moods.value.map((m) => ({ title: m.name, value: m.id })),
  { title: 'Energy', value: ENERGY },
]);
const selectedName = computed(() => (isEnergy.value ? 'Energy' : (mood.value?.name ?? '')));
</script>

<template>
  <div class="moods-page" :class="{ narrow }" @keydown="onKeydown">
    <!-- Mood list (a dropdown beside the name when narrow) -->
    <nav v-if="!narrow" ref="moodList" class="mood-list" aria-label="Moods">
      <div class="list-items">
        <button
          v-for="m in moods"
          :key="m.id"
          type="button"
          class="mood-item"
          :class="{ selected: m.id === selectedId }"
          :aria-current="m.id === selectedId ? 'true' : undefined"
          @click="selectedId = m.id"
        >
          <span class="text-truncate">{{ m.name }}</span>
          <span class="count">{{ flagCount(m.id) || '' }}</span>
        </button>
      </div>
      <button type="button" class="mood-item" :class="{ selected: isEnergy }" @click="selectedId = ENERGY">
        <span>Energy</span>
      </button>
      <button type="button" class="mood-item add" @click="addOpen = true">+ Mood</button>
    </nav>

    <main class="mood-main">
      <header class="main-head">
        <VSelect
          v-if="narrow"
          :model-value="selectedId"
          :items="moodItems"
          density="compact"
          variant="outlined"
          hide-details
          class="mood-select"
          aria-label="Mood"
          @update:model-value="(v: string) => (selectedId = v)"
        />
        <h1 v-else class="mood-title">{{ selectedName }}</h1>
        <span v-if="!isEnergy" class="counts text-body-2 text-medium-emphasis">
          {{ songCount }} songs · {{ flagCount(selectedId) }} examples
        </span>
        <span v-if="updating" class="updating text-caption text-medium-emphasis">
          <VProgressCircular indeterminate size="12" width="2" class="mr-1" />Updating bars…
        </span>
        <span v-else-if="notUpdated?.moodId === selectedId" class="updating text-caption text-error">{{ notUpdated.text }}</span>
        <span v-if="focusName && !isEnergy" class="focus-pin text-body-2">
          Focus: {{ focusName }}
          <VBtn icon size="x-small" variant="text" aria-label="Clear focus" class="hit" @click="clearFocus">
            <VIcon size="small">mdi-close</VIcon>
            <VTooltip activator="parent" location="top" open-on-focus>Clear focus</VTooltip>
          </VBtn>
        </span>
        <VSpacer />
        <VBtn v-if="narrow" size="small" variant="text" @click="addOpen = true">+ Mood</VBtn>
        <VMenu location="bottom end">
          <template #activator="{ props: menuProps }">
            <VBtn v-bind="menuProps" icon size="small" variant="text" aria-label="Mood actions">
              <VIcon>mdi-dots-horizontal</VIcon>
            </VBtn>
          </template>
          <VList density="compact">
            <template v-if="!isEnergy">
              <VListItem title="Rename" @click="renameOpen = true" />
              <VListItem title="Move up" :disabled="moods[0]?.id === selectedId" @click="moveMood(-1)" />
              <VListItem title="Move down" :disabled="moods[moods.length - 1]?.id === selectedId" @click="moveMood(1)" />
              <VListItem title="Delete mood" :disabled="moods.length <= 1" @click="deleteOpen = true" />
              <VListItem :title="`Never suggest (${neverList.length})`" :disabled="!prefsOk" @click="neverOpen = true" />
              <VDivider />
            </template>
            <VListItem title="Import case studies" :disabled="importing" @click="importCaseStudies" />
            <VListItem title="Recalibrate" @click="recalibrateOpen = true" />
            <VListItem title="Reset all moods" @click="resetOpen = true" />
          </VList>
        </VMenu>
      </header>
      <VAlert v-if="store.moodVocabularyUnreadable" type="error" variant="tonal" density="compact" class="mb-2">
        Mood list unreadable; defaults shown. Reset all moods to fix it.
      </VAlert>

      <!-- Energy: two lists and Save -->
      <section v-if="isEnergy" class="energy">
        <VTextarea v-model="energyPositive" label="Energetic" variant="outlined" density="compact" auto-grow rows="4" hide-details />
        <VTextarea v-model="energyNegative" label="Calm" variant="outlined" density="compact" auto-grow rows="4" hide-details />
        <div class="energy-save">
          <VBtn color="primary" variant="tonal" :disabled="!energyChanged || !energyValid" @click="saveEnergy">Save</VBtn>
        </div>
      </section>

      <template v-else-if="mood">
        <div class="version-toolbar">
          <div class="version-tabs" role="tablist" aria-label="Scoring versions">
            <button v-for="c in columns.filter(c => c.kind !== 'split')" :key="c.key" :data-version-key="c.key" type="button" role="tab" :aria-selected="c.key === selectedKey" :tabindex="c.key === selectedKey ? 0 : -1" :class="{ active: c.key === selectedKey }" @click="selectedKey = c.key">
              {{ c.key === 'current' ? 'Saved' : c.kind === 'before' ? 'Previous' : c.name }}
              <span v-if="c.running" class="text-caption">…</span>
            </button>
          </div>
          <VBtn v-if="selectedDraft" variant="text" size="small" :disabled="searchBusy || using" @click="discard(selectedDraft.key)">Discard</VBtn>
        </div>
        <section class="description-editor" aria-label="Mood description">
          <VTextarea id="mood-more" v-model="moreText" label="More like" variant="outlined" density="compact" rows="2" auto-grow hide-details :disabled="using || searchBusy || (!canAdd && (!selectedDraft || selectedDraft.source === 'before'))" />
          <VTextarea v-model="lessText" label="Less like" variant="outlined" density="compact" rows="2" auto-grow hide-details :disabled="using || searchBusy || (!canAdd && (!selectedDraft || selectedDraft.source === 'before'))" />
        </section>
        <div class="preview-toolbar">
          <VMenu>
            <template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" variant="text" size="small" append-icon="mdi-chevron-down" :disabled="!canSearch || searchBusy">Wording</VBtn></template>
            <VList density="compact">
              <VListItem title="Suggest alternatives" @click="search('suggest')" />
              <VListItem title="Recombine these words" @click="search('arrange')" />
              <VDivider />
              <VListItem title="Rebuild from wording" subtitle="Relearn the whole mood" @click="search('learn', true)" />
            </VList>
          </VMenu>
          <VProgressCircular v-if="searchBusy || selectedColumn.evaluating" indeterminate size="18" width="2" />
          <span v-if="selectedFit" class="text-caption text-medium-emphasis">Feedback disagreement {{ selectedFit.miss.toFixed(3) }}</span>
          <VChip v-if="selectedColumn.profile && selectedColumn.profile.version < 6" size="x-small" variant="tonal">Earlier scoring<VTooltip activator="parent">This version predates evidence-controlled learning; compare its numerical scores with care.</VTooltip></VChip>
          <VChip v-if="selectedColumn.profile && selectedColumn.profile.trainingCount !== flaggedIds.size" size="x-small" color="primary" variant="tonal">Feedback changed</VChip>
          <VSpacer />
          <VBtn v-if="runningDraft" variant="text" size="small" @click="cancelSearch(runningDraft.key)">Cancel</VBtn>
          <VBtn v-else-if="canApply" color="primary" variant="tonal" :disabled="using" @click="onUse(selectedKey)">{{ selectedDraft?.source === 'before' ? 'Restore version' : 'Apply' }}</VBtn>
          <VBtn v-else color="primary" variant="tonal" :disabled="using || searchBusy || !canPreview || !moreText.trim()" @click="search('learn')">Preview</VBtn>
          <VBtn v-if="canApply && selectedDraft?.source !== 'before'" variant="text" size="small" :disabled="using || searchBusy" @click="search('learn')">Refit</VBtn>
        </div>
        <div v-if="selectedColumn.error" role="alert" class="text-body-2 text-error">{{ selectedColumn.error }}</div>
        <details v-if="selectedColumn.profile" class="scoring-details">
          <summary>Scoring details</summary>
          <div class="text-caption text-medium-emphasis">{{ selectedColumn.profile.trainingCount }} examples · {{ selectedColumn.profile.adjustment ? 'Saved model + local feedback' : selectedColumn.profile.method === 'local' ? 'Similar sounds' : selectedColumn.profile.method === 'global' ? 'Broad pattern' : selectedColumn.profile.method === 'words' ? 'Words only' : `${selectedColumn.profile.audioWeights.length} audio signals` }}</div>
          <div v-if="selectedColumn.review?.validationMae != null" class="text-caption text-medium-emphasis">Earlier rating error {{ selectedColumn.review.validationMae.toFixed(3) }}</div>
          <div v-if="selectedColumn.review?.validationLoss != null" class="text-caption text-medium-emphasis">Held-out disagreement {{ selectedColumn.review.validationLoss.toFixed(3) }}</div>
          <div v-for="axis in selectedColumn.profile.wordAxes" :key="axis.label" class="text-caption">{{ axis.weight.toFixed(3) }} · {{ axis.prompts.join(', ') }}</div>
        </details>
        <details class="mb-4">
          <summary>Comparisons<span v-if="comparisons.length"> · {{ comparisons.length }}</span></summary>
          <div v-for="(pair,i) in comparisons" :key="pair.join(':')" class="d-flex align-center ga-2 mt-2">
            <span class="text-body-2">{{ comparisonName(pair[0]) }} <span class="text-medium-emphasis">more {{ mood.name }} than</span> {{ comparisonName(pair[1]) }}</span>
            <VBtn icon="mdi-close" size="x-small" variant="text" aria-label="Remove comparison" @click="saveComparisons(comparisons.filter((_,n) => n !== i))" />
          </div>
          <div class="d-flex align-center ga-3 mt-3">
            <VAutocomplete v-model="higherSong" :items="comparisonSongs" label="More" density="compact" hide-details />
            <VAutocomplete v-model="lowerSong" :items="comparisonSongs" label="Less" density="compact" hide-details />
            <VBtn :disabled="!prefsOk || !higherSong || !lowerSong || higherSong === lowerSong" @click="higherSong && lowerSong && saveComparisons([...comparisons, [higherSong,lowerSong]])">Add</VBtn>
          </div>
        </details>
        <!-- Flags and Top 15 -->
        <div class="bottom">
          <MoodFlagsPanel
            class="panel"
            :mood-id="mood.id"
            :columns="flagColumns"
            :focus-entry-id="baseline?.focusEntryId ?? pinnedFocus"
            :narrow="narrow"
          />
          <MoodTopPanel
            class="panel"
            :mood-id="mood.id"
            :review="topReview"
            :is-current="selectedColumn.kind === 'current' || !selectedColumn.review"
            :version-name="selectedColumn.review ? selectedColumn.name : 'Saved'"
            :flagged-ids="flaggedIds"
            :high-ids="highIds"
          />
        </div>
      </template>
    </main>

    <!-- Dialogs -->
    <PromptDialog
      v-model="askSplitNameOpen"
      title="New mood name"
      label="Name"
      :initial="splitNameInitial"
      confirm-label="Use split"
      @submit="useSplit"
    />
    <PromptDialog
      v-model="renameOpen"
      title="Rename"
      label="Name"
      :initial="mood?.name ?? ''"
      :validate="renameProblem"
      @submit="renameMood"
    />
    <PromptDialog v-model="addOpen" title="New mood" label="Name" confirm-label="Add" :validate="addProblem" @submit="addMood" />
    <VDialog v-model="neverOpen" max-width="440">
      <VCard :title="`Never suggest (${neverList.length})`">
        <VCardText>
          <div v-if="neverList.length === 0" class="text-body-2 text-medium-emphasis">None</div>
          <div class="d-flex flex-wrap ga-2">
            <VChip v-for="w in neverList" :key="w" closable size="small" @click:close="removeNeverSuggest(selectedId, w)">
              {{ w }}
            </VChip>
          </div>
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn variant="plain" @click="neverOpen = false">Close</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
    <ConfirmDialog
      v-model="deleteOpen"
      :title="`Delete ${mood?.name ?? 'mood'}?`"
      :message="deleteMessage"
      confirm-label="Delete"
      confirm-color="error"
      @confirm="deleteMood"
    />
    <ConfirmDialog
      v-model="resetOpen"
      title="Reset all moods?"
      message="Restores the shipped moods and words."
      confirm-label="Reset"
      confirm-color="error"
      @confirm="resetAll"
    />
    <ConfirmDialog
      v-model="recalibrateOpen"
      title="Recalibrate?"
      message="Rescores every mood. Your levels stay."
      confirm-label="Recalibrate"
      @confirm="recalibrateAll"
    />
    <VDialog v-model="importReportOpen" max-width="480">
      <VCard v-if="importReport" title="Case studies imported">
        <VCardText>
          <div class="mb-2">
            {{ importReport.imported }} flag{{ importReport.imported === 1 ? '' : 's' }}
            <template v-if="importReport.settledSkipped > 0"> · {{ importReport.settledSkipped }} settled, left out</template>
          </div>
          <div v-if="importReport.unmatched.length > 0" class="mb-2">
            <div class="text-subtitle-2">Not found ({{ importReport.unmatched.length }})</div>
            <ul class="text-caption import-list">
              <li v-for="name in importReport.unmatched" :key="name">{{ name }}</li>
            </ul>
          </div>
          <div v-if="importReport.skippedMoods.length > 0">
            <div class="text-subtitle-2">Unknown moods</div>
            <div class="text-caption">{{ importReport.skippedMoods.join(', ') }}</div>
          </div>
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn variant="plain" @click="importReportOpen = false">Close</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>

<style scoped>
 .description-editor { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; flex: 0 0 auto; }
.version-toolbar, .preview-toolbar { display: flex; align-items: center; gap: 8px; flex: 0 0 auto; }
.version-tabs { display: flex; flex: 1; overflow-x: auto; gap: 4px; }
.version-tabs button { appearance: none; background: transparent; border: 0; border-bottom: 2px solid transparent; color: rgba(var(--v-theme-on-surface), .6); padding: 8px 12px; white-space: nowrap; font-size: .875rem; }
.version-tabs button.active { color: rgb(var(--v-theme-primary)); border-bottom-color: currentColor; }
.version-tabs button:focus-visible { outline: 1px solid currentColor; }
.scoring-details { font-size: .75rem; color: rgba(var(--v-theme-on-surface), .6); }
.scoring-details[open] { padding-bottom: 12px; }
@media (max-width: 850px) { .description-editor { grid-template-columns: 1fr; } }


.moods-page {
  height: calc(100vh - var(--v-layout-top, 0px) - var(--v-layout-bottom, 0px));
  display: grid;
  grid-template-columns: 200px minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr);
  /* clip, not hidden: nothing (such as scrollIntoView) may scroll the page itself. */
  overflow: clip;
}
.moods-page.narrow {
  grid-template-columns: minmax(0, 1fr);
}
.mood-list {
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-right: 1px solid rgba(var(--v-theme-on-surface), 0.12);
  padding: 8px 0;
}
.list-items {
  flex: 1 1 auto;
  overflow-y: auto;
  min-height: 0;
}
.mood-item {
  appearance: none;
  background: transparent;
  border: 0;
  display: flex;
  align-items: center;
  width: 100%;
  gap: 8px;
  min-height: 34px;
  padding: 4px 12px 4px 13px;
  border-left: 3px solid transparent;
  text-align: left;
  font-size: 0.875rem;
  color: inherit;
}
.mood-item:hover {
  background: rgba(var(--v-theme-on-surface), 0.05);
}
.mood-item:focus-visible {
  outline: 2px solid rgba(var(--v-theme-primary), 0.6);
  outline-offset: -2px;
}
.mood-item.selected {
  border-left-color: rgb(var(--v-theme-primary));
  background: rgba(var(--v-theme-primary), 0.08);
  font-weight: 600;
}
.mood-item .count {
  margin-left: auto;
  font-variant-numeric: tabular-nums;
  color: rgba(var(--v-theme-on-surface), 0.6);
  font-size: 0.8125rem;
}
.mood-item.add {
  color: rgb(var(--v-theme-primary));
  border-top: 1px solid rgba(var(--v-theme-on-surface), 0.08);
}
.mood-main {
  display: flex;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
  padding: 8px 16px 16px;
  overflow: hidden;
  gap: 8px;
}
.main-head {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 40px;
  flex: 0 0 auto;
}
.mood-title {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0;
}
.mood-select {
  flex: 0 0 200px;
}
.updating {
  display: inline-flex;
  align-items: center;
}
.focus-pin {
  display: inline-flex;
  align-items: center;
  gap: 2px;
}
.hit {
  min-width: 28px;
  min-height: 28px;
}
.bottom {
  flex: 1 1 0;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1.15fr) minmax(0, 1fr);
  gap: 12px;
}
/* One scroll surface per result panel; the editor stays in place. */
.narrow .bottom {
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: repeat(2, minmax(0, 1fr));
}
.panel {
  min-height: 0;
  overflow: auto;
  border: 1px solid rgba(var(--v-theme-on-surface), 0.12);
  border-radius: 6px;
}
.energy {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
  max-width: 900px;
}
.energy-save {
  grid-column: 1 / -1;
}
.import-list {
  padding-left: 20px;
  margin: 0;
}
</style>
