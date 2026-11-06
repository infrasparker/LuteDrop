<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { MoodVocabulary, ScryHit } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import ConfirmDialog from './ConfirmDialog.vue';

/// Settings → Moods: the mood vocabulary the analyzer scores, and its
/// calibration.
///
/// - One row per mood: its id (lowercase letters, fixed once saved), its
///   name, and its prompts, one per line. Rows can be added, removed and
///   moved; Energy is always last, with its two sides of prompts.
/// - Save stores the vocabulary. A removed mood loses every value, so Save
///   first asks when that drops levels set by you. When prompts changed, it
///   then offers "Recalibrate now?".
/// - Recalibrate clears the calibration and every suggested mood value
///   (levels set by you stay) and starts an analysis run, which calibrates
///   again from the current prompts.
/// - The word bank above the editor inserts a tag word into the prompt field
///   that last had focus, as a line of its own.
/// - Preview on a row lists the 15 tracks its current prompts (saved or not)
///   rank highest, by raw similarity, without saving or recalibrating, so a
///   prompt set can be checked before committing to it.
const open = defineModel<boolean>({ required: true });

const store = useAppStore();

/// Tag words the analyzer's text model knows well (MTG-Jamendo moods and
/// themes).
const WORD_BANK = [
  'action', 'adventure', 'ambiental', 'calm', 'dark', 'deep', 'dramatic', 'dream', 'emotional',
  'energetic', 'epic', 'fast', 'fun', 'funny', 'groovy', 'happy', 'heavy', 'hopeful', 'horror',
  'inspiring', 'meditative', 'melancholic', 'mellow', 'melodic', 'motivational', 'nature', 'party',
  'positive', 'powerful', 'relaxing', 'sad', 'slow', 'soft', 'soundscape', 'space', 'travel',
  'upbeat', 'uplifting',
];

const ID_PATTERN = /^[a-z]+$/;

interface DraftRow {
  /// Stable key for the list while ids are being typed.
  key: number;
  id: string;
  name: string;
  prompts: string;
  /// True for a row added in this session: its id can still be edited.
  isNew: boolean;
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
  rows.value = v.moods.map((m) => ({ key: nextKey++, id: m.id, name: m.name, prompts: m.prompts.join('\n'), isNew: false }));
  energyPositive.value = v.energy.positive.join('\n');
  energyNegative.value = v.energy.negative.join('\n');
}

watch(open, (isOpen) => {
  if (isOpen) load(store.moodVocabulary);
}, { immediate: true });

const draft = computed<MoodVocabulary>(() => ({
  moods: rows.value.map((r) => ({ id: r.id.trim(), name: r.name.trim(), prompts: lines(r.prompts) })),
  energy: { positive: lines(energyPositive.value), negative: lines(energyNegative.value) },
}));

/// What is wrong with one row, or null. The backend checks the same rules.
function rowError(row: DraftRow, index: number): string | null {
  const id = row.id.trim();
  if (!ID_PATTERN.test(id)) return 'Id: lowercase letters a–z only';
  if (id === 'energy') return 'The id "energy" is kept for Energy';
  if (rows.value.some((r, i) => i !== index && r.id.trim() === id)) return 'This id is used twice';
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
  rows.value.push({ key: nextKey++, id: '', name: '', prompts: '', isNew: true });
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

/// A new row's id follows its name until the id is typed by hand.
function onNameInput(row: DraftRow, name: string): void {
  const derived = (n: string): string => n.toLowerCase().replace(/[^a-z]/g, '');
  if (row.isNew && row.id === derived(row.name)) row.id = derived(name);
  row.name = name;
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

/// "Artist / Album" from the file's two parent folders.
function albumOf(filePath: string): string {
  const parts = filePath.split(/[\\/]/).filter((p) => p.length > 0 && !/^[A-Za-z]:$/.test(p));
  return [parts[parts.length - 3], parts[parts.length - 2]].filter(Boolean).join(' / ');
}

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

const removeMessage = computed(() => {
  const names = removed.value.map((m) => m.name).join(', ');
  const n = removedLevels.value;
  return `Removing ${names} deletes its values on every track, including ${n} ${n === 1 ? 'level' : 'levels'} set by you.`;
});

const calibratedLabel = computed(() => {
  const c = store.moodCalibration;
  if (!c) return 'Not calibrated yet. The next analysis run calibrates once it covers 50 music tracks.';
  const date = new Date(Number(c.calibratedAt)).toLocaleString();
  return `Last calibrated: ${date}, ${c.tracks} tracks`;
});
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
          is fixed or reset, so your own levels for other moods are not deleted.
        </VAlert>
        <VAlert v-if="store.moodsNeedRecalibration" type="warning" variant="tonal" density="compact" class="mb-3">
          Some prompts changed since the last calibration. Those moods are not suggested again until you recalibrate.
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
            <span>Id</span>
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
              v-model="row.id"
              :readonly="!row.isNew"
              :title="row.isNew ? undefined : 'An id cannot change once saved'"
              variant="outlined"
              density="compact"
              hide-details
              aria-label="Id"
            />
            <VTextField
              :model-value="row.name"
              variant="outlined"
              density="compact"
              hide-details
              aria-label="Name"
              @update:model-value="(v: string) => onNameInput(row, v)"
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
        </div>
      </VCardText>
      <VDivider />
      <VCardActions>
        <VBtn text="Discard changes" variant="plain" :disabled="!dirty" @click="load(store.moodVocabulary)" />
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
      message="This clears the calibration and every suggested mood value, then analyzes the library again to suggest them from the current prompts. Levels set by you stay."
      confirm-label="Recalibrate"
      @confirm="recalibrate"
    />
  </VDialog>
</template>

<style scoped>
.mood-head,
.mood-row {
  display: grid;
  grid-template-columns: 32px 130px 150px 1fr 40px;
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

.word-bank .v-chip {
  cursor: pointer;
}
</style>
