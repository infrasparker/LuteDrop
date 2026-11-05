<script setup lang="ts">
// Suggests library music similar to what a track already holds. Seeds are the
// music entries behind the track's clips; results exclude what the track
// already has. Selected results are added to the track, or the whole search
// can be continued in the Vibe view.
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryEntry, SceneConfig, VibeHit } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useToast } from '../composables/useToast';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';
import VibeResults from './VibeResults.vue';

const RESULT_LIMIT = 50;
// Used to tell game tags from analyzer tags when the saved vocabulary cannot be read.
const DEFAULT_VOCABULARY = [
  'brass', 'folk', 'choir', 'synth', 'bright', 'dark', 'heroic', 'horror', 'lively', 'peaceful',
  'ominous', 'majestic', 'moody', 'mischievous', 'haunting', 'somber', 'slow', 'driving', 'frantic',
];

const props = defineProps<{
  modelValue: boolean;
  trackId: string;
  seedIds: string[];
}>();

const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>();

const store = useAppStore();
const audio = useAudio();
const toast = useToast();
const router = useRouter();

const track = computed(() => store.findTrack(props.trackId));
const entriesById = computed(() => {
  const map = new Map<string, LibraryEntry>();
  for (const e of store.libraryEntries) map.set(e.id, e);
  return map;
});

const vocabulary = ref(new Set(DEFAULT_VOCABULARY));
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
  selectedIds.value = [];
  try {
    const result = await invoke<VibeHit[]>('similar_entries', { entryIds: allSeeds.value, limit: RESULT_LIMIT });
    if (id !== request) return;
    hits.value = result;
  } catch (e) {
    if (id !== request) return;
    error.value = String(e);
  } finally {
    if (id === request) loading.value = false;
  }
}

async function loadVocabulary(): Promise<void> {
  try {
    const config = await invoke<SceneConfig>('get_scene_config');
    vocabulary.value = new Set(config.vocabulary);
  } catch {
    // Keep the default vocabulary; it only decides which tags show as game tags.
  }
}

watch(
  () => props.modelValue,
  (open) => {
    if (open) {
      extraSeeds.value = [];
      load();
      loadVocabulary();
    } else {
      request++;
      stopPreview();
    }
  },
  { immediate: true },
);

// --- Preview playback ---

const playingId = computed(() =>
  store.playbackStates.get(LIBRARY_PLAYBACK_ID)?.isPlaying ? store.libraryPlayingEntryId : null,
);

function stopPreview(): void {
  if (playingId.value) audio.stopLibrarySound();
}

function togglePreview(entryId: string): void {
  if (playingId.value === entryId) {
    audio.stopLibrarySound();
    return;
  }
  const entry = entriesById.value.get(entryId);
  if (!entry) return;
  audio.playLibrarySound(
    entry.id,
    entry.filePath,
    entry.trimStart ?? 0,
    entry.volume,
    entry.trimStart, entry.trimEnd,
    entry.loopStart, entry.loopEnd,
  );
}

// --- Actions ---

function close(): void {
  emit('update:modelValue', false);
}

async function addSelected(): Promise<void> {
  const ids = selectedIds.value.filter((id) => !trackEntryIds.value.has(id));
  if (ids.length === 0 || adding.value) return;
  adding.value = true;
  try {
    await store.command('add_clips', { trackId: props.trackId, libraryEntryIds: ids });
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
        <VBtn
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
