<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { useRouter } from 'vue-router';
import { storeToRefs } from 'pinia';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../stores/appStore';
import { useLibraryImport } from '../composables/useLibraryImport';
import { useAudio } from '../composables/useAudio';
import { useToast } from '../composables/useToast';
import TagFilter from '../components/TagFilter.vue';
import TagCellEditor from '../components/TagCellEditor.vue';
import ImportDialog from '../components/ImportDialog.vue';
import FixMissingDialog from '../components/FixMissingDialog.vue';
import AutoTaggingDialog from '../components/AutoTaggingDialog.vue';
import ConfirmDialog from '../components/ConfirmDialog.vue';
import type { LibraryEntry, LibraryEntryUsage, Role } from '@generated/index';
import MoodFilters from '../components/MoodFilters.vue';
import MoodProfile from '../components/MoodProfile.vue';
import {
  clickSort,
  emptyChipSelection,
  emptyMoodFilters,
  lensHeaderParts,
  lensLabel,
  lensOn as lensIsOn,
  lensScore,
  matchesMoodFilters,
  matchesTagSelection,
  moodLayers,
  moodValue,
  sortByKey,
  type ChipSelection,
  type MoodFilters as MoodFilterState,
  type SortDir,
  type SortKey,
} from '../utils/moods';
import { AUDIO_EXTENSIONS } from '../constants';
import { formatDuration } from '../shared/format';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';

const store = useAppStore();
const { libraryEntries, tags } = storeToRefs(store);
const { pickFiles, pickFolder, runImport } = useLibraryImport();
const audio = useAudio();
const toast = useToast();
const router = useRouter();

const searchQuery = ref('');
const tagSelection = ref<ChipSelection>(emptyChipSelection());
const untagged = ref(false);
const hasAuto = ref(false);
const selectedType = ref<string>('all');
const moodFilters = ref<MoodFilterState>(emptyMoodFilters());
const sortKey = ref<SortKey>('name');
const sortDir = ref<SortDir>('asc');
// Lenses and role apply to music only and their bar is hidden for ambiance,
// so its filters and sort must not keep applying there.
watch(selectedType, (type) => {
  if (type === 'ambiance') {
    moodFilters.value = emptyMoodFilters();
    sortKey.value = 'name';
    sortDir.value = 'asc';
  }
});
const lensOn = computed(() => lensIsOn(moodFilters.value));

// Search text also matches the album folder (parent) and artist folder
// (grandparent) of the file path, so a game is findable by soundtrack folder.
function searchFields(entry: LibraryEntry): string[] {
  const parts = entry.filePath.split(/[\\/]/).filter((p) => p.length > 0 && !/^[A-Za-z]:$/.test(p));
  // Empty segments and a leading drive letter are dropped, so "c:" matches nothing.
  // The last part is the file itself.
  return [entry.name, parts[parts.length - 2] ?? '', parts[parts.length - 3] ?? ''];
}

function roleLabel(role: Role | null): string {
  if (role === 'combat') return 'Combat';
  if (role === 'nonCombat') return 'Non-combat';
  return 'No role';
}

watch(tags, (current) => {
  const { include, exclude } = tagSelection.value;
  const known = (t: string): boolean => current.includes(t);
  if (!include.every(known) || !exclude.every(known)) {
    tagSelection.value = { include: include.filter(known), exclude: exclude.filter(known) };
  }
});

const filteredEntries = computed(() => {
  const matching = libraryEntries.value.filter((entry) => {
    const query = (searchQuery.value ?? '').trim().toLowerCase();
    const matchesSearch =
      query === '' || searchFields(entry).some((f) => f.toLowerCase().includes(query));
    // Auto tags are part of `tags`, so "untagged" means no user tags.
    const matchesTags = untagged.value
      ? entry.tags.length === entry.autoTags.length
      : matchesTagSelection(entry.tags, tagSelection.value);
    const matchesType =
      selectedType.value === 'all' || entry.entryType === selectedType.value;
    const matchesAuto = !hasAuto.value || entry.autoTags.length > 0;
    return matchesSearch && matchesTags && matchesType && matchesAuto && matchesMoodFilters(entry, moodFilters.value);
  });
  return sortByKey(matching, sortKey.value, sortDir.value, moodFilters.value, (e) => e, (e) => store.isEntryMissing(e.id));
});

