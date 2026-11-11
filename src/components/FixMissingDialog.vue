<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../stores/appStore';
import { useToast } from '../composables/useToast';
import { AUDIO_EXTENSIONS } from '../constants';
import ConfirmDialog from './ConfirmDialog.vue';
import type { FixMissingReport, LibraryEntryUsage, MatchKind } from '@generated/index';

const props = defineProps<{
  modelValue: boolean;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const store = useAppStore();
const toast = useToast();

// --- State ---

// The latest scan result from the backend (null until the first scan finishes).
const report = ref<FixMissingReport | null>(null);
// True while a scan is running (can take a few seconds on a network share).
const loading = ref(false);
// Which missing entry we are looking at (0-based position in report.entries).
const index = ref(0);
// The candidate file path the user has clicked in the list, if any.
const selectedPath = ref<string | null>(null);
// True while a relink call is running (it hashes the new file, which can be slow).
// Used to block clicks so one relink cannot be started twice.
const relinking = ref(false);
// Message from the last failed scan, or null. Shown instead of the all-clear text.
const loadError = ref<string | null>(null);
// Running totals for the summary screen at the end.
const relinked = ref(0);
const skipped = ref(0);
const removed = ref(0);

// Human-readable labels for the match-quality chip on each candidate.
// Bumped every time the dialog closes or a new scan/relink starts. A backend call
// remembers the number it started with and drops its result if the number has
// changed, so a slow answer from an earlier pass cannot touch the current one.
let requestId = 0;

const KIND_LABELS: Record<MatchKind, string> = {
  exact: 'Exact',
  contains: 'Contains',
  similar: 'Similar',
};

// The entry currently being fixed, or undefined when we are past the last one.
const current = computed(() => report.value?.entries[index.value]);

// --- Loading and scanning ---

// Each time the dialog opens, start from scratch and scan with the remembered
// folder (an empty string tells the backend to pick the folder itself).
watch(
  () => props.modelValue,
  (isOpen) => {
    if (!isOpen) {
      requestId++; // drop any scan or relink still in flight
      return;
    }
    relinked.value = 0;
    skipped.value = 0;
    removed.value = 0;
    index.value = 0;
    selectedPath.value = null;
    void load('');
  },
);

// Ask the backend for candidates under `folder`, then show the first entry.
async function load(folder: string): Promise<void> {
  const id = ++requestId;
  loading.value = true;
  loadError.value = null;
  try {
    const result = await invoke<FixMissingReport>('find_missing_file_candidates', { folder });
    if (id !== requestId) return; // dialog was closed or a newer scan started
    report.value = result;
    // A fresh report starts a fresh pass, so the counters start over too.
    relinked.value = 0;
    skipped.value = 0;
    removed.value = 0;
    index.value = 0;
    selectedPath.value = null;
    preselect();
  } catch (e) {
    if (id !== requestId) return;
    loadError.value = String(e);
    toast.showError(`Could not scan for missing files: ${e}`);
  } finally {
    if (id === requestId) loading.value = false;
  }
}

// If the current entry has exactly one unused Exact match, tick it for the user
// so the common case is a single click on Relink.
function preselect(): void {
  const unusedExact = (current.value?.candidates ?? []).filter(
    (c) => c.kind === 'exact' && !c.usedByEntryName,
  );
  selectedPath.value = unusedExact.length === 1 ? unusedExact[0].filePath : null;
}

// --- Folder choice ---

// Let the user pick a different folder to search, then rescan it.
async function changeFolder(): Promise<void> {
  const path = await open({ directory: true });
  if (!path || Array.isArray(path)) return;
  await load(path);
}

// Scan the same folder again (e.g. after the user copied files into it).
async function rescan(): Promise<void> {
  await load(report.value?.searchRoot ?? '');
}

// --- Fixing entries ---

// Point the current entry at `path`. store.command shows its own error toast
// and does not throw, so success is detected by checking the entry afterwards.
async function relink(path: string): Promise<void> {
  const entryId = current.value?.entry.id;
  if (!entryId || relinking.value) return;
  const id = ++requestId;
  relinking.value = true;
  try {
    await store.command('relocate_library_entry', { entryId, newFilePath: path });
  } finally {
    relinking.value = false;
  }
  if (id !== requestId) return; // dialog was closed or reopened meanwhile
  const updated = store.findLibraryEntry(entryId);
  if (updated?.filePath !== path) return; // failed; stay on this entry
  if (store.isEntryMissing(entryId)) {
    // The path was saved but the file is empty or unreadable, so it is still missing.
    toast.showError('That file is empty or unreadable; the entry is still missing.');
    return;
  }
  relinked.value++;
  // Mark the file as taken in the other entries' lists, because the scan
  // result is stale and would otherwise let a second entry pick the same file.
  for (const other of report.value?.entries ?? []) {
    for (const c of other.candidates) {
      if (c.filePath === path) c.usedByEntryName = updated.name;
    }
  }
  advance();
}

// Let the user choose any audio file by hand instead of using the list.
async function browse(): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
  });
  if (!path || Array.isArray(path)) return;
  await relink(path);
}

