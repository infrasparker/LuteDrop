<script setup lang="ts">
import { ref, computed } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import TagFilter from './TagFilter.vue';
import LibraryPickerRow from './LibraryPickerRow.vue';

const props = defineProps<{
  modelValue: boolean;
  trackId: string;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const store = useAppStore();
const audio = useAudio();
const { libraryEntries } = storeToRefs(store);

const searchQuery = ref('');
const selectedTags = ref<string[]>([]);
const selectedIds = ref<Set<string>>(new Set());

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

function toggleEntry(entryId: string): void {
  const next = new Set(selectedIds.value);
  if (next.has(entryId)) {
    next.delete(entryId);
  } else {
    next.add(entryId);
  }
  selectedIds.value = next;
}

function close(): void {
  audio.stopLibrarySound();
  selectedIds.value = new Set();
  searchQuery.value = '';
  selectedTags.value = [];
  emit('update:modelValue', false);
}

async function addClips(): Promise<void> {
  for (const libraryEntryId of selectedIds.value) {
    await store.command('add_clip', { trackId: props.trackId, libraryEntryId });
  }
  close();
}
</script>

<template>
  <VDialog :model-value="modelValue" @update:model-value="close" max-width="600">
    <VCard>
      <VCardTitle class="d-flex align-center">
        <span>Add Clips from Library</span>
        <VSpacer />
        <VBtn icon="mdi-close" variant="text" size="small" @click="close" />
      </VCardTitle>

      <VCardText>
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
          <span v-if="selectedIds.size > 0" class="ml-2 text-primary">
            · {{ selectedIds.size }} selected
          </span>
        </div>

        <!-- Entry list -->
        <VList
          v-if="filteredEntries.length > 0"
          style="max-height: 360px; overflow-y: auto;"
          lines="two"
        >
          <LibraryPickerRow
            v-for="entry in filteredEntries"
            :key="entry.id"
            :entry="entry"
            :selected="selectedIds.has(entry.id)"
            @select="toggleEntry"
          />
        </VList>

        <!-- Empty state -->
        <div
          v-else
          class="d-flex flex-column align-center justify-center text-medium-emphasis pa-8"
        >
          <VIcon size="48" class="mb-2">mdi-music-box-multiple-outline</VIcon>
          <span>No entries found</span>
        </div>
      </VCardText>

      <VDivider />

      <VCardActions>
        <VSpacer />
        <VBtn text="Cancel" variant="plain" @click="close" />
        <VBtn
          :disabled="selectedIds.size === 0"
          color="primary"
          variant="tonal"
          @click="addClips"
        >
          Add {{ selectedIds.size > 0 ? selectedIds.size : '' }} Clip{{ selectedIds.size !== 1 ? 's' : '' }}
        </VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
</template>