// Column headers are the only sort control: status (missing files first),
// Type, Name, the lens score column while a lens is included, Role, Energy
// and Duration. `filteredEntries` is already in that order (`sortByKey`,
// entries with no value last), so the table's own sort is a no-op that only
// draws the header arrow.
const keepOrder = (): number => 0;
const headers = computed(() => [
  { title: '', key: 'status', sortable: true, sort: keepOrder, width: '48px' },
  { title: 'Type', key: 'entryType', sortable: true, sort: keepOrder, width: '60px' },
  { title: 'Name', key: 'name', sortable: true, sort: keepOrder, width: '32%' },
  ...(lensOn.value
    ? [{ title: lensLabel(moodFilters.value), key: 'lens', sortable: true, sort: keepOrder, width: '160px' }]
    : []),
  { title: 'Role', key: 'role', sortable: true, sort: keepOrder, width: '60px' },
  { title: 'Energy', key: 'energy', sortable: true, sort: keepOrder, width: '80px', align: 'end' as const },
  { title: 'Tags', key: 'tags', sortable: false },
  { title: 'Duration', key: 'duration', sortable: true, sort: keepOrder, width: '80px', align: 'end' as const },
  { title: '', key: 'actions', sortable: false, width: '112px', align: 'end' as const },
]);

const tableSortBy = computed(() => [{ key: sortKey.value, order: sortDir.value }]);

/// A header click: the table reports its new sort; the clicked key is the
/// first one that differs from ours (or ours again, flipped).
function onSortBy(next: { key: string; order?: unknown }[]): void {
  const key = (next.find((s) => s.key !== sortKey.value)?.key ?? sortKey.value) as SortKey;
  const picked = clickSort({ key: sortKey.value, dir: sortDir.value }, key);
  sortKey.value = picked.key;
  sortDir.value = picked.dir;
}

/// Hover text for an Energy cell: where the value comes from.
function energyHint(entry: LibraryEntry): string {
  const { auto, user } = moodLayers(entry, 'energy');
  if (user !== null) return `Set by you${auto === null ? '' : ` · suggested ${auto.toFixed(2)}`}`;
  return 'Suggested';
}

const previewState = computed(() => store.playbackStates.get(LIBRARY_PLAYBACK_ID));

function isPreviewPlaying(entryId: string): boolean {
  return store.libraryPlayingEntryId === entryId && !!previewState.value?.isPlaying;
}

function togglePreview(entry: LibraryEntry): void {
  if (isPreviewPlaying(entry.id)) {
    audio.stopLibrarySound();
  } else {
    audio.playLibrarySound(
      entry.id,
      entry.filePath,
      entry.trimStart ?? 0,
      entry.volume,
      entry.trimStart, entry.trimEnd,
      entry.loopStart, entry.loopEnd,
    );
  }
}

function openEditor(entryId: string): void {
  store.editEntryId = entryId;
}

function moreLikeThis(entryId: string): void {
  router.push({ name: 'scry', query: { seed: entryId } });
}

// --- Multi-select + bulk tagging ---

const selectedIds = ref<string[]>([]);
const bulkTags = ref<string[]>([]);
const bulkBusy = ref(false);

// Drop selections for entries that no longer exist (deleted, etc.).
watch(libraryEntries, (entries) => {
  const ids = new Set(entries.map((e) => e.id));
  if (selectedIds.value.some((id) => !ids.has(id))) {
    selectedIds.value = selectedIds.value.filter((id) => ids.has(id));
  }
});

