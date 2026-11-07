<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open as openFile } from '@tauri-apps/plugin-dialog';
import type { AppState, CaseImportReport, EnergyPreview, MoodResetImpact, MoodVocabulary, ScryHit } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from '../composables/useToast';
import { calibrationLabel } from '../utils/moods';
import { albumOf, tuneTargetCount } from '../utils/moodTune';
import { newMoodId, WORD_BANK } from '../utils/moodWords';
import ConfirmDialog from './ConfirmDialog.vue';
import MoodTuneDialog from './MoodTuneDialog.vue';

/// Settings → Moods: the mood vocabulary the analyzer scores, and its
/// calibration.
///
/// - One row per mood: its name and its prompts, one per line. Each mood
///   also has an internal id (see `newMoodId`) that is never shown: stored
///   values, calibration and the analyzer key moods by it, so renaming a
///   mood keeps everything attached. Rows can be added, removed and
///   moved; Energy is always last, with its two sides of prompts.
/// - Save stores the vocabulary. A removed mood loses every value, so Save
///   first asks when that drops levels set by you. When prompts changed, it
///   then offers "Recalibrate now?".
/// - Recalibrate clears the calibration and every suggested mood value
///   (levels set by you stay) and starts an analysis run, which calibrates
///   again from the current prompts.
/// - The word bank above the editor inserts a tag word into the prompt field
///   that last had focus, as a line of its own.
/// - Reset to defaults puts back the shipped mood list (after a confirm that
///   counts what it deletes). It works even while the stored list is
///   unreadable, which Save refuses; afterwards it offers "Recalibrate now?"
///   when moods need it.
/// - Preview on a row lists the 15 tracks its current prompts (saved or not)
///   rank highest, by raw similarity, without saving or recalibrating, so a
///   prompt set can be checked before committing to it.
/// - Tune on a saved row opens the Tune dialog for that mood; "N flags" opens
///   it at the mood's Flags list. Accepting there offers "Recalibrate now?"
///   as a manual save does.
/// - Import case studies reads a case-study JSON file the user picks and
///   adds its cases as flags (one-off; see `import_case_studies`).
const open = defineModel<boolean>({ required: true });

const store = useAppStore();

interface DraftRow {
  /// Stable key for the list rendering.
  key: number;
  /// Internal, never shown or edited.
  id: string;
  name: string;
  prompts: string;
}

let nextKey = 0;
const rows = ref<DraftRow[]>([]);
const energyPositive = ref('');
const energyNegative = ref('');
const saving = ref(false);
const recalibrating = ref(false);

const lines = (text: string): string[] =>
  text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);

function load(v: MoodVocabulary): void {
  rows.value = v.moods.map((m) => ({ key: nextKey++, id: m.id, name: m.name, prompts: m.prompts.join('\n') }));
  energyPositive.value = v.energy.positive.join('\n');
  energyNegative.value = v.energy.negative.join('\n');
}

watch(open, (isOpen) => {
  if (isOpen) load(store.moodVocabulary);
}, { immediate: true });

const draft = computed<MoodVocabulary>(() => ({
  moods: rows.value.map((r) => ({ id: r.id, name: r.name.trim(), prompts: lines(r.prompts) })),
  energy: { positive: lines(energyPositive.value), negative: lines(energyNegative.value) },
}));

/// What is wrong with one row, or null. The backend checks the same rules.
function rowError(row: DraftRow, index: number): string | null {
  const name = row.name.trim();
  if (!name) return 'Give it a name';
  if (name.toLowerCase() === 'energy' || rows.value.some((r, i) => i !== index && r.name.trim().toLowerCase() === name.toLowerCase())) {
    return 'This name is used twice';
  }
  if (lines(row.prompts).length === 0) return 'Add at least one prompt';
  return null;
}

const errors = computed(() => rows.value.map((r, i) => rowError(r, i)));
const energyError = computed(() =>
  lines(energyPositive.value).length === 0 || lines(energyNegative.value).length === 0
    ? 'Energy needs at least one prompt on each side'
    : null,
);
const valid = computed(
  () => rows.value.length > 0 && errors.value.every((e) => e === null) && energyError.value === null,
);

const sameList = (a: string[], b: string[]): boolean => a.length === b.length && a.every((x, i) => x === b[i]);

