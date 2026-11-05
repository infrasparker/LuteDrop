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
import type { LibraryEntry, LibraryEntryUsage, Pace, Role } from '@generated/index';
import SoundFilters from '../components/SoundFilters.vue';
import { emptySoundFilters, matchesSoundFilters, type SoundFilters as SoundFilterState } from '../utils/mood';
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
const selectedTags = ref<string[]>([]);
const untagged = ref(false);
const hasAuto = ref(false);
const selectedType = ref<string>('all');
const roleFilter = ref<'all' | Role | 'unset'>('all');
const soundFilters = ref<SoundFilterState>(emptySoundFilters());
// Role, mood and pace apply to music only and are hidden for ambiance, so their filters must not keep applying there.
watch(selectedType, (type) => {
  if (type === 'ambiance') {
    roleFilter.value = 'all';
    soundFilters.value = emptySoundFilters();
  }
});

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
  const stale = selectedTags.value.filter((t) => !current.includes(t));
  if (stale.length > 0) {
    selectedTags.value = selectedTags.value.filter((t) => current.includes(t));
  }
});

const filteredEntries = computed(() => {
  return libraryEntries.value.filter((entry) => {
    const query = (searchQuery.value ?? '').trim().toLowerCase();
    const matchesSearch =
      query === '' || searchFields(entry).some((f) => f.toLowerCase().includes(query));
    // Auto tags are part of `tags`, so "untagged" means no user tags.
    const matchesTags = untagged.value
      ? entry.tags.length === entry.autoTags.length
      : selectedTags.value.length === 0 ||
        selectedTags.value.every((tag) => entry.tags.includes(tag));
    const matchesType =
      selectedType.value === 'all' || entry.entryType === selectedType.value;
    const matchesAuto = !hasAuto.value || entry.autoTags.length > 0;
    const matchesRole =
      roleFilter.value === 'all' ||
      (roleFilter.value === 'unset'
        ? entry.entryType === 'music' && entry.role === null
        : entry.role === roleFilter.value);
    const matchesSound = matchesSoundFilters(entry, soundFilters.value);
    return matchesSearch && matchesTags && matchesType && matchesAuto && matchesRole && matchesSound;
  });
});

const headers = [
  { title: '', key: 'status', sortable: true, width: '48px',
    sort: (a: boolean, b: boolean) => (a === b ? 0 : a ? -1 : 1) },
  { title: 'Type', key: 'entryType', sortable: true, width: '60px' },
  { title: 'Name', key: 'name', sortable: true, width: '40%' },
  { title: 'Role', key: 'role', sortable: true, width: '60px' },
  { title: 'Tags', key: 'tags', sortable: false },
  { title: 'Duration', key: 'duration', sortable: true, width: '80px', align: 'end' as const },
  { title: '', key: 'actions', sortable: false, width: '112px', align: 'end' as const },
];

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
  // Role and pace apply to music only.
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
const applyBulkPace = (pace: Pace | null) => applyBulkMusic('set_entry_pace', { pace });

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
      <!-- Role applies to music only, so it is hidden while browsing ambiance. -->
      <VBtnToggle
        v-if="selectedType !== 'ambiance'"
        v-model="roleFilter"
        mandatory
        density="compact"
        variant="outlined"
        divided
        class="ml-3"
      >
        <VBtn value="all" size="small">Any role</VBtn>
        <VBtn value="combat" size="small" prepend-icon="mdi-sword-cross">Combat</VBtn>
        <VBtn value="nonCombat" size="small" prepend-icon="mdi-music-note">Non-combat</VBtn>
        <VBtn value="unset" size="small">Unset</VBtn>
      </VBtnToggle>
      <SoundFilters v-if="selectedType !== 'ambiance'" v-model="soundFilters" size="small" class="ml-3" />
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

    <div class="d-flex align-end mb-2 ga-2">
      <div class="d-flex flex-wrap ga-1 flex-grow-1" style="min-width: 0;">
        <TagFilter v-model:selected-tags="selectedTags" v-model:untagged="untagged" v-model:has-auto="hasAuto" />
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
      item-value="id"
      :item-height="48"
      fixed-header
      hover
      density="comfortable"
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
            {{ roleLabel(item.role) }} · Auto{{ item.roleAutoScore === null ? '' : ` · ${item.roleAutoScore.toFixed(2)}` }}
          </VTooltip>
        </span>
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
            <VListItem prepend-icon="mdi-restore" title="Use auto" subtitle="Remove your choice" @click="applyBulkRole(null)" />
          </VList>
        </VMenu>
        <VMenu>
          <template #activator="{ props: menuProps }">
            <VBtn
              v-bind="menuProps"
              prepend-icon="mdi-play-pause"
              append-icon="mdi-menu-down"
              variant="tonal"
              size="small"
              :disabled="bulkBusy"
            >
              Set pace
            </VBtn>
          </template>
          <VList density="compact">
            <VListItem prepend-icon="mdi-pause" title="Still" @click="applyBulkPace('still')" />
            <VListItem prepend-icon="mdi-play" title="Moving" @click="applyBulkPace('moving')" />
            <VListItem prepend-icon="mdi-restore" title="Use auto" subtitle="Remove your choice" @click="applyBulkPace(null)" />
          </VList>
        </VMenu>
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