function clearSelection(): void {
  selectedIds.value = [];
  bulkTags.value = [];
}

function cleanBulkTags(): string[] {
  return bulkTags.value.map((t) => t.trim()).filter((t) => t.length > 0);
}

async function applyBulkTags(mode: 'add' | 'remove'): Promise<void> {
  const tags = cleanBulkTags();
  if (tags.length === 0 || selectedIds.value.length === 0) return;
  bulkBusy.value = true;
  try {
    await store.command('bulk_edit_library_entry_tags', {
      entryIds: selectedIds.value,
      add: mode === 'add' ? tags : [],
      remove: mode === 'remove' ? tags : [],
    });
    bulkTags.value = [];
  } finally {
    bulkBusy.value = false;
  }
}

async function applyBulkMusic(cmd: string, args: Record<string, unknown>): Promise<void> {
  // Role applies to music only.
  const ids = libraryEntries.value
    .filter((e) => selectedIds.value.includes(e.id) && e.entryType === 'music')
    .map((e) => e.id);
  if (ids.length === 0) return;
  bulkBusy.value = true;
  try {
    await store.command(cmd, { entryIds: ids, ...args });
  } finally {
    bulkBusy.value = false;
  }
}

const applyBulkRole = (role: Role | null) => applyBulkMusic('set_entry_role', { role });
// "Set mood levels…" opens the mood profile for the selected music entries;
// ambiance has no moods.
const selectedMusic = computed(() =>
  libraryEntries.value.filter((e) => selectedIds.value.includes(e.id) && e.entryType === 'music'),
);
const showMoodDialog = ref(false);
watch(selectedMusic, (list) => {
  if (list.length === 0) showMoodDialog.value = false;
});

const selectedHaveAuto = computed(() =>
  libraryEntries.value.some((e) => selectedIds.value.includes(e.id) && e.autoTags.length > 0),
);

async function applyAutoDecision(cmd: 'keep_auto_tags' | 'reject_auto_tags'): Promise<void> {
  if (selectedIds.value.length === 0) return;
  bulkBusy.value = true;
  try {
    await store.command(cmd, { entryIds: selectedIds.value, tags: null });
  } finally {
    bulkBusy.value = false;
  }
}

// --- Import flow ---

const showImportDialog = ref(false);
const pendingImportPaths = ref<string[]>([]);

async function startImportFiles(): Promise<void> {
  const paths = await pickFiles();
  if (paths.length === 0) return;
  pendingImportPaths.value = paths;
  showImportDialog.value = true;
}

async function startImportFolder(): Promise<void> {
  const paths = await pickFolder();
  if (paths.length === 0) return;
  pendingImportPaths.value = paths;
  showImportDialog.value = true;
}

async function handleImportConfirm(payload: { paths: string[]; tags: string[]; entryType: string }): Promise<void> {
  pendingImportPaths.value = [];
  await runImport(payload.paths, payload.tags, payload.entryType);
}

// --- Fix missing files flow ---

const showFixMissing = ref(false);
const showAutoTagging = ref(false);

// --- Delete flow ---

const showDeleteConfirm = ref(false);
const deleteTargetId = ref<string | null>(null);
const deleteTargetName = ref('');
const deleteUsage = ref<LibraryEntryUsage[]>([]);

async function deleteEntry(entryId: string): Promise<void> {
  const entry = libraryEntries.value.find((e) => e.id === entryId);
  let usage: LibraryEntryUsage[];
  try {
    usage = await invoke<LibraryEntryUsage[]>('get_library_entry_usage', { entryId });
  } catch (e) {
    // Do NOT fall through to the unconfirmed-delete branch on a failed
    // usage check — we can't know the entry is safe to delete.
    toast.showError(`Could not check whether "${entry?.name ?? entryId}" is in use: ${e}`);
    return;
  }

  if (usage.length === 0) {
    await store.command('delete_library_entry', { entryId });
    return;
  }

  deleteTargetId.value = entryId;
  deleteTargetName.value = entry?.name ?? '';
  deleteUsage.value = usage;
  showDeleteConfirm.value = true;
}

