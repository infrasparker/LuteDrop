<script setup lang="ts">
// Browse music by sound: a text search or seed tracks give ranked results,
// and a map shows every analyzed music track. Selected tracks become a new
// shuffle playlist or are added to an existing track.
import { computed, onMounted, ref, watch } from 'vue';
import { useRoute } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryEntry, SceneConfig, VibeHit, VibePoint } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useAutoTagEvents } from '../composables/useAutoTagEvents';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';
import VibeMap from '../components/VibeMap.vue';
import VibeResults from '../components/VibeResults.vue';
import CreatePlaylistDialog from '../components/CreatePlaylistDialog.vue';
import AutoTaggingDialog from '../components/AutoTaggingDialog.vue';

const RESULT_LIMIT = 100;
// Used to tell game tags from analyzer tags when the saved vocabulary cannot be read.
const DEFAULT_VOCABULARY = [
  'brass', 'folk', 'choir', 'synth', 'bright', 'dark', 'heroic', 'horror', 'lively', 'peaceful',
  'ominous', 'majestic', 'moody', 'mischievous', 'haunting', 'somber', 'slow', 'driving', 'frantic',
];

const store = useAppStore();
const route = useRoute();
const audio = useAudio();
const { completedRuns } = useAutoTagEvents();

const entriesById = computed(() => {
  const map = new Map<string, LibraryEntry>();
  for (const e of store.libraryEntries) map.set(e.id, e);
  return map;
});

const analyzedCount = computed(
  () => store.libraryEntries.filter((e) => e.entryType === 'music' && e.analysis?.status === 'embedded').length,
);

const vocabulary = ref(new Set(DEFAULT_VOCABULARY));

// --- Results (text search or seeds) ---

const searchText = ref('');
const seeds = ref<string[]>([]);
// What the current results answer: a submitted search, the seeds, or nothing yet.
const query = ref<{ kind: 'search'; text: string } | { kind: 'seeds' } | null>(null);
const hits = ref<VibeHit[]>([]);
const resultsLoading = ref(false);
const resultsError = ref<string | null>(null);

// A superseded request still completes on the backend; only the latest
// request's response is applied.
let resultsRequest = 0;

async function fetchResults(cmd: 'search_vibe' | 'similar_entries', args: Record<string, unknown>): Promise<void> {
  const id = ++resultsRequest;
  resultsLoading.value = true;
  resultsError.value = null;
  try {
    const result = await invoke<VibeHit[]>(cmd, { ...args, limit: RESULT_LIMIT });
    if (id !== resultsRequest) return;
    hits.value = result;
  } catch (e) {
    if (id !== resultsRequest) return;
    hits.value = [];
    resultsError.value = String(e);
  } finally {
    if (id === resultsRequest) resultsLoading.value = false;
  }
}

function clearResults(): void {
  resultsRequest++;
  resultsLoading.value = false;
  resultsError.value = null;
  hits.value = [];
  query.value = null;
}

function submitSearch(): void {
  const text = (searchText.value ?? '').trim();
  seeds.value = [];
  if (!text) {
    clearResults();
    return;
  }
  query.value = { kind: 'search', text };
  fetchResults('search_vibe', { text });
}

function runSeeds(): void {
  if (seeds.value.length === 0) {
    clearResults();
    return;
  }
  searchText.value = '';
  query.value = { kind: 'seeds' };
  fetchResults('similar_entries', { entryIds: seeds.value });
}

function addSeed(entryId: string): void {
  if (seeds.value.includes(entryId)) return;
  seeds.value = [...seeds.value, entryId];
  runSeeds();
}

function removeSeed(entryId: string): void {
  seeds.value = seeds.value.filter((id) => id !== entryId);
  runSeeds();
}

function retryResults(): void {
  if (query.value?.kind === 'search') {
    fetchResults('search_vibe', { text: query.value.text });
  } else if (query.value?.kind === 'seeds') {
    runSeeds();
  }
}

function seedName(entryId: string): string {
  return entriesById.value.get(entryId)?.name ?? 'Unknown track';
}

// "More like this" from the Library opens this view with `?seed=<id>`.
watch(
  () => route.query.seed,
  (raw) => {
    const ids = (Array.isArray(raw) ? raw : [raw]).filter((v): v is string => typeof v === 'string' && v.length > 0);
    if (ids.length === 0) return;
    seeds.value = [...new Set(ids)];
    runSeeds();
  },
  { immediate: true },
);

const resultsHeading = computed(() => {
  if (query.value?.kind === 'search') return `Matches for "${query.value.text}"`;
  if (query.value?.kind === 'seeds') return seeds.value.length === 1 ? 'Similar to the seed' : 'Similar to the seeds';
  return '';
});

// --- Map ---

const mapPoints = ref<VibePoint[]>([]);
const mapLoading = ref(false);
const mapError = ref<string | null>(null);
const mapLoaded = ref(false);
let mapRequest = 0;