const dirty = computed(() => JSON.stringify(draft.value) !== JSON.stringify(store.moodVocabulary));

/// Moods saved before but left out of the draft.
const removed = computed(() => {
  const kept = new Set(draft.value.moods.map((m) => m.id));
  return store.moodVocabulary.moods.filter((m) => !kept.has(m.id));
});

/// True when the draft scores something differently: a new mood, or changed
/// prompts (renaming and reordering do not count).
const promptsChanged = computed(() => {
  const before = new Map(store.moodVocabulary.moods.map((m) => [m.id, m.prompts]));
  const moodsChanged = draft.value.moods.some((m) => {
    const old = before.get(m.id);
    return old === undefined || !sameList(old, m.prompts);
  });
  const e = store.moodVocabulary.energy;
  return (
    moodsChanged ||
    !sameList(e.positive, draft.value.energy.positive) ||
    !sameList(e.negative, draft.value.energy.negative)
  );
});

/// Levels set by you that removing `id` would delete.
const userLevelCount = (id: string): number =>
  store.libraryEntries.filter((e) => e.moods.some((m) => m.mood === id && m.user !== null)).length;

const removedLevels = computed(() => removed.value.reduce((n, m) => n + userLevelCount(m.id), 0));

// --- Editing ---

function addMood(): void {
  const taken = new Set(rows.value.map((r) => r.id));
  let id = newMoodId();
  while (taken.has(id)) id = newMoodId();
  rows.value.push({ key: nextKey++, id, name: '', prompts: '' });
}

function removeRow(index: number): void {
  rows.value.splice(index, 1);
}

function move(index: number, step: -1 | 1): void {
  const to = index + step;
  if (to < 0 || to >= rows.value.length) return;
  const [row] = rows.value.splice(index, 1);
  rows.value.splice(to, 0, row);
}

// --- Word bank ---

/// The prompt field that last had focus: a mood row (by key) or a side of
/// Energy, with its textarea element.
const focused = ref<{ target: DraftRow | 'positive' | 'negative'; el: HTMLTextAreaElement } | null>(null);

function onPromptFocus(target: DraftRow | 'positive' | 'negative', e: FocusEvent): void {
  focused.value = { target, el: e.target as HTMLTextAreaElement };
}

function readField(target: DraftRow | 'positive' | 'negative'): string {
  if (target === 'positive') return energyPositive.value;
  if (target === 'negative') return energyNegative.value;
  return target.prompts;
}

function writeField(target: DraftRow | 'positive' | 'negative', text: string): void {
  if (target === 'positive') energyPositive.value = text;
  else if (target === 'negative') energyNegative.value = text;
  else target.prompts = text;
}

/// Puts `word` on its own line: on the caret's line when that line is empty,
/// otherwise on a new line right after it. The caret ends after the word.
function insertWord(word: string): void {
  const f = focused.value;
  if (!f) return;
  const text = readField(f.target);
  const caret = Math.min(f.el.selectionStart ?? text.length, text.length);
  const lineStart = text.lastIndexOf('\n', caret - 1) + 1;
  const nextBreak = text.indexOf('\n', caret);
  const lineEnd = nextBreak === -1 ? text.length : nextBreak;
  const lineEmpty = text.slice(lineStart, lineEnd).trim() === '';
  const next = lineEmpty
    ? text.slice(0, lineStart) + word + text.slice(lineEnd)
    : text.slice(0, lineEnd) + '\n' + word + text.slice(lineEnd);
  const caretAfter = (lineEmpty ? lineStart : lineEnd + 1) + word.length;
  writeField(f.target, next);
  requestAnimationFrame(() => {
    f.el.focus();
    f.el.setSelectionRange(caretAfter, caretAfter);
  });
}

// --- Preview ---

interface PreviewState {
  loading: boolean;
  error: string | null;
  /// The prompts the list was made from, to tell when it is out of date.
  prompts: string[];
  tracks: { id: string; name: string; album: string; score: number }[];
}

/// Preview results per row key; a row without an entry shows none.
const previews = reactive(new Map<number, PreviewState>());

