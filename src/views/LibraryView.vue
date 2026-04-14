<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { storeToRefs } from 'pinia';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../stores/appStore';
import { useLibraryImport } from '../composables/useLibraryImport';
import TagFilter from '../components/TagFilter.vue';
import LibraryEntryRow from '../components/LibraryEntryRow.vue';

const store = useAppStore();
const { libraryEntries, tags } = storeToRefs(store);
const { importFiles, importFolder } = useLibraryImport();

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

function openEditor(entryId: string): void {
  store.editEntryId = entryId;
}

// Delete entry
async function deleteEntry(entryId: string): Promise<void> {
  await store.command('delete_library_entry', { entryId });
}

// Relocate entry (Task 14)
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

    <!-- Entry list -->
    <VList v-if="filteredEntries.length > 0" class="overflow-y-auto" lines="two">
      <LibraryEntryRow
        v-for="entry in filteredEntries"
        :key="entry.id"
        :entry="entry"
        @click="openEditor(entry.id)"
      >
        <template #actions>
          <VBtn
            v-if="store.isEntryMissing(entry.id)"
            icon="mdi-folder-search"
            size="small"
            variant="text"
            color="warning"
            title="Relocate file"
            @click.stop="relocateEntry(entry.id)"
          />
          <VBtn
            icon="mdi-delete"
            size="small"
            variant="text"
            color="error"
            title="Delete"
            @click.stop="deleteEntry(entry.id)"
          />
        </template>
      </LibraryEntryRow>
    </VList>

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