async function loadMap(): Promise<void> {
  const id = ++mapRequest;
  mapLoading.value = true;
  mapError.value = null;
  try {
    const points = await invoke<VibePoint[]>('vibe_map');
    if (id !== mapRequest) return;
    mapPoints.value = points;
    mapLoaded.value = true;
  } catch (e) {
    if (id !== mapRequest) return;
    mapError.value = String(e);
  } finally {
    if (id === mapRequest) mapLoading.value = false;
  }
}

// Load once analyzed tracks exist (the store may hydrate after mount), and
// reload after each analysis run.
watch(
  analyzedCount,
  (n) => {
    if (n > 0 && !mapLoaded.value && !mapLoading.value && !mapError.value) loadMap();
  },
  { immediate: true },
);
watch(completedRuns, () => {
  if (analyzedCount.value > 0) loadMap();
});

const mappedPoints = computed(() => mapPoints.value.filter((p) => entriesById.value.has(p.entryId)));
const resultIds = computed(() => hits.value.map((h) => h.entryId));

// --- Preview playback ---

const previewState = computed(() => store.playbackStates.get(LIBRARY_PLAYBACK_ID));
const playingId = computed(() =>
  previewState.value?.isPlaying ? store.libraryPlayingEntryId : null,
);

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

// --- Selection and playlists ---

const selectedIds = ref<string[]>([]);

watch(entriesById, (map) => {
  if (selectedIds.value.some((id) => !map.has(id))) {
    selectedIds.value = selectedIds.value.filter((id) => map.has(id));
  }
  if (seeds.value.some((id) => !map.has(id)) && map.size > 0) {
    seeds.value = seeds.value.filter((id) => map.has(id));
  }
});

function addToSelection(ids: string[]): void {
  const current = new Set(selectedIds.value);
  selectedIds.value = [...selectedIds.value, ...ids.filter((id) => !current.has(id))];
}

const showPlaylistDialog = ref(false);
const playlistTarget = ref<'new' | 'existing'>('new');

function openPlaylistDialog(target: 'new' | 'existing'): void {
  playlistTarget.value = target;
  showPlaylistDialog.value = true;
}

const suggestedName = computed(() => {
  if (query.value?.kind === 'search') return query.value.text;
  if (query.value?.kind === 'seeds' && seeds.value.length > 0) return `Like ${seedName(seeds.value[0])}`;
  return 'Vibe playlist';
});

const showAutoTagging = ref(false);

onMounted(async () => {
  try {
    const config = await invoke<SceneConfig>('get_scene_config');
    vocabulary.value = new Set(config.vocabulary);
  } catch {
    // Keep the default vocabulary; it only decides which tags show as game tags.
  }
});
</script>