async function preview(row: DraftRow): Promise<void> {
  const prompts = lines(row.prompts);
  if (prompts.length === 0) return;
  // Reactive, so the updates below reach the template through the map.
  const state = reactive<PreviewState>({ loading: true, error: null, prompts, tracks: [] });
  previews.set(row.key, state);
  try {
    const hits = await invoke<ScryHit[]>('preview_mood_prompts', { prompts });
    if (previews.get(row.key) !== state) return;
    state.tracks = hits.map((h) => {
      const entry = store.findLibraryEntry(h.entryId);
      return { id: h.entryId, name: entry?.name ?? h.entryId, album: entry ? albumOf(entry.filePath) : '', score: h.score };
    });
  } catch (e) {
    if (previews.get(row.key) === state) state.error = String(e);
  } finally {
    state.loading = false;
  }
}

const previewStale = (row: DraftRow, state: PreviewState): boolean => !sameList(lines(row.prompts), state.prompts);

/// The Energy preview: the most and least energetic tracks under the two
/// sides as written (saved or not), raw difference of similarities.
interface EnergyPreviewState {
  loading: boolean;
  error: string | null;
  positive: string[];
  negative: string[];
  most: PreviewState['tracks'];
  least: PreviewState['tracks'];
}
const energyPreview = ref<EnergyPreviewState | null>(null);

async function previewEnergy(): Promise<void> {
  const positive = lines(energyPositive.value);
  const negative = lines(energyNegative.value);
  if (positive.length === 0 || negative.length === 0) return;
  const state = reactive<EnergyPreviewState>({ loading: true, error: null, positive, negative, most: [], least: [] });
  energyPreview.value = state;
  const toTracks = (hits: ScryHit[]): PreviewState['tracks'] =>
    hits.map((h) => {
      const entry = store.findLibraryEntry(h.entryId);
      return { id: h.entryId, name: entry?.name ?? h.entryId, album: entry ? albumOf(entry.filePath) : '', score: h.score };
    });
  try {
    const result = await invoke<EnergyPreview>('preview_energy_prompts', { positive, negative });
    if (energyPreview.value !== state) return;
    state.most = toTracks(result.most);
    state.least = toTracks(result.least);
  } catch (e) {
    if (energyPreview.value === state) state.error = String(e);
  } finally {
    state.loading = false;
  }
}

const energyPreviewStale = computed(
  () =>
    energyPreview.value !== null &&
    (!sameList(lines(energyPositive.value), energyPreview.value.positive) ||
      !sameList(lines(energyNegative.value), energyPreview.value.negative)),
);

// --- Save and recalibrate ---

const confirmRemove = ref(false);
const askRecalibrate = ref(false);
const confirmRecalibrate = ref(false);

function onSave(): void {
  if (!valid.value || !dirty.value || store.moodVocabularyUnreadable) return;
  if (removedLevels.value > 0) confirmRemove.value = true;
  else void save();
}

async function save(): Promise<void> {
  const changedPrompts = promptsChanged.value;
  saving.value = true;
  const ok = await store.command('set_mood_vocabulary', { vocabulary: draft.value });
  saving.value = false;
  if (!ok) return;
  load(store.moodVocabulary);
  if (changedPrompts) askRecalibrate.value = true;
}

async function recalibrate(): Promise<void> {
  recalibrating.value = true;
  await store.command('recalibrate_moods');
  recalibrating.value = false;
}

// --- Reset to defaults ---

const resetImpact = ref<MoodResetImpact | null>(null);
const confirmReset = ref(false);
const resetting = ref(false);

/// Asks the backend what a reset would delete (counted from the stored rows,
/// so it is right even when the stored list is unreadable), then confirms.
async function onReset(): Promise<void> {
  try {
    resetImpact.value = await invoke<MoodResetImpact>('mood_reset_impact');
    confirmReset.value = true;
  } catch (e) {
    resetImpact.value = null;
    useToast().showError(String(e));
  }
}

async function reset(): Promise<void> {
  resetting.value = true;
  const ok = await store.command('reset_mood_vocabulary');
  resetting.value = false;
  if (!ok) return;
  previews.clear();
  energyPreview.value = null;
  load(store.moodVocabulary);
  if (store.moodsNeedRecalibration) askRecalibrate.value = true;
}