// Leave this entry missing and move on.
function skip(): void {
  if (relinking.value) return;
  skipped.value++;
  advance();
}

// --- Removing an entry ---

// Shown while the user is being asked to confirm removing an entry that
// clips still use.
const showRemoveConfirm = ref(false);
const removeUsage = ref<LibraryEntryUsage[]>([]);

// Delete the current entry from the library instead of relinking it. If any
// clips use it, ask first, because deleting the entry deletes those clips too
// (same rule as the Library view's delete button).
async function remove(): Promise<void> {
  const entryId = current.value?.entry.id;
  if (!entryId || relinking.value) return;
  let usage: LibraryEntryUsage[];
  try {
    usage = await invoke<LibraryEntryUsage[]>('get_library_entry_usage', { entryId });
  } catch (e) {
    toast.showError(`Could not check whether "${current.value?.entry.name}" is in use: ${e}`);
    return;
  }
  if (usage.length === 0) {
    await performRemove();
    return;
  }
  removeUsage.value = usage;
  showRemoveConfirm.value = true;
}

async function performRemove(): Promise<void> {
  const entryId = current.value?.entry.id;
  if (!entryId) return;
  const id = ++requestId;
  relinking.value = true; // reuse the busy flag so the other buttons lock too
  try {
    await store.command('delete_library_entry', { entryId });
  } finally {
    relinking.value = false;
  }
  removeUsage.value = [];
  if (id !== requestId) return;
  if (store.findLibraryEntry(entryId)) return; // delete failed; stay here
  removed.value++;
  // Drop the entry from the report so "Entry X of N" stays truthful; the
  // next entry slides into the current index.
  if (report.value) {
    report.value.entries = report.value.entries.filter((e) => e.entry.id !== entryId);
  }
  selectedPath.value = null;
  preselect();
}

// Move to the next missing entry and pre-select its best match.
function advance(): void {
  index.value++;
  selectedPath.value = null;
  preselect();
}

// Close without saving anything extra (relinks already happened one by one).
function close(): void {
  emit('update:modelValue', false);
}
</script>

