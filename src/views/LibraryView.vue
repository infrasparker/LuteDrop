<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { storeToRefs } from 'pinia';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../stores/appStore';
import { useLibraryImport } from '../composables/useLibraryImport';
import { useAudio } from '../composables/useAudio';
import TagFilter from '../components/TagFilter.vue';
import type { LibraryEntry } from '@generated/index';

const store = useAppStore();
const { libraryEntries, tags } = storeToRefs(store);
const { importFiles, importFolder } = useLibraryImport();
const audio = useAudio();

const searchQuery = ref('');
const selectedTags = ref<string[]>([]);

// Remove stale tags from the filter when they no longer exist in the store
watch(tags, (current) => {
  const stale = selectedTags.value.filter((t) => !current.includes(t));
  if (stale.length > 0) {
    selectedTags.value = selectedTags.value.filter((t) => current.includes(t));
  }
});

const filteredEntries = computed(() => {
  return libraryEntries.value.filter((entry) => {
    const matchesSearch =
      searchQuery.value === '' ||
      entry.name.toLowerCase().includes(searchQuery.value.toLowerCase());
    const matchesTags =
      selectedTags.value.length === 0 ||
      selectedTags.value.every((tag) => entry.tags.includes(tag));
    return matchesSearch && matchesTags;
  });
});

// Table columns
const headers = [
  { title: '', key: 'status', sortable: true, width: '48px',
    sort: (a: boolean, b: boolean) => (a === b ? 0 : a ? -1 : 1) },
  { title: 'Name', key: 'name', sortable: true },
  { title: 'Tags', key: 'tags', sortable: false, width: '250px' },
  { title: 'Duration', key: 'duration', sortable: true, width: '80px', align: 'end' as const },
  { title: '', key: 'actions', sortable: false, width: '80px', align: 'end' as const },
];

// Preview playback
const previewState = computed(() => store.playbackStates.get('__library'));

function isPreviewPlaying(entryId: string): boolean {
  return store.libraryPlayingEntryId === entryId && !!previewState.value?.isPlaying;
}

watch(previewState, (state) => {
  if (!state?.isPlaying) {
    store.libraryPlayingEntryId = null;
  }
});

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

function formatDuration(seconds: number | null): string {
  if (seconds === null) return '--:--';
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

function openEditor(entryId: string): void {
  store.editEntryId = entryId;
}

async function removeTag(entryId: string, tag: string): Promise<void> {
  const entry = libraryEntries.value.find((e) => e.id === entryId);
  if (!entry) return;
  const newTags = entry.tags.filter((t) => t !== tag);
  await store.command('update_library_entry_tags', { entryId, tags: newTags });
}

// Delete entry
async function deleteEntry(entryId: string): Promise<void> {
  await store.command('delete_library_entry', { entryId });
}

// Relocate entry
const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];

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
  <div class="d-flex flex-column pa-3" style="height: 100%;">
    <!-- Teleport search + import into app bar main row -->
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
      <VSpacer />
      <VBtn prepend-icon="mdi-file-music" variant="tonal" size="small" rounded="xl" @click="importFiles">
        Import Files
      </VBtn>
      <VBtn prepend-icon="mdi-folder-music" variant="tonal" size="small" rounded="xl" class="ml-2" @click="importFolder">
        Import Folder
      </VBtn>
    </Teleport>

    <!-- Tag filter + count -->
    <div class="d-flex align-end mb-2 ga-2">
      <div class="d-flex flex-wrap ga-1 flex-grow-1" style="min-width: 0;">
        <TagFilter v-model:selectedTags="selectedTags" />
      </div>
      <span class="text-caption text-medium-emphasis" style="white-space: nowrap;">
        {{ filteredEntries.length }} {{ filteredEntries.length === 1 ? 'entry' : 'entries' }}
      </span>
    </div>

    <!-- Entry data table -->
    <VDataTableVirtual
      v-if="filteredEntries.length > 0"
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

      <template #item.tags="{ item }">
        <VChip
          v-for="tag in item.tags"
          :key="tag"
          size="small"
          class="mr-1"
          closable
          @click:close.stop="removeTag(item.id, tag)"
          @click.stop
        >
          {{ tag }}
        </VChip>
      </template>

      <template #item.duration="{ item }">
        {{ formatDuration(item.duration) }}
      </template>

      <template #item.actions="{ item }">
        <VBtn
          icon="mdi-delete"
          size="small"
          variant="text"
          color="error"
          title="Delete"
          @click.stop="deleteEntry(item.id)"
        />
      </template>
    </VDataTableVirtual>

    <!-- Empty state -->
    <div
      v-else
      class="d-flex flex-column align-center justify-center text-medium-emphasis pa-8"
    >
      <VIcon size="64" class="mb-4">mdi-music-box-multiple-outline</VIcon>
      <span>Import some audio files</span>
    </div>

  </div>
</template>

<style scoped>
:deep(.preview-active) {
  background: rgba(240, 168, 48, 0.08) !important;
  border-left: 3px solid #F0A830;
}

.library-table {
  height: 0;
  min-height: 0;
}
</style>