async function confirmDelete(): Promise<void> {
  const entryId = deleteTargetId.value;
  deleteTargetId.value = null;
  deleteUsage.value = [];
  deleteTargetName.value = '';
  if (!entryId) return;
  await store.command('delete_library_entry', { entryId });
}

// --- Relocate flow ---

async function relocateEntry(entryId: string): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
  });
  if (!path || Array.isArray(path)) return;
  await store.command('relocate_library_entry', { entryId, newFilePath: path });
}
</script>

<template>
  <div class="d-flex flex-column pa-3" style="height: 100%; position: relative;">
    <Teleport to="#app-bar-main">
      <VTextField
        v-model="searchQuery"
        placeholder="Search"
        prepend-inner-icon="mdi-magnify"
        variant="solo-filled"
        flat
        density="compact"
        clearable
        hide-details
        single-line
        class="flex-grow-1"
        style="max-width: 500px;"
      />
      <VBtnToggle v-model="selectedType" mandatory density="compact" variant="outlined" divided class="ml-3">
        <VBtn value="all" size="small">All</VBtn>
        <VBtn value="music" size="small" prepend-icon="mdi-music-note">Music</VBtn>
        <VBtn value="ambiance" size="small" prepend-icon="mdi-weather-fog">Ambiance</VBtn>
      </VBtnToggle>
      <VSpacer />
      <VBadge
        color="warning"
        :content="store.missingEntryIds.size"
        :model-value="store.missingEntryIds.size > 0"
      >
        <VBtn
          prepend-icon="mdi-wrench"
          variant="tonal"
          size="small"
          rounded="xl"
          class="mr-2"
          @click="showFixMissing = true"
        >
          Fix missing files
        </VBtn>
      </VBadge>
      <VBtn
        prepend-icon="mdi-creation"
        variant="tonal"
        size="small"
        rounded="xl"
        class="mr-2"
        @click="showAutoTagging = true"
      >
        Auto-tagging
      </VBtn>
      <VMenu :disabled="!!store.importProgress">
        <template #activator="{ props: menuProps }">
          <VBtn
            v-bind="menuProps"
            prepend-icon="mdi-import"
            append-icon="mdi-menu-down"
            variant="tonal"
            size="small"
            rounded="xl"
            :disabled="!!store.importProgress"
            :title="store.importProgress ? 'Import in progress...' : undefined"
          >
            Import
          </VBtn>
        </template>
        <VList density="compact">
          <VListItem prepend-icon="mdi-file-music" @click="startImportFiles">
            <VListItemTitle>Files...</VListItemTitle>
          </VListItem>
          <VListItem prepend-icon="mdi-folder-music" @click="startImportFolder">
            <VListItemTitle>Folder...</VListItemTitle>
          </VListItem>
        </VList>
      </VMenu>
    </Teleport>

    <!-- Lenses, role and sort apply to music only, so the bar is hidden while browsing ambiance. -->
    <MoodFilters
      v-if="selectedType !== 'ambiance'"
      v-model="moodFilters"
      v-model:sort="sortKey"
      v-model:dir="sortDir"
      size="small"
      default-sort="name"
      class="mb-2"
    />

    <div class="d-flex align-end mb-2 ga-2">
      <div class="d-flex flex-wrap ga-1 flex-grow-1" style="min-width: 0;">
        <TagFilter v-model:selection="tagSelection" v-model:untagged="untagged" v-model:has-auto="hasAuto" />
      </div>
      <span class="text-caption text-medium-emphasis" style="white-space: nowrap;">
        {{ filteredEntries.length }} {{ filteredEntries.length === 1 ? 'entry' : 'entries' }}
      </span>
    </div>


    <VDataTableVirtual
      v-if="filteredEntries.length > 0"
      v-model="selectedIds"
      show-select
      :headers="headers"
      :items="filteredEntries"
      :sort-by="tableSortBy"
      must-sort
      item-value="id"
      :item-height="48"
      fixed-header
      hover
      density="comfortable"
      @update:sort-by="onSortBy"
      class="library-table flex-grow-1"
      :row-props="({ item }: { item: LibraryEntry }) => ({
        class: [
          'cursor-pointer',
          store.isEntryMissing(item.id) ? 'opacity-50' : '',
          isPreviewPlaying(item.id) ? 'preview-active' : '',
        ],
        onClick: () => openEditor(item.id),
      })"
    >
      <!-- Stop the checkbox click from also opening the editor -->
      <template #item.data-table-select="{ internalItem, isSelected, toggleSelect }">
        <VCheckboxBtn
          :model-value="isSelected(internalItem)"
          density="compact"
          @click.stop
          @update:model-value="toggleSelect(internalItem)"
        />
      </template>

      <template #item.status="{ item }">
        <VBtn
          v-if="store.isEntryMissing(item.id)"
          icon="mdi-alert"
          size="small"
          variant="text"
          color="warning"
          title="File missing — click to relocate"
          @click.stop="relocateEntry(item.id)"
        />
        <VBtn
          v-else
          :icon="isPreviewPlaying(item.id) ? 'mdi-stop' : 'mdi-play'"
          variant="text"
          size="small"
          @click.stop="togglePreview(item)"
        />
      </template>

      <template #item.entryType="{ item }">
        <VIcon size="small" :title="item.entryType === 'ambiance' ? 'Ambiance' : 'Music'">
          {{ item.entryType === 'ambiance' ? 'mdi-weather-fog' : 'mdi-music-note' }}
        </VIcon>
      </template>

      <template #item.role="{ item }">
        <span v-if="item.role" class="role-cell" :title="roleLabel(item.role)">
          <VIcon size="small" :color="item.roleUser === null ? undefined : 'primary'">
            {{ item.role === 'combat' ? 'mdi-sword-cross' : 'mdi-music-note' }}
          </VIcon>
          <VIcon v-if="item.roleUser === null" size="x-small" class="role-auto">mdi-creation</VIcon>
          <VTooltip v-if="item.roleUser === null" activator="parent" location="top" open-delay="300">
            {{ roleLabel(item.role) }} · Suggested{{ item.roleAuto === null ? ' (default)' : item.roleAutoScore === null ? '' : ` · ${item.roleAutoScore.toFixed(2)}` }}
          </VTooltip>
        </span>
      </template>

      <template #header.lens="{ column, getSortIcon }">
        <div class="v-data-table-header__content lens-header" :title="lensLabel(moodFilters)">
          <span>{{ lensHeaderParts(moodFilters).ranked }}</span>
          <span v-if="lensHeaderParts(moodFilters).excluded.length > 0">
            · <s class="text-error">{{ lensHeaderParts(moodFilters).excluded.join(', ') }}</s>
          </span>
          <VIcon class="v-data-table-header__sort-icon" :icon="getSortIcon(column)" />
        </div>
      </template>

      <template #item.lens="{ item }">
        <div v-if="lensScore(item, moodFilters) !== null" class="lens-cell" :title="lensLabel(moodFilters)">
          <div class="lens-bar"><div class="lens-fill" :style="{ width: `${(lensScore(item, moodFilters) ?? 0) * 100}%` }" /></div>
          <span class="lens-value">{{ (lensScore(item, moodFilters) ?? 0).toFixed(2) }}</span>
        </div>
        <span v-else class="text-medium-emphasis" title="Not analyzed yet">—</span>
      </template>

      <template #item.energy="{ item }">
        <span
          v-if="moodValue(item, 'energy') !== null"
          class="energy-cell"
          :class="moodLayers(item, 'energy').user === null ? 'text-medium-emphasis' : 'text-primary'"
          :title="energyHint(item)"
        >{{ moodValue(item, 'energy')?.toFixed(2) }}</span>
      </template>

      <template #item.tags="{ item }">
        <TagCellEditor :entry-id="item.id" :tags="item.tags" :auto-tags="item.autoTags" />
      </template>

      <template #item.duration="{ item }">
        {{ formatDuration(item.duration) }}
      </template>

      <template #item.actions="{ item }">
        <div class="d-flex flex-nowrap justify-end align-center">
          <VBtn
            v-if="item.entryType === 'music'"
            icon="mdi-compass-outline"
            size="small"
            density="comfortable"
            variant="text"
            title="More like this"
            @click.stop="moreLikeThis(item.id)"
          />
          <VBtn
            icon="mdi-delete"
            size="small"
            density="comfortable"
            variant="text"
            color="error"
            title="Delete"
            @click.stop="deleteEntry(item.id)"
          />
        </div>
      </template>
    </VDataTableVirtual>

    <div
      v-else
      class="d-flex flex-column align-center justify-center text-medium-emphasis pa-8"
    >
      <VIcon size="64" class="mb-4">mdi-music-box-multiple-outline</VIcon>
      <span>Import some audio files</span>
    </div>

    <!-- Bulk tag bar: floats over the bottom of the table so the rows
         never shift when it appears -->
    <VFadeTransition>
      <div v-if="selectedIds.length > 0" class="bulk-bar elevation-8 d-flex flex-wrap align-center ga-2 px-3 py-2">
        <VIcon size="small" color="primary">mdi-checkbox-multiple-marked</VIcon>
        <span class="text-body-2 font-weight-medium" style="white-space: nowrap;">
          {{ selectedIds.length }} selected
        </span>
        <VCombobox
          v-model="bulkTags"
          :items="tags"
          placeholder="Tags to add or remove"
          variant="outlined"
          density="compact"
          hide-details
          hide-no-data
          multiple
          chips
          closable-chips
          class="flex-grow-1"
          style="min-width: 240px;"
        />
        <VBtn
          prepend-icon="mdi-tag-plus"
          color="primary"
          variant="tonal"
          size="small"
          :disabled="bulkBusy || cleanBulkTags().length === 0"
          :loading="bulkBusy"
          @click="applyBulkTags('add')"
        >
          Add
        </VBtn>
        <VBtn
          prepend-icon="mdi-tag-minus"
          variant="tonal"
          size="small"
          :disabled="bulkBusy || cleanBulkTags().length === 0"
          @click="applyBulkTags('remove')"
        >
          Remove
        </VBtn>
        <VMenu>
          <template #activator="{ props: menuProps }">
            <VBtn
              v-bind="menuProps"
              prepend-icon="mdi-sword-cross"
              append-icon="mdi-menu-down"
              variant="tonal"
              size="small"
              :disabled="bulkBusy"
            >
              Set role
            </VBtn>
          </template>
          <VList density="compact">
            <VListItem prepend-icon="mdi-sword-cross" title="Combat" @click="applyBulkRole('combat')" />
            <VListItem prepend-icon="mdi-music-note" title="Non-combat" @click="applyBulkRole('nonCombat')" />
            <VListItem prepend-icon="mdi-restore" title="Back to suggestion" subtitle="Clear the role you set" @click="applyBulkRole(null)" />
          </VList>
        </VMenu>
        <VBtn
          prepend-icon="mdi-tune-vertical"
          variant="tonal"
          size="small"
          :disabled="bulkBusy || selectedMusic.length === 0"
          @click="showMoodDialog = true"
        >
          Set mood levels…
        </VBtn>
        <VBtn
          prepend-icon="mdi-check"
          variant="tonal"
          size="small"
          :disabled="bulkBusy || !selectedHaveAuto"
          @click="applyAutoDecision('keep_auto_tags')"
        >
          Keep auto tags
        </VBtn>
        <VBtn
          prepend-icon="mdi-creation"
          variant="tonal"
          size="small"
          :disabled="bulkBusy || !selectedHaveAuto"
          @click="applyAutoDecision('reject_auto_tags')"
        >
          Remove auto tags
        </VBtn>
        <VBtn
          icon="mdi-close"
          variant="text"
          size="small"
          title="Clear selection"
          @click="clearSelection"
        />
      </div>
    </VFadeTransition>

    <ImportDialog
      v-model="showImportDialog"
      :paths="pendingImportPaths"
      @import="handleImportConfirm"
    />

    <VDialog v-model="showMoodDialog" max-width="560">
      <VCard>
        <VCardTitle class="pt-4 px-5">
          Mood levels · {{ selectedMusic.length }} music {{ selectedMusic.length === 1 ? 'entry' : 'entries' }}
        </VCardTitle>
        <VCardText class="px-5">
          <div class="text-caption text-medium-emphasis mb-3">
            Click a bar to set that level on every selected entry. "mixed" means the entries differ.
          </div>
          <MoodProfile :key="selectedMusic.map((e) => e.id).join(',')" :entries="selectedMusic" />
        </VCardText>
        <VCardActions class="px-5 pb-4">
          <VSpacer />
          <VBtn variant="text" @click="showMoodDialog = false">Done</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>

    <FixMissingDialog v-model="showFixMissing" />
    <AutoTaggingDialog v-model="showAutoTagging" />

    <ConfirmDialog
      v-model="showDeleteConfirm"
      title="Delete library entry?"
      confirm-label="Delete"
      confirm-color="error"
      @confirm="confirmDelete"
    >
      <div class="mb-2">
        <strong>{{ deleteTargetName }}</strong> is used in
        {{ deleteUsage.length }} clip{{ deleteUsage.length === 1 ? '' : 's' }}:
      </div>
      <VList density="compact" class="usage-list pa-0 mb-2" max-height="220">
        <VListItem
          v-for="usage in deleteUsage"
          :key="usage.clipId"
          density="compact"
        >
          <VListItemTitle>
            {{ usage.compName }} <VIcon size="x-small">mdi-chevron-right</VIcon> {{ usage.trackName }}
          </VListItemTitle>
        </VListItem>
      </VList>
      <div class="text-caption text-warning">
        Deleting will remove this entry and every clip that uses it.
      </div>
    </ConfirmDialog>
  </div>
