<script setup lang="ts">
import { ref, computed } from 'vue';
import { storeToRefs } from 'pinia';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../stores/appStore';
import { useLibraryImport } from '../composables/useLibraryImport';
import TagFilter from '../components/TagFilter.vue';
import LibraryEntryRow from '../components/LibraryEntryRow.vue';
import WaveformEditorDialog from '../components/WaveformEditorDialog.vue';

const store = useAppStore();
const { libraryEntries } = storeToRefs(store);
const { importFiles, importFolder } = useLibraryImport();

const searchQuery = ref('');
const selectedTags = ref<string[]>([]);

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

// Waveform editor dialog state
const showEditorDialog = ref(false);
const editorEntryId = ref('');

function openEditor(entryId: string): void {
  editorEntryId.value = entryId;
  showEditorDialog.value = true;
}

// Tag edit dialog state
const showTagDialog = ref(false);
const tagDialogEntryId = ref('');
const tagDialogValue = ref('');

function openTagDialog(entryId: string, currentTags: string[]): void {
  tagDialogEntryId.value = entryId;
  tagDialogValue.value = currentTags.join(', ');
  showTagDialog.value = true;
}

async function saveTagDialog(): Promise<void> {
  const tags = tagDialogValue.value
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  await store.command('update_library_entry_tags', {
    entryId: tagDialogEntryId.value,
    tags,
  });
  showTagDialog.value = false;
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
  await store.checkMissingFiles();
}
</script>

<template>
  <VContainer fluid class="d-flex flex-column" style="height: 100%;">
    <!-- Top bar -->
    <div class="d-flex align-center mb-4 gap-2">
      <span class="text-h6 font-weight-bold">Library</span>
      <VSpacer />
      <VBtn prepend-icon="mdi-file-music" variant="tonal" @click="importFiles">
        Import Files
      </VBtn>
      <VBtn prepend-icon="mdi-folder-music" variant="tonal" @click="importFolder">
        Import Folder
      </VBtn>
    </div>

    <!-- Search -->
    <VTextField
      v-model="searchQuery"
      label="Search"
      prepend-inner-icon="mdi-magnify"
      variant="outlined"
      density="compact"
      clearable
      class="mb-2"
      hide-details
    />

    <!-- Tag filter -->
    <div class="mb-3">
      <TagFilter v-model:selectedTags="selectedTags" />
    </div>

    <!-- Entry count -->
    <div class="text-caption text-medium-emphasis mb-2">
      {{ filteredEntries.length }} {{ filteredEntries.length === 1 ? 'entry' : 'entries' }}
    </div>

    <!-- Entry list -->
    <VList v-if="filteredEntries.length > 0" class="flex-grow-1 overflow-y-auto" lines="two">
      <LibraryEntryRow
        v-for="entry in filteredEntries"
        :key="entry.id"
        :entry="entry"
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
            icon="mdi-waveform"
            size="small"
            variant="text"
            title="Edit waveform"
            @click.stop="openEditor(entry.id)"
          />
          <VBtn
            icon="mdi-tag-edit"
            size="small"
            variant="text"
            title="Edit tags"
            @click.stop="openTagDialog(entry.id, entry.tags)"
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
      class="flex-grow-1 d-flex flex-column align-center justify-center text-medium-emphasis"
    >
      <VIcon size="64" class="mb-4">mdi-music-box-multiple-outline</VIcon>
      <span>Import some audio files</span>
    </div>

    <!-- Waveform editor dialog -->
    <WaveformEditorDialog
      v-model="showEditorDialog"
      :entry-id="editorEntryId"
    />

    <!-- Tag edit dialog -->
    <VDialog v-model="showTagDialog" max-width="400">
      <VCard title="Edit Tags">
        <VCardText>
          <VTextField
            v-model="tagDialogValue"
            label="Tags (comma-separated)"
            variant="outlined"
            autofocus
            hint="e.g. ambient, combat, tavern"
            @keydown.enter="saveTagDialog"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showTagDialog = false" />
          <VBtn text="Save" color="primary" variant="tonal" @click="saveTagDialog" />
        </VCardActions>
      </VCard>
    </VDialog>
  </VContainer>
</template>
