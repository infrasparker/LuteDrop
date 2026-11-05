<script setup lang="ts">
// Suggests library music similar to what a track already holds. Seeds are the
// music entries behind the track's clips; results exclude what the track
// already has. Selected results are added to the track, or the whole search
// can be continued in the Vibe view.
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryEntry, VibeHit } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useLibraryPreview } from '../composables/useLibraryPreview';
import { useToast } from '../composables/useToast';
import { useVocabulary } from '../composables/useVocabulary';
import VibeResults from './VibeResults.vue';

const RESULT_LIMIT = 50;

const props = defineProps<{
  modelValue: boolean;
  trackId: string;
  seedIds: string[];
  // A seamless track holds a single clip, so results cannot be added to it.
  isSeamless?: boolean;
}>();

const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>();

const store = useAppStore();
const { playingId, toggle: togglePreview, stop: stopPreview } = useLibraryPreview();
const { vocabulary, reloadVocabulary } = useVocabulary(false);
const toast = useToast();
const router = useRouter();

const track = computed(() => store.findTrack(props.trackId));
const entriesById = computed(() => {
  const map = new Map<string, LibraryEntry>();
  for (const e of store.libraryEntries) map.set(e.id, e);
  return map;
});

const hits = ref<VibeHit[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const selectedIds = ref<string[]>([]);
const adding = ref(false);
// Seeds the user added from result rows, on top of the track's own.
const extraSeeds = ref<string[]>([]);
const allSeeds = computed(() => [...props.seedIds, ...extraSeeds.value]);
let request = 0;

// Entries already in the track are not suggested again.
const trackEntryIds = computed(() => new Set(track.value?.clips.map((c) => c.libraryEntryId) ?? []));
const shownHits = computed(() => hits.value.filter((h) => !trackEntryIds.value.has(h.entryId)));

async function load(): Promise<void> {
  const id = ++request;
  loading.value = true;
  error.value = null;
  hits.value = [];
  try {
    // Entries already in the track are filtered out, so ask for that many extra.
    const limit = RESULT_LIMIT + trackEntryIds.value.size;
    const result = await invoke<VibeHit[]>('similar_entries', { entryIds: allSeeds.value, limit });
    if (id !== request) return;
    hits.value = result;
    // Keep checked rows that are still in the results.
    const returned = new Set(result.map((h) => h.entryId));
    selectedIds.value = selectedIds.value.filter((entryId) => returned.has(entryId));
  } catch (e) {
    if (id !== request) return;
    error.value = String(e);
  } finally {
    if (id === request) loading.value = false;
  }
}

watch(
  () => props.modelValue,
  (open, wasOpen) => {
    if (open) {
      extraSeeds.value = [];
      selectedIds.value = [];
      load();
      reloadVocabulary();
    } else if (wasOpen) {
      request++;
      stopPreview();
    }
  },
  { immediate: true },
);

// --- Actions ---

function close(): void {
  emit('update:modelValue', false);
}

async function addSelected(): Promise<void> {
  const ids = selectedIds.value.filter((id) => !trackEntryIds.value.has(id));
  if (ids.length === 0 || adding.value) return;
  adding.value = true;
  try {
    const ok = await store.command('add_clips', { trackId: props.trackId, libraryEntryIds: ids });
    if (!ok) return;
    toast.showSuccess(`Added ${ids.length} to "${track.value?.name ?? 'track'}"`);
    close();
  } finally {
    adding.value = false;
  }
}

function addSeed(entryId: string): void {
  if (allSeeds.value.includes(entryId)) return;
  extraSeeds.value = [...extraSeeds.value, entryId];
  load();
}

function openInVibe(): void {
  close();
  router.push({ name: 'vibe', query: { seed: allSeeds.value } });
}
</script>

<template>
  <VDialog :model-value="modelValue" max-width="720" @update:model-value="(v) => emit('update:modelValue', v)">
    <VCard class="find-similar-card d-flex flex-column">
      <VCardTitle class="pt-4">Similar to "{{ track?.name }}"</VCardTitle>
      <VCardText class="find-similar-body d-flex flex-column pb-0">
        <VAlert v-if="error" type="error" variant="tonal" density="compact" class="mb-2">
          {{ error }}
          <template #append>
            <VBtn size="small" variant="text" @click="load">Retry</VBtn>
          </template>
        </VAlert>
        <div v-else-if="loading" class="d-flex align-center justify-center pa-6 ga-3 text-medium-emphasis">
          <VProgressCircular indeterminate size="24" width="2" />
          <span class="text-body-2">Asking the analyzer...</span>
        </div>
        <div v-else-if="shownHits.length === 0" class="text-center text-medium-emphasis pa-6 text-body-2">
          No similar tracks found beyond what this track already has.
        </div>
        <VibeResults
          v-else
          v-model:selected-ids="selectedIds"
          :hits="shownHits"
          :entries-by-id="entriesById"
          :seed-ids="allSeeds"
          :playing-id="playingId"
          :vocabulary="vocabulary"
          @play="togglePreview"
          @add-seed="addSeed"
        />
      </VCardText>
      <VCardActions class="px-4 pb-3">
        <VBtn prepend-icon="mdi-compass-outline" variant="text" @click="openInVibe">Open in Vibe</VBtn>
        <VSpacer />
        <VBtn variant="text" @click="close">Close</VBtn>
        <span v-if="isSeamless" class="text-caption text-medium-emphasis">Seamless tracks hold one clip</span>
        <VBtn
          v-else
          color="primary"
          variant="flat"
          :disabled="selectedIds.length === 0"
          :loading="adding"
          @click="addSelected"
        >
          Add {{ selectedIds.length }} to this track
        </VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
</template>

<style scoped>
.find-similar-card {
  height: min(640px, 85vh);
}

.find-similar-body {
  flex: 1 1 0;
  min-height: 0;
}
</style>