</template>

<style scoped>
.bulk-bar {
  position: absolute;
  left: 50%;
  bottom: 16px;
  transform: translateX(-50%);
  width: min(960px, calc(100% - 32px));
  z-index: 2;
  background: rgb(var(--v-theme-surface));
  border: 1px solid rgba(var(--v-theme-primary), 0.5);
  border-radius: 10px;
}

:deep(.preview-active) {
  background: rgba(240, 168, 48, 0.08) !important;
  border-left: 3px solid #F0A830;
}

.role-cell {
  position: relative;
  display: inline-flex;
}

.role-auto {
  position: absolute;
  top: -4px;
  right: -8px;
  opacity: 0.8;
}

.lens-cell {
  display: flex;
  align-items: center;
  gap: 6px;
}

.lens-bar {
  flex: 1 1 auto;
  min-width: 40px;
  height: 6px;
  border-radius: 3px;
  background: rgba(var(--v-theme-on-surface), 0.1);
  overflow: hidden;
}

.lens-fill {
  height: 100%;
  background: rgb(var(--v-theme-primary));
}

.lens-value,
.energy-cell {
  font-size: 0.8125rem;
  font-variant-numeric: tabular-nums;
}

.library-table {
  height: 0;
  min-height: 0;
}

.usage-list {
  overflow-y: auto;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 4px;
}
</style>