<template>
  <VDialog :model-value="modelValue" max-width="640" @update:model-value="(v) => emit('update:modelValue', v)">
    <VCard title="Fix missing files">
      <VCardText>
        <!-- Header: which folder we are searching, and buttons to change it -->
        <div class="d-flex align-center ga-2 mb-2">
          <span class="text-caption text-medium-emphasis">Search folder:</span>
          <span
            class="text-caption flex-grow-1 text-truncate"
            style="direction: rtl; text-align: left;"
            :title="report?.searchRoot"
          >{{ report?.searchRoot || 'none chosen' }}</span>
          <VBtn size="x-small" variant="tonal" prepend-icon="mdi-folder-search" :disabled="loading || relinking" @click="changeFolder">
            Change...
          </VBtn>
          <VBtn size="x-small" variant="text" prepend-icon="mdi-refresh" :disabled="loading || relinking" @click="rescan">
            Rescan
          </VBtn>
        </div>

        <!-- Warning: the folder is gone (e.g. an unplugged drive) -->
        <VAlert
          v-if="report && !report.searchRootExists"
          type="warning"
          variant="tonal"
          density="compact"
          class="mb-3"
        >
          {{ report.searchRoot === '' ? 'No folder to search yet. Choose a folder.' : 'Folder not found. Choose a folder to search.' }}
        </VAlert>

        <!-- Loading: scan in progress -->
        <div v-if="loading" class="d-flex justify-center pa-8">
          <VProgressCircular indeterminate />
        </div>

        <!-- The scan failed: say so instead of claiming everything is fine -->
        <div v-else-if="loadError" class="text-error pa-4 text-center">
          Could not search: {{ loadError }}
        </div>

        <!-- Nothing to fix -->
        <div v-else-if="!report || report.entries.length === 0" class="text-medium-emphasis pa-4 text-center">
          All library files are present.
        </div>

        <!-- Finished going through every entry: summary -->
        <div v-else-if="!current" class="pa-4 text-center">
          Relinked {{ relinked }}, removed {{ removed }}, skipped {{ skipped }}, {{ store.missingEntryIds.size }} still missing.
        </div>

        <!-- One missing entry and its candidate replacement files -->
        <div v-else>
          <div class="text-caption text-medium-emphasis mb-1">
            Entry {{ index + 1 }} of {{ report.entries.length }}
          </div>
          <div class="d-flex align-center ga-2 mb-1">
            <VIcon size="small">
              {{ current.entry.entryType === 'ambiance' ? 'mdi-weather-fog' : 'mdi-music-note' }}
            </VIcon>
            <strong>{{ current.entry.name }}</strong>
            <VChip v-for="tag in current.entry.tags" :key="tag" size="x-small">{{ tag }}</VChip>
          </div>
          <div
            class="text-caption text-warning text-truncate mb-3"
            style="direction: rtl; text-align: left;"
            :title="current.entry.filePath"
          >{{ current.entry.filePath }}</div>

          <VList
            v-if="current.candidates.length > 0"
            density="compact"
            class="candidate-list pa-0"
            max-height="260"
          >
            <VListItem
              v-for="c in current.candidates"
              :key="c.filePath"
              :title="c.relativePath"
              :subtitle="c.usedByEntryName ? `already linked to ${c.usedByEntryName}` : undefined"
              :disabled="!!c.usedByEntryName"
              :active="selectedPath === c.filePath"
              density="compact"
              @click="selectedPath = c.filePath"
            >
              <template #append>
                <VChip size="x-small" :color="c.kind === 'exact' ? 'success' : undefined">
                  {{ KIND_LABELS[c.kind] }}
                </VChip>
              </template>
            </VListItem>
          </VList>
          <div v-else class="text-medium-emphasis text-caption pa-3 text-center">
            No matching files under this folder.
          </div>
        </div>
      </VCardText>
      <VDivider />
      <VCardActions>
        <VBtn
          v-if="current && !loading"
          text="Browse..."
          variant="plain"
          :disabled="relinking"
          prepend-icon="mdi-file-search"
          @click="browse"
        />
        <VBtn
          v-if="current && !loading"
          text="Remove"
          variant="plain"
          color="error"
          :disabled="relinking"
          prepend-icon="mdi-delete"
          title="Delete this entry from the library"
          @click="remove"
        />
        <VSpacer />
        <VBtn v-if="current && !loading" text="Skip" variant="plain" :disabled="relinking" @click="skip" />
        <VBtn
          v-if="current && !loading"
          text="Relink"
          color="primary"
          variant="tonal"
          :disabled="!selectedPath || relinking"
          :loading="relinking"
          @click="selectedPath && relink(selectedPath)"
        />
        <VBtn text="Close" variant="plain" @click="close" />
      </VCardActions>
    </VCard>
  </VDialog>

  <ConfirmDialog
    v-model="showRemoveConfirm"
    title="Remove library entry?"
    confirm-label="Remove"
    confirm-color="error"
    @confirm="performRemove"
  >
    <div class="mb-2">
      <strong>{{ current?.entry.name }}</strong> is used in
      {{ removeUsage.length }} clip{{ removeUsage.length === 1 ? '' : 's' }}:
    </div>
    <VList density="compact" class="candidate-list pa-0 mb-2" max-height="220">
      <VListItem v-for="usage in removeUsage" :key="usage.clipId" density="compact">
        <VListItemTitle>
          {{ usage.sceneName }} <VIcon size="x-small">mdi-chevron-right</VIcon> {{ usage.trackName }}
        </VListItemTitle>
      </VListItem>
    </VList>
    <div class="text-caption text-warning">
      Removing will delete this entry and every clip that uses it.
    </div>
  </ConfirmDialog>
</template>

<style scoped>
.candidate-list {
  overflow-y: auto;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 4px;
}
</style>