const resetMessage = computed(() => {
  const i = resetImpact.value;
  const parts = [
    'This replaces the mood list with the shipped defaults, including their prompts and the Energy prompts.',
    'Levels set by you are kept for moods that are still in the list.',
  ];
  if (i && i.rows > 0) {
    const levels = i.userLevels === 1 ? '1 level set by you' : `${i.userLevels} levels set by you`;
    const values = i.rows === 1 ? '1 stored value' : `${i.rows} stored values`;
    parts.push(`Moods not in the defaults (${i.moods.join(', ')}) are deleted: ${values}, including ${levels}.`);
  } else {
    parts.push('No stored values are deleted.');
  }
  parts.push('Moods whose prompts change will need a recalibration.');
  if (dirty.value) parts.push('Unsaved edits in this panel are discarded.');
  return parts.join(' ');
});

const removeMessage = computed(() => {
  const names = removed.value.map((m) => m.name).join(', ');
  const n = removedLevels.value;
  return `Removing ${names} deletes its values on every track, including ${n} ${n === 1 ? 'level' : 'levels'} set by you.`;
});

const calibratedLabel = computed(() => calibrationLabel(store.moodCalibration));

// --- Tune ---

const tuneOpen = ref(false);
const tuneMoodId = ref<string | null>(null);
const tuneShowFlags = ref(false);

/// Tune targets per saved mood, for each row's "N flags" link: flags plus
/// hand-set levels without a flag (Tune reads those as flags too).
const flagCount = (id: string): number => tuneTargetCount(store.libraryEntries, id);

/// The saved prompts of a row's mood, or null for a mood not saved yet.
const savedPrompts = (row: DraftRow): string[] | null =>
  store.moodVocabulary.moods.find((m) => m.id === row.id)?.prompts ?? null;

/// Tune works on the saved lines, so a row with unsaved prompts must be
/// saved (or discarded) first.
const tuneBlocked = (row: DraftRow): boolean => {
  const saved = savedPrompts(row);
  return saved !== null && !sameList(saved, lines(row.prompts));
};

function openTune(row: DraftRow, showFlags: boolean): void {
  tuneMoodId.value = row.id;
  tuneShowFlags.value = showFlags;
  tuneOpen.value = true;
}

/// After an accept: the tuned mood's row takes its new saved lines, and the
/// mood a split added (`newMoodId`) gets a row; other unsaved edits, removed
/// rows included, stay as they are. Then the same "Recalibrate now?" offer as
/// a manual save.
function onTuneAccepted(newMoodId: string | null): void {
  const tuned = store.moodVocabulary.moods.find((m) => m.id === tuneMoodId.value);
  const row = rows.value.find((r) => r.id === tuneMoodId.value);
  if (tuned && row) row.prompts = tuned.prompts.join('\n');
  const added = newMoodId === null ? undefined : store.moodVocabulary.moods.find((m) => m.id === newMoodId);
  if (added && !rows.value.some((r) => r.id === added.id)) {
    rows.value.push({ key: nextKey++, id: added.id, name: added.name, prompts: added.prompts.join('\n') });
  }
  askRecalibrate.value = true;
}

// --- Case-study import ---

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
    // The import does not return the app state: reload it so the new flags show.
    store.hydrate(await invoke<AppState>('load_data'));
  } catch (e) {
    useToast().showError(String(e));
  } finally {
    importing.value = false;
  }
}
</script>