<template>
  <div class="vibe-view d-flex flex-column pa-3">
    <Teleport to="#app-bar-main">
      <VTextField
        v-model="searchText"
        placeholder="Describe a sound, e.g. tense strings building to a battle"
        prepend-inner-icon="mdi-waveform"
        variant="solo-filled"
        flat
        density="compact"
        clearable
        hide-details
        single-line
        class="flex-grow-1"
        style="max-width: 560px;"
        :disabled="analyzedCount === 0"
        @keydown.enter="submitSearch"
        @click:clear="clearResults"
      >
        <template #append-inner>
          <VBtn icon="mdi-magnify" size="x-small" variant="text" title="Search" @click="submitSearch" />
        </template>
      </VTextField>
      <VSpacer />
      <VBtn prepend-icon="mdi-creation" variant="tonal" size="small" rounded="xl" @click="showAutoTagging = true">
        Auto-tagging
      </VBtn>
    </Teleport>

    <!-- Nothing analyzed yet: explain where the data comes from. -->
    <div v-if="analyzedCount === 0" class="d-flex flex-column align-center justify-center text-center flex-grow-1 pa-8">
      <VIcon size="64" class="mb-4 text-medium-emphasis">mdi-compass-outline</VIcon>
      <div class="text-h6 mb-2">No analyzed music yet</div>
      <div class="text-body-2 text-medium-emphasis mb-4" style="max-width: 460px;">
        The Vibe view searches and maps music the analyzer has processed. Open Auto-tagging,
        connect the analyzer, and run Analyze library; analyzed music tracks then appear here.
      </div>
      <VBtn prepend-icon="mdi-creation" color="primary" variant="tonal" @click="showAutoTagging = true">
        Open Auto-tagging
      </VBtn>
    </div>

    <template v-else>
      <div v-if="seeds.length > 0" class="d-flex flex-wrap align-center ga-1 mb-2">
        <span class="text-caption text-medium-emphasis mr-1">Seeds</span>
        <VChip
          v-for="id in seeds"
          :key="id"
          size="small"
          prepend-icon="mdi-seed-outline"
          closable
          @click:close="removeSeed(id)"
        >
          {{ seedName(id) }}
        </VChip>
      </div>

      <div class="vibe-panes flex-grow-1">
        <section class="vibe-pane d-flex flex-column">
          <div v-if="resultsHeading" class="text-subtitle-2 mb-1 text-truncate">{{ resultsHeading }}</div>

          <VAlert v-if="resultsError" type="error" variant="tonal" density="compact" class="mb-2">
            {{ resultsError }}
            <template #append>
              <VBtn size="small" variant="text" @click="retryResults">Retry</VBtn>
            </template>
          </VAlert>

          <div v-if="resultsLoading" class="d-flex align-center justify-center pa-6 ga-3 text-medium-emphasis">
            <VProgressCircular indeterminate size="24" width="2" />
            <span class="text-body-2">Asking the analyzer...</span>
          </div>
          <div
            v-else-if="!query"
            class="d-flex flex-column align-center justify-center text-center text-medium-emphasis pa-6 flex-grow-1"
          >
            <VIcon size="40" class="mb-2">mdi-magnify</VIcon>
            <div class="text-body-2" style="max-width: 360px;">
              Describe a sound in the search bar and press Enter, or use "More like this" in the
              Library. Lasso tracks on the map to select them.
            </div>
          </div>
          <div
            v-else-if="!resultsError && hits.length === 0"
            class="text-body-2 text-medium-emphasis pa-6 text-center"
          >
            No results.
          </div>
          <VibeResults
            v-else-if="hits.length > 0"
            v-model:selected-ids="selectedIds"
            class="flex-grow-1"
            :hits="hits"
            :entries-by-id="entriesById"
            :seed-ids="seeds"
            :playing-id="playingId"
            :vocabulary="vocabulary"
            @play="togglePreview"
            @add-seed="addSeed"
          />
        </section>

        <section class="vibe-pane d-flex flex-column">
          <VAlert v-if="mapError" type="error" variant="tonal" density="compact" class="mb-2">
            {{ mapError }}
            <template #append>
              <VBtn size="small" variant="text" :loading="mapLoading" @click="loadMap">Retry</VBtn>
            </template>
          </VAlert>
          <div class="vibe-map-host flex-grow-1">
            <VibeMap
              :points="mappedPoints"
              :entries-by-id="entriesById"
              :seed-ids="seeds"
              :result-ids="resultIds"
              :selected-ids="selectedIds"
              :playing-id="playingId"
              @play="togglePreview"
              @lasso="addToSelection"
            />
            <div v-if="mapLoading" class="map-overlay d-flex align-center justify-center ga-3 text-medium-emphasis">
              <VProgressCircular indeterminate size="24" width="2" />
              <span class="text-body-2">Building the map...</span>
            </div>
            <div
              v-else-if="mapLoaded && mappedPoints.length === 0 && !mapError"
              class="map-overlay d-flex align-center justify-center text-body-2 text-medium-emphasis"
            >
              The analyzer returned no map points.
            </div>
          </div>
        </section>
      </div>
    </template>

    <!-- Selection bar: floats over the bottom so the panes never shift. -->
    <VFadeTransition>
      <div v-if="selectedIds.length > 0" class="selection-bar elevation-8 d-flex flex-wrap align-center ga-2 px-3 py-2">
        <VIcon size="small" color="primary">mdi-checkbox-multiple-marked</VIcon>
        <span class="text-body-2 font-weight-medium" style="white-space: nowrap;">
          {{ selectedIds.length }} selected
        </span>
        <VSpacer />
        <VBtn prepend-icon="mdi-playlist-plus" color="primary" variant="tonal" size="small" @click="openPlaylistDialog('new')">
          Create playlist
        </VBtn>
        <VBtn prepend-icon="mdi-playlist-music" variant="tonal" size="small" @click="openPlaylistDialog('existing')">
          Add to existing track
        </VBtn>
        <VBtn icon="mdi-close" variant="text" size="small" title="Clear selection" @click="selectedIds = []" />
      </div>
    </VFadeTransition>

    <CreatePlaylistDialog
      v-model="showPlaylistDialog"
      :entry-ids="selectedIds"
      :target="playlistTarget"
      :suggested-name="suggestedName"
      @done="selectedIds = []"
    />
    <AutoTaggingDialog v-model="showAutoTagging" />
  </div>
</template>

<style scoped>
.vibe-view {
  height: 100%;
  position: relative;
}

.vibe-panes {
  display: grid;
  grid-template-columns: minmax(340px, 1fr) minmax(320px, 1.2fr);
  gap: 16px;
  min-height: 0;
  /* Room for the floating selection bar. */
  padding-bottom: 56px;
}

@media (max-width: 900px) {
  .vibe-panes {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(240px, 1fr) minmax(280px, 1fr);
  }
}

.vibe-pane {
  min-height: 0;
  min-width: 0;
}

.vibe-map-host {
  position: relative;
  min-height: 0;
}

.map-overlay {
  position: absolute;
  inset: 0 0 32px 0;
  pointer-events: none;
}

.selection-bar {
  position: absolute;
  left: 50%;
  bottom: 16px;
  transform: translateX(-50%);
  width: min(720px, calc(100% - 32px));
  z-index: 2;
  background: rgb(var(--v-theme-surface));
  border: 1px solid rgba(var(--v-theme-primary), 0.5);
  border-radius: 10px;
}
</style>
