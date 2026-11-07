<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open as openFile } from '@tauri-apps/plugin-dialog';
import { useDisplay } from 'vuetify';
import type { AcceptedLines, AppState, CaseImportReport, MoodDef, MoodFlagRow, MoodTunePrefs, MoodVocabulary, TuneResult } from '@generated/index';
import { useAppStore, type MoodDraft } from '../stores/appStore';
import { useToast } from '../composables/useToast';
import { useAutoTagEvents } from '../composables/useAutoTagEvents';
import {
  bandWord,
  confirmLine,
  confirmTitle,
  MAX_PAGE_VERSIONS,
  nextVersionName,
  neverSuggestWords,
  parseLines,
  reachedBars,
  regressions,
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
import MoodVersionColumn from '../components/moods/MoodVersionColumn.vue';
import MoodFlagsPanel from '../components/moods/MoodFlagsPanel.vue';
import MoodTopPanel from '../components/moods/MoodTopPanel.vue';

/// The Moods page: the mood list on the left (Energy pinned last), and for
/// the selected mood its versions as columns (Current, then drafts), the
/// Flags panel and the Top 15 panel. Nothing changes until a version is
/// used; Use asks only when it would break a flag that holds now, then
/// rescores (recalibrates) on its own.
const store = useAppStore();
const toast = useToast();
const autotag = useAutoTagEvents();
const { width } = useDisplay();
const narrow = computed(() => width.value < 1200);

const ENERGY = 'energy';
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
const moodNameOf = (id: string): string => moods.value.find((m) => m.id === id)?.name ?? id;
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
const bandOf = (entryId: string): string => {
  const v = store.findLibraryEntry(entryId)?.moods.find((m) => m.mood === selectedId.value);
  return v ? bandWord(v.flag, v.flag ? null : v.user) : '';
};

// --- Drafts (version columns) ---

let nextKey = 1;
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
const otherMoodNames = computed(() => moods.value.filter((m) => m.id !== selectedId.value).map((m) => m.name));

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

function compare(moodId: string, lines: string[]): Promise<TuneResult> {
  const focus = store.moodFocus && store.moodFocus.moodId === moodId ? store.moodFocus.entryId : null;
  return invoke<TuneResult>('tune_mood', {
    moodId,
    mode: 'compare',
    extraWords: [],
    bank: [],
    focusEntryId: focus,
    excludeWords: [],
    candidates: [lines],
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
  if (d.source === 'split' || d.running) return;
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
  try {
    const r = await compare(moodId, lines);
    if (tokens.get(d.key) !== t) return;
    d.review = r.proposals[0] ?? null;
    d.reviewedLines = lines.join('\n');
    d.reviewedAgainst = r.linesTunedFrom.join('\n');
    keepBaseline(moodId, r);
  } catch (e) {
    if (tokens.get(d.key) === t) d.error = String(e);
  } finally {
    if (tokens.get(d.key) === t) d.evaluating = false;
  }
}

function schedule(d: MoodDraft): void {
  clearTimer(d.key);
  const moodId = selectedId.value;
  timers.set(d.key, window.setTimeout(() => void evaluate(moodId, d), 600));
}

const isStale = (d: MoodDraft): boolean =>
  d.review !== null && (d.reviewedLines !== parseLines(d.text).join('\n') || d.reviewedAgainst !== currentText.value);

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

/// The selected mood's flags and hand-set levels, so a change re-evaluates.
const flagSig = computed(() =>
  store.libraryEntries
    .map((e) => {
      const v = e.moods.find((m) => m.mood === selectedId.value);
      return v && (v.flag || v.user !== null) ? `${e.id}:${JSON.stringify(v.flag)}:${v.user}` : '';
    })
    .filter(Boolean)
    .join('|'),
);
let flagTimer: number | undefined;
watch(flagSig, (now, before) => {
  if (before === undefined || now === before) return;
  window.clearTimeout(flagTimer);
  flagTimer = window.setTimeout(refreshAll, 300);
});

// --- Columns ---

const currentColumn = computed<ColumnModel>(() => ({
  key: 'current',
  name: 'Current',
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
const splitRegressions = (d: MoodDraft): Regression[] =>
  d.split ? [...d.split.keep.flags, ...d.split.new.flags].filter((f) => f.fresh && !f.holds).map((outcome) => ({ outcome, leftTop: false })) : [];

const columns = computed<ColumnModel[]>(() => [
  currentColumn.value,
  ...drafts.value.map(
    (d): ColumnModel => ({
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
      regressionCount: d.source === 'split' ? splitRegressions(d).length : regressionsOf(d).length,
      splitOffer: d.source === 'suggested' && d.split !== null,
      split: d.source === 'split' ? d.split : null,
    }),
  ),
]);

/// The columns the Flags panel shows a value for (not a split).
const flagColumns = computed(() =>
  columns.value
    .filter((c) => c.kind !== 'split')
    .map((c) => {
      const d = drafts.value.find((x) => x.key === c.key);
      const ids = new Set(d ? regressionsOf(d).map((r) => r.outcome.song.entryId) : []);
      return { key: c.key, name: c.name, review: c.review, stale: c.stale, regressionIds: ids };
    }),
);
const selectedColumn = computed(() => columns.value.find((c) => c.key === selectedKey.value) ?? currentColumn.value);
const topReview = computed(() => {
  const c = selectedColumn.value;
  if (c.kind === 'split') return c.split?.keep ?? null;
  return c.review;
});

const colRefs = new Map<number | 'current', InstanceType<typeof MoodVersionColumn>>();
function setColRef(key: number | 'current', el: unknown): void {
  if (el) colRefs.set(key, el as InstanceType<typeof MoodVersionColumn>);
  else colRefs.delete(key);
}

function onText(d: MoodDraft, text: string): void {
  d.text = text;
  selectedKey.value = d.key;
  schedule(d);
}
function onCommit(d: MoodDraft): void {
  if (isStale(d) || (d.review === null && parseLines(d.text).length > 0)) void evaluate(selectedId.value, d);
}

/// Typing into Current makes a new version holding the edit.
async function onFork(text: string, line: number, caret: number): Promise<void> {
  if (!canAdd.value) {
    toast.showInfo(`${MAX_PAGE_VERSIONS} versions at most`);
    return;
  }
  const d = addDraft(selectedId.value, { source: 'typed', text });
  selectedKey.value = d.key;
  schedule(d);
  await nextTick();
  colRefs.get(d.key)?.focusLine(line, caret);
}

/// Typing into the ghost column starts a new version.
const ghostText = ref('');
async function onGhostInput(e: Event): Promise<void> {
  const el = e.target as HTMLTextAreaElement;
  const text = el.value;
  el.value = '';
  ghostText.value = '';
  if (!canAdd.value || text.trim() === '') return;
  const d = addDraft(selectedId.value, { source: 'typed', text });
  selectedKey.value = d.key;
  schedule(d);
  await nextTick();
  colRefs.get(d.key)?.focusLine(0, text.length);
}

function copySelected(): void {
  if (!canAdd.value || isEnergy.value) return;
  const c = selectedColumn.value;
  const text = c.kind === 'split' ? (c.split?.keep.lines.join('\n') ?? '') : c.text;
  const d = addDraft(selectedId.value, { source: 'typed', text, review: c.review, reviewedLines: c.review ? parseLines(text).join('\n') : null, reviewedAgainst: c.review ? currentText.value : null });
  selectedKey.value = d.key;
  void scrollToColumn(d.key);
}

function discard(key: number | 'current'): void {
  if (key === 'current') return;
  removeDraft(selectedId.value, key);
}

function rename(d: MoodDraft, name: string): void {
  d.name = name;
}

// --- Arrange / Suggest ---

/// Arrange and Suggest search against flags: with none they are off.
const canSearch = computed(() => canAdd.value && flaggedIds.value.size > 0);

async function scrollToColumn(key: number): Promise<void> {
  await nextTick();
  document.querySelector(`[data-col="${key}"]`)?.scrollIntoView({ behavior: 'smooth', inline: 'nearest', block: 'nearest' });
}

async function search(kind: 'arrange' | 'suggest'): Promise<void> {
  const m = mood.value;
  if (!m || !canSearch.value) return;
  const from = selectedColumn.value;
  const extra = from.kind === 'version' || from.kind === 'before' ? parseLines(from.text) : [];
  const d = addDraft(m.id, { source: kind === 'arrange' ? 'arranged' : 'suggested', running: kind });
  selectedKey.value = d.key;
  void scrollToColumn(d.key);
  const t = bump(d.key);
  // Flags may change while the search runs; then its review is out of date.
  const sigAtStart = flagSig.value;
  try {
    const r = await invoke<TuneResult>('tune_mood', {
      moodId: m.id,
      mode: kind,
      extraWords: extra,
      bank: kind === 'suggest' ? WORD_BANK : [],
      focusEntryId: pinnedFocus.value,
      excludeWords: prefs.value[m.id]?.neverSuggest ?? [],
      candidates: [],
    });
    if (tokens.get(d.key) !== t) return;
    const p = r.proposals[0];
    if (!p) {
      removeDraft(m.id, d.key);
      toast.showInfo('Nothing beat Current');
      return;
    }
    d.text = p.lines.join('\n');
    d.review = p;
    d.reviewedLines = p.lines.join('\n');
    d.reviewedAgainst = r.linesTunedFrom.join('\n');
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

function openSplit(d: MoodDraft): void {
  if (!d.split) return;
  const existing = drafts.value.find((x) => x.source === 'split');
  if (existing) removeDraft(selectedId.value, existing.key);
  const at = drafts.value.findIndex((x) => x.key === d.key) + 1;
  const s = addDraft(selectedId.value, { source: 'split', name: 'Split', split: d.split, reviewedAgainst: d.reviewedAgainst }, at);
  selectedKey.value = s.key;
}

// --- Use ---

const confirm = reactive({ open: false, title: '', lines: [] as string[], action: null as null | (() => void) });
const using = ref(false);

function askBreak(broken: Regression[], action: () => void): void {
  if (broken.length === 0) {
    action();
    return;
  }
  confirm.title = confirmTitle(broken.length);
  confirm.lines = broken.map((r) =>
    confirmLine(r.outcome.song.name, r.outcome.before, r.outcome.after, bandOf(r.outcome.song.entryId), r.leftTop),
  );
  confirm.action = action;
  confirm.open = true;
}

async function onUse(key: number | 'current'): Promise<void> {
  const d = drafts.value.find((x) => x.key === key);
  const m = mood.value;
  if (!d || !m || using.value) return;
  if (d.source === 'split') {
    if (!d.split) return;
    askBreak(splitRegressions(d), () => {
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
    });
    store.hydrate(out.state);
    converted = out.converted;
  } catch (e) {
    toast.showError(String(e));
    return;
  } finally {
    using.value = false;
  }
  const used = { name: d.name, source: d.source, text: d.text };
  const at = draftList(moodId).findIndex((x) => x.key === d.key);
  removeDraft(moodId, d.key);
  for (const b of draftList(moodId).filter((x) => x.source === 'before')) removeDraft(moodId, b.key);
  addDraft(moodId, { source: 'before', name: 'Before', text: oldLines.join('\n') }, 0);
  selectedKeyByMood[moodId] = 'current';
  toast.showSuccess(`${m.name} updated`, {
    label: 'Undo',
    run: () => void undo(moodId, review.lines, oldLines, freshBefore, converted, used, at),
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
  used: Pick<MoodDraft, 'name' | 'source' | 'text'>,
  at: number,
): Promise<void> {
  try {
    const out = await invoke<AcceptedLines>('accept_tuned_lines', {
      moodId,
      linesTunedFrom: newLines,
      lines: oldLines,
      holdingEntryIds: freshBefore,
      reachedBars: {},
    });
    store.hydrate(out.state);
    if (converted.length > 0) store.hydrate(await invoke<AppState>('restore_mood_flags', { rows: converted }));
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
const notUpdatedId = ref<string | null>(null);
const updating = computed(() => rescoringId.value !== null && rescoringId.value === selectedId.value);
let sawProgress = false;
let updatingTimer: number | undefined;
async function rescore(moodId: string): Promise<void> {
  rescoringId.value = moodId;
  notUpdatedId.value = null;
  sawProgress = false;
  try {
    await invoke('rescore_moods');
  } catch {
    rescoringId.value = null;
    notUpdatedId.value = moodId;
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
  if (p?.phase === 'error') {
    notUpdatedId.value = rescoringId.value;
    rescoringId.value = null;
  } else if (p) sawProgress = true;
  else if (sawProgress) rescoringId.value = null;
});
watch(autotag.completedRuns, () => {
  rescoringId.value = null;
});

/// The full Recalibrate (⋯ menu): clears and rescores every mood.
async function recalibrateAll(): Promise<void> {
  await store.command('recalibrate_moods');
}

// --- Never suggest (per-mood prefs) ---

const prefs = ref<Record<string, MoodTunePrefs>>({});
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
async function neverSuggest(line: string): Promise<void> {
  if (!prefsOk.value) return;
  const id = selectedId.value;
  const have = new Set(neverList.value.map((w) => w.toLowerCase()));
  const words = neverSuggestWords(line).filter((w) => !have.has(w.toLowerCase()));
  if (words.length === 0) return;
  try {
    prefs.value = await invoke<Record<string, MoodTunePrefs>>('add_never_suggest', { mood: id, words });
    toast.showInfo(`Never suggest: ${words.join(', ')}`, {
      label: 'Undo',
      run: () => void Promise.all(words.map((w) => removeNeverSuggest(id, w))),
    });
  } catch (e) {
    toast.showError(String(e));
  }
}
async function removeNeverSuggest(id: string, word: string): Promise<void> {
  try {
    prefs.value = await invoke<Record<string, MoodTunePrefs>>('remove_never_suggest', { mood: id, word });
  } catch (e) {
    toast.showError(String(e));
  }
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
  if (e.altKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
    e.preventDefault();
    const keys = columns.value.map((c) => c.key);
    const i = keys.indexOf(selectedKey.value);
    const next = keys[Math.max(0, Math.min(keys.length - 1, i + (e.key === 'ArrowLeft' ? -1 : 1)))];
    selectedKey.value = next;
    document.querySelector<HTMLElement>(`[data-col="${next}"] .col-head`)?.focus();
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
    } else if (d) void evaluate(selectedId.value, d);
    else void evaluateCurrent();
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
          {{ songCount }} songs · {{ flagCount(selectedId) }} flags
        </span>
        <span v-if="updating" class="updating text-caption text-medium-emphasis">
          <VProgressCircular indeterminate size="12" width="2" class="mr-1" />Updating bars…
        </span>
        <span v-else-if="notUpdatedId === selectedId" class="updating text-caption text-error">Bars not updated</span>
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
        <!-- Versions -->
        <div class="versions-row">
          <div class="versions-scroll">
            <MoodVersionColumn
              v-for="c in columns"
              :key="c.key"
              :ref="(el) => setColRef(c.key, el)"
              :col="c"
              :selected="c.key === selectedKey"
              :current-lines="mood.prompts"
              :mood-name="mood.name"
              :mood-name-of="moodNameOf"
              :other-mood-names="otherMoodNames"
              :never-suggest-off="!prefsOk"
              :busy="using"
              @select="selectedKey = c.key"
              @rename="(n) => { const d = drafts.find((x) => x.key === c.key); if (d) rename(d, n); }"
              @discard="discard(c.key)"
              @update:text="(t) => { const d = drafts.find((x) => x.key === c.key); if (d) onText(d, t); }"
              @commit="() => { const d = drafts.find((x) => x.key === c.key); if (d) onCommit(d); }"
              @fork="onFork"
              @use="onUse(c.key)"
              @cancel="cancelSearch(c.key as number)"
              @never-suggest="neverSuggest"
              @open-split="() => { const d = drafts.find((x) => x.key === c.key); if (d) openSplit(d); }"
            />
            <!-- Ghost last column: type a new version, or make one -->
            <section class="ghost-col" aria-label="New version">
              <textarea
                class="ghost-input"
                rows="2"
                placeholder="Type a version"
                :disabled="!canAdd"
                aria-label="Type a version"
                @input="onGhostInput"
              />
              <div class="ghost-actions">
                <span class="ghost-btn">
                  <VBtn size="small" variant="tonal" :disabled="!canAdd" @click="copySelected">Copy</VBtn>
                  <VTooltip activator="parent" location="top" open-on-focus>
                    {{ canAdd ? 'Copy the selected column (Ctrl+D)' : `${MAX_PAGE_VERSIONS} versions at most` }}
                  </VTooltip>
                </span>
                <span class="ghost-btn">
                  <VBtn size="small" variant="tonal" :disabled="!canSearch" @click="search('arrange')">Arrange</VBtn>
                  <VTooltip activator="parent" location="top" open-on-focus>
                    {{ flaggedIds.size === 0 ? 'Flag a few songs first' : "Search the selected column's words and Current's" }}
                  </VTooltip>
                </span>
                <span class="ghost-btn">
                  <VBtn size="small" variant="tonal" :disabled="!canSearch" @click="search('suggest')">Suggest</VBtn>
                  <VTooltip activator="parent" location="top" open-on-focus>
                    {{ flaggedIds.size === 0 ? 'Flag a few songs first' : 'Search new words too' }}
                  </VTooltip>
                </span>
              </div>
              <div v-if="!canAdd" class="ghost-cap text-caption text-medium-emphasis">{{ versionCount }}/{{ MAX_PAGE_VERSIONS }}</div>
            </section>
          </div>
        </div>

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
            :is-current="selectedColumn.kind === 'current'"
            :version-name="selectedColumn.name"
            :flagged-ids="flaggedIds"
            :high-ids="highIds"
          />
        </div>
      </template>
    </main>

    <!-- Dialogs -->
    <VDialog v-model="confirm.open" max-width="560">
      <VCard :title="confirm.title">
        <VCardText>
          <div v-for="l in confirm.lines" :key="l" class="text-body-2">{{ l }}</div>
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn variant="plain" @click="confirm.open = false">Cancel</VBtn>
          <VBtn color="error" variant="flat" @click="confirm.open = false; confirm.action?.()">Use anyway</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
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
.moods-page {
  height: calc(100vh - var(--v-layout-top, 0px) - var(--v-layout-bottom, 0px));
  display: grid;
  grid-template-columns: 200px minmax(0, 1fr);
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
  padding: 8px 12px 10px;
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
.versions-row {
  display: flex;
  gap: 8px;
  flex: 0 1 auto;
  min-height: 180px;
  height: 36%;
}
.versions-scroll {
  display: flex;
  gap: 8px;
  overflow-x: auto;
  overflow-y: hidden;
  min-width: 0;
  flex: 1 1 auto;
  align-items: stretch;
  padding-bottom: 4px;
  scroll-snap-type: x mandatory;
  /* Snapped and scrolled-to columns land right of the sticky Current. */
  scroll-padding-left: 288px;
}
.ghost-col {
  flex: 0 0 270px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border: 1px dashed rgba(var(--v-theme-on-surface), 0.2);
  border-radius: 6px;
  scroll-snap-align: start;
}
.ghost-input {
  resize: none;
  font: inherit;
  font-size: 0.875rem;
  color: inherit;
  background: transparent;
  border: 0;
  outline: none;
  min-height: 44px;
}
.ghost-input::placeholder {
  color: rgba(var(--v-theme-on-surface), 0.45);
}
.ghost-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
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
/* Narrow: the page scrolls; the two panels stack, each scrolling on its own. */
.narrow .mood-main {
  overflow-y: auto;
}
.narrow .versions-row {
  flex: 0 0 auto;
  height: 380px;
  max-height: none;
}
.narrow .bottom {
  flex: 0 0 auto;
  grid-template-columns: minmax(0, 1fr);
}
.narrow .panel {
  max-height: 360px;
}
.panel {
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