<template>
  <VDialog v-model="open" max-width="860" scrollable>
    <VCard title="Moods" prepend-icon="mdi-palette-outline">
      <VCardText>
        <!-- Calibration -->
        <div class="d-flex align-center ga-3 mb-2">
          <div class="text-body-2">{{ calibratedLabel }}</div>
          <VSpacer />
          <VBtn
            variant="text"
            size="small"
            prepend-icon="mdi-file-import-outline"
            :loading="importing"
            @click="importCaseStudies"
          >
            Import case studies…
          </VBtn>
          <VBtn
            variant="tonal"
            size="small"
            prepend-icon="mdi-tune-variant"
            :loading="recalibrating"
            @click="confirmRecalibrate = true"
          >
            Recalibrate
          </VBtn>
        </div>
        <VAlert v-if="store.moodVocabularyUnreadable" type="error" variant="tonal" density="compact" class="mb-3">
          The stored mood list is unreadable, so the default moods are shown. Saving is blocked until the stored list
          is fixed or reset, so your own levels for other moods are not deleted. Reset to defaults below writes the
          shipped list.
        </VAlert>
        <VAlert v-if="store.moodsNeedRecalibration" type="warning" variant="tonal" density="compact" class="mb-3">
          Some prompts changed since the last calibration. Those moods are not suggested again until the next analysis run or a Recalibrate.
        </VAlert>

        <VDivider class="my-3" />

        <!-- Word bank: stays in view while the rows below scroll -->
        <div class="word-bank-bar">
          <div class="text-subtitle-2 mb-1">Word bank</div>
          <div class="d-flex flex-wrap ga-1 mb-1 word-bank">
            <VChip
              v-for="word in WORD_BANK"
              :key="word"
              size="small"
              :disabled="!focused"
              @mousedown.prevent
              @click="insertWord(word)"
            >
              {{ word }}
            </VChip>
          </div>
          <div class="text-caption text-medium-emphasis mb-2">
            Single tag words work as well as sentences; the analyzer was trained on tag-like text.
            <template v-if="!focused">Click into a prompt field, then a word to add it.</template>
          </div>
        </div>

        <!-- Moods -->
        <div class="mood-table">
          <div class="mood-head text-caption text-medium-emphasis">
            <span />
            <span>Name</span>
            <span>Prompts (one per line)</span>
            <span />
          </div>
          <div v-for="(row, i) in rows" :key="row.key" class="mood-row" :data-mood="row.id">
            <div class="d-flex flex-column">
              <VBtn
                icon="mdi-chevron-up"
                size="x-small"
                variant="text"
                :disabled="i === 0"
                :aria-label="`Move ${row.name || 'mood'} up`"
                @click="move(i, -1)"
              />
              <VBtn
                icon="mdi-chevron-down"
                size="x-small"
                variant="text"
                :disabled="i === rows.length - 1"
                :aria-label="`Move ${row.name || 'mood'} down`"
                @click="move(i, 1)"
              />
            </div>
            <VTextField
              v-model="row.name"
              variant="outlined"
              density="compact"
              hide-details
              aria-label="Name"
            />
            <div>
              <VTextarea
                v-model="row.prompts"
                variant="outlined"
                density="compact"
                rows="2"
                auto-grow
                hide-details
                aria-label="Prompts"
                @focus="(e: FocusEvent) => onPromptFocus(row, e)"
              />
              <div v-if="errors[i]" class="text-caption text-error mt-1">{{ errors[i] }}</div>
              <div class="d-flex align-center ga-2 mt-1">
                <VBtn
                  size="x-small"
                  variant="tonal"
                  prepend-icon="mdi-eye-outline"
                  :loading="previews.get(row.key)?.loading"
                  :disabled="lines(row.prompts).length === 0"
                  @click="preview(row)"
                >
                  Preview
                </VBtn>
                <template v-if="savedPrompts(row) !== null">
                  <VBtn
                    size="x-small"
                    variant="tonal"
                    prepend-icon="mdi-tune-vertical"
                    :disabled="tuneBlocked(row)"
                    @click="openTune(row, false)"
                  >
                    Tune
                  </VBtn>
                  <a href="#" class="text-caption flags-link" @click.prevent="openTune(row, true)">
                    {{ flagCount(row.id) }} flag{{ flagCount(row.id) === 1 ? '' : 's' }}
                  </a>
                  <span v-if="tuneBlocked(row)" class="text-caption text-medium-emphasis">Save the prompts to tune</span>
                </template>
                <span v-if="previews.get(row.key)" class="text-caption text-medium-emphasis">
                  <template v-if="previewStale(row, previews.get(row.key)!)">Prompts changed since this preview</template>
                  <template v-else>Top tracks for these prompts, raw similarity (not saved)</template>
                </span>
                <VSpacer />
                <VBtn
                  v-if="previews.get(row.key)"
                  icon="mdi-close"
                  size="x-small"
                  variant="text"
                  aria-label="Close preview"
                  @click="previews.delete(row.key)"
                />
              </div>
              <div v-if="previews.get(row.key)?.error" class="text-caption text-error mt-1">
                {{ previews.get(row.key)!.error }}
              </div>
              <ol
                v-else-if="previews.get(row.key) && !previews.get(row.key)!.loading"
                class="preview-list text-caption mt-1"
                :class="{ stale: previewStale(row, previews.get(row.key)!) }"
              >
                <li v-for="t in previews.get(row.key)!.tracks" :key="t.id">
                  <span class="preview-score">{{ t.score.toFixed(3) }}</span>
                  <span class="preview-name">{{ t.name }}</span>
                  <span v-if="t.album" class="text-medium-emphasis"> · {{ t.album }}</span>
                </li>
                <li v-if="previews.get(row.key)!.tracks.length === 0" class="text-medium-emphasis">
                  No analyzed music yet.
                </li>
              </ol>
            </div>
            <VBtn
              icon="mdi-delete-outline"
              size="small"
              variant="text"
              :disabled="rows.length === 1"
              :aria-label="`Remove ${row.name || 'mood'}`"
              @click="removeRow(i)"
            />
          </div>
          <VBtn class="mt-2" size="small" variant="text" prepend-icon="mdi-plus" @click="addMood">Add mood</VBtn>

          <!-- Energy: fixed, last, two-sided -->
          <div class="text-subtitle-2 mt-4 mb-1">
            <VIcon size="small">mdi-lightning-bolt</VIcon> Energy
          </div>
          <div class="text-caption text-medium-emphasis mb-2">
            Scored two-sided: how much a track matches the energetic prompts minus the calm ones.
          </div>
          <div class="d-flex ga-3">
            <VTextarea
              v-model="energyPositive"
              label="Energetic (one per line)"
              variant="outlined"
              density="compact"
              rows="2"
              auto-grow
              hide-details
              @focus="(e: FocusEvent) => onPromptFocus('positive', e)"
            />
            <VTextarea
              v-model="energyNegative"
              label="Calm (one per line)"
              variant="outlined"
              density="compact"
              rows="2"
              auto-grow
              hide-details
              @focus="(e: FocusEvent) => onPromptFocus('negative', e)"
            />
          </div>
          <div v-if="energyError" class="text-caption text-error mt-1">{{ energyError }}</div>
          <div class="d-flex align-center ga-2 mt-1">
            <VBtn
              size="x-small"
              variant="tonal"
              prepend-icon="mdi-eye-outline"
              :loading="energyPreview?.loading"
              :disabled="energyError !== null"
              @click="previewEnergy"
            >
              Preview
            </VBtn>
            <span v-if="energyPreview" class="text-caption text-medium-emphasis">
              <template v-if="energyPreviewStale">Prompts changed since this preview</template>
              <template v-else>Most and least energetic tracks for these prompts, raw difference (not saved)</template>
            </span>
            <VSpacer />
            <VBtn
              v-if="energyPreview"
              icon="mdi-close"
              size="x-small"
              variant="text"
              aria-label="Close Energy preview"
              @click="energyPreview = null"
            />
          </div>
          <div v-if="energyPreview?.error" class="text-caption text-error mt-1">{{ energyPreview.error }}</div>
          <div v-else-if="energyPreview && !energyPreview.loading" class="d-flex ga-3 mt-1" :class="{ stale: energyPreviewStale }">
            <div class="flex-1-1-0">
              <div class="text-caption font-weight-medium">Most energetic</div>
              <ol class="preview-list text-caption" :class="{ stale: energyPreviewStale }">
                <li v-for="t in energyPreview.most" :key="t.id">
                  <span class="preview-score">{{ t.score.toFixed(3) }}</span>
                  <span class="preview-name">{{ t.name }}</span>
                  <span v-if="t.album" class="text-medium-emphasis"> · {{ t.album }}</span>
                </li>
                <li v-if="energyPreview.most.length === 0" class="text-medium-emphasis">No analyzed music yet.</li>
              </ol>
            </div>
            <div class="flex-1-1-0">
              <div class="text-caption font-weight-medium">Least energetic</div>
              <ol class="preview-list text-caption" :class="{ stale: energyPreviewStale }">
                <li v-for="t in energyPreview.least" :key="t.id">
                  <span class="preview-score">{{ t.score.toFixed(3) }}</span>
                  <span class="preview-name">{{ t.name }}</span>
                  <span v-if="t.album" class="text-medium-emphasis"> · {{ t.album }}</span>
                </li>
              </ol>
            </div>
          </div>
        </div>
      </VCardText>
      <VDivider />
      <VCardActions>
        <VBtn text="Discard changes" variant="plain" :disabled="!dirty" @click="load(store.moodVocabulary)" />
        <VBtn
          text="Reset to defaults"
          variant="plain"
          prepend-icon="mdi-restore"
          :color="store.moodVocabularyUnreadable ? 'error' : undefined"
          :loading="resetting"
          @click="onReset"
        />
        <VSpacer />
        <VBtn text="Close" variant="plain" @click="open = false" />
        <VBtn
          text="Save"
          color="primary"
          variant="tonal"
          :disabled="!valid || !dirty || store.moodVocabularyUnreadable"
          :loading="saving"
          @click="onSave"
        />
      </VCardActions>
    </VCard>

    <ConfirmDialog
      v-model="confirmRemove"
      title="Remove moods?"
      :message="removeMessage"
      confirm-label="Remove and save"
      confirm-color="error"
      @confirm="save"
    />
    <ConfirmDialog
      v-model="confirmReset"
      title="Reset moods to defaults?"
      :message="resetMessage"
      confirm-label="Reset"
      confirm-color="error"
      max-width="520"
      @confirm="reset"
    />
    <ConfirmDialog
      v-model="askRecalibrate"
      title="Recalibrate now?"
      message="The prompts changed. Recalibrating suggests every mood again from the new prompts; levels set by you stay. Until then, changed moods keep their old suggestions."
      confirm-label="Recalibrate"
      cancel-label="Later"
      @confirm="recalibrate"
    />
    <ConfirmDialog
      v-model="confirmRecalibrate"
      title="Recalibrate moods?"
      message="This clears the calibration and every suggested mood value, then analyzes the library again. The new calibration comes from the analyzer's reference set when it has one, otherwise from your library. Levels set by you stay."
      confirm-label="Recalibrate"
      @confirm="recalibrate"
    />
    <MoodTuneDialog
      v-model="tuneOpen"
      :mood-id="tuneMoodId"
      :show-flags="tuneShowFlags"
      @accepted="onTuneAccepted"
    />
    <VDialog v-model="importReportOpen" max-width="520">
      <VCard v-if="importReport" title="Case studies imported">
        <VCardText>
          <div class="mb-2">
            {{ importReport.imported }} flag{{ importReport.imported === 1 ? '' : 's' }} imported.
          </div>
          <div v-if="importReport.unmatched.length > 0" class="mb-2">
            <div class="text-subtitle-2">Not found in the library ({{ importReport.unmatched.length }})</div>
            <ul class="text-caption import-list">
              <li v-for="name in importReport.unmatched" :key="name">{{ name }}</li>
            </ul>
          </div>
          <div v-if="importReport.skippedMoods.length > 0">
            <div class="text-subtitle-2">Moods not in the list, skipped</div>
            <div class="text-caption">{{ importReport.skippedMoods.join(', ') }}</div>
          </div>
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Close" variant="plain" @click="importReportOpen = false" />
        </VCardActions>
      </VCard>
    </VDialog>
  </VDialog>
</template>

<style scoped>
.mood-head,
.mood-row {
  display: grid;
  grid-template-columns: 32px 180px 1fr 40px;
  gap: 8px;
  align-items: start;
}

.mood-head {
  margin-bottom: 4px;
}

.mood-row {
  margin-bottom: 8px;
}

.word-bank-bar {
  position: sticky;
  top: 0;
  z-index: 1;
  padding-top: 4px;
  padding-bottom: 4px;
  background: rgb(var(--v-theme-surface));
}

.preview-list {
  padding-left: 22px;
  margin: 0;
}

.preview-list.stale {
  opacity: 0.5;
}

.preview-score {
  display: inline-block;
  min-width: 3.2em;
  font-variant-numeric: tabular-nums;
  opacity: 0.7;
}

.flags-link {
  color: inherit;
  opacity: 0.8;
}

.import-list {
  padding-left: 20px;
  margin: 0;
}

.word-bank .v-chip {
  cursor: pointer;
}
</style>
