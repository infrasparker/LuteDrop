<script setup lang="ts">
// Browse music by sound: a text search or seed tracks give ranked results,
// and a map shows every analyzed music track. Selected tracks become clips:
// on a new comp (one shuffle track) or appended to an existing track. With `?target=<trackId>`
// (Find similar on a Soundstage track) results leave out what that track
// already holds and selections are added straight to it.
import { computed, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';
import type { LibraryEntry, ScryHit, ScryPoint } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAutoTagEvents } from '../composables/useAutoTagEvents';
import { useLibraryPreview } from '../composables/useLibraryPreview';
import { useToast } from '../composables/useToast';
import ScryMap from '../components/ScryMap.vue';
import ScryResults from '../components/ScryResults.vue';
import CreatePlaylistDialog from '../components/CreatePlaylistDialog.vue';
import AutoTaggingDialog from '../components/AutoTaggingDialog.vue';

const RESULT_LIMIT = 100;

const store = useAppStore();
const route = useRoute();
const router = useRouter();
const toast = useToast();
const { playingId, toggle: togglePreview } = useLibraryPreview();
const { completedRuns } = useAutoTagEvents();

const entriesById = computed(() => {
  const map = new Map<string, LibraryEntry>();
  for (const e of store.libraryEntries) map.set(e.id, e);
  return map;
});

const analyzedCount = computed(
  () => store.libraryEntries.filter((e) => e.entryType === 'music' && e.analysis?.status === 'embedded').length,
);

// --- Target track (Find similar from the Soundstage) ---

// The comp and track named by `?target=`; null when absent or unknown, which
// leaves the view in its normal mode.
const target = computed(() => {
  const id = route.query.target;
  if (typeof id !== 'string' || !id) return null;
  for (const group of store.groups) {
    for (const comp of group.comps) {
      const track = comp.tracks.find((t) => t.id === id);
      if (track) return { comp, track };
    }
  }
  return null;
});

// If the target track is deleted while the page is open, leave target mode.
watch(target, (now, before) => {
  if (before && !now && route.query.target) {
    toast.showError('That track no longer exists');
    const { target: _gone, ...rest } = route.query;
    void router.replace({ query: rest });
  }
});

const targetEntryIds = computed(() => new Set(target.value?.track.clips.map((c) => c.libraryEntryId) ?? []));
// A seamless track plays a single clip, so results are not added to it.
const targetIsSeamless = computed(() => target.value?.track.mode.type === 'seamless');

// Back to where the user came from (the Soundstage keeps its place), or to
// the Soundstage when there is no history entry to return to.
function goBack(): void {
  if (window.history.state?.back) router.back();
  else router.push({ name: 'soundstage' });
}

// --- Results (text search or seeds) ---

// Vibe asks the analyzer to match a description; Name filters track names as you type.
const searchMode = ref<'vibe' | 'name'>('vibe');
const searchText = ref('');
const seeds = ref<string[]>([]);
// A clicked empty spot on the map, in map coordinates.
const spot = ref<{ x: number; y: number } | null>(null);
// What the current results answer: a vibe search, a name search, a map spot,
// the seeds, or nothing yet.
type Query =
  | { kind: 'search'; text: string }
  | { kind: 'name'; text: string }
  | { kind: 'spot' }
  | { kind: 'seeds' };
const query = ref<Query | null>(null);
const hits = ref<ScryHit[]>([]);
const resultsLoading = ref(false);
const resultsError = ref<string | null>(null);

// A superseded request still completes on the backend; only the latest
// request's response is applied.
let resultsRequest = 0;

async function fetchResults(cmd: 'search_scry' | 'similar_entries', args: Record<string, unknown>): Promise<void> {
  const id = ++resultsRequest;
  resultsLoading.value = true;
  resultsError.value = null;
  try {
    // Entries already in the target track are hidden, so ask for that many extra.
    const limit = RESULT_LIMIT + targetEntryIds.value.size;
    const result = await invoke<ScryHit[]>(cmd, { ...args, limit });
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

// Results computed here rather than by the analyzer; any request still in
// flight is superseded.
function setLocalHits(next: ScryHit[]): void {
  resultsRequest++;
  resultsLoading.value = false;
  resultsError.value = null;
  hits.value = next;
}

function clearResults(): void {
  setLocalHits([]);
  query.value = null;
}

function submitSearch(): void {
  const text = (searchText.value ?? '').trim();
  if (!text) return;
  if (searchMode.value === 'name') {
    runNameSearch(text);
    return;
  }
  seeds.value = [];
  spot.value = null;
  query.value = { kind: 'search', text };
  fetchResults('search_scry', { text });
}

// Analyzed music whose name contains the text: names starting with it rank
// first, then names with a word starting with it, then the rest.
function runNameSearch(text: string): void {
  const needle = text.toLowerCase();
  const found: ScryHit[] = [];
  for (const e of store.libraryEntries) {
    if (e.entryType !== 'music' || e.analysis?.status !== 'embedded') continue;
    const name = e.name.toLowerCase();
    const at = name.indexOf(needle);
    if (at < 0) continue;
    const wordStart = at === 0 || /[^a-z0-9]/.test(name[at - 1]);
    found.push({ entryId: e.id, score: at === 0 ? 1 : wordStart ? 0.75 : 0.5 });
  }
  found.sort((a, b) => b.score - a.score || seedName(a.entryId).localeCompare(seedName(b.entryId)));
  seeds.value = [];
  spot.value = null;
  query.value = { kind: 'name', text };
  setLocalHits(found.slice(0, RESULT_LIMIT));
}

// Name mode searches as you type.
watch(searchText, (raw) => {
  if (searchMode.value !== 'name') return;
  const text = (raw ?? '').trim();
  if (text) runNameSearch(text);
  else if (query.value?.kind === 'name') clearResults();
});

// Switching modes re-runs whatever is typed in the new mode.
function toggleSearchMode(): void {
  searchMode.value = searchMode.value === 'vibe' ? 'name' : 'vibe';
  if ((searchText.value ?? '').trim()) submitSearch();
}

function clearSpot(): void {
  spot.value = null;
  if (query.value?.kind === 'spot') clearResults();
}

// Shift-right-click on the map: drop the spot and every seed.
function clearSpotAndSeeds(): void {
  clearSpot();
  if (seeds.value.length > 0) {
    seeds.value = [];
    runSeeds();
  }
}

// A spot click returns as many tracks as a seed or vibe search does.
const SPOT_LIMIT = RESULT_LIMIT;

// The tracks nearest a clicked empty spot on the map, closest first.
function runSpot(pos: { x: number; y: number }): void {
  const ranked = mappedPoints.value
    .map((p) => ({ entryId: p.entryId, d: Math.hypot(p.x - pos.x, p.y - pos.y) }))
    .sort((a, b) => a.d - b.d)
    .slice(0, SPOT_LIMIT);
  if (ranked.length === 0) return;
  seeds.value = [];
  searchText.value = '';
  spot.value = pos;
  query.value = { kind: 'spot' };
  // Closest gets the full bar; the farthest of the set still shows a sliver.
  const far = ranked[ranked.length - 1].d || 1;
  setLocalHits(ranked.map((r) => ({ entryId: r.entryId, score: 1 - 0.9 * (r.d / far) })));
}

function runSeeds(): void {
  if (seeds.value.length === 0) {
    clearResults();
    return;
  }
  searchText.value = '';
  spot.value = null;
  query.value = { kind: 'seeds' };
  fetchResults('similar_entries', { entryIds: seeds.value });
}

function addSeed(entryId: string): void {
  if (seeds.value.includes(entryId)) return;
  seeds.value = [...seeds.value, entryId];
  runSeeds();
}

// From the map: a seed click toggles the track in or out of the seeds.
function toggleSeed(entryId: string): void {
  if (seeds.value.includes(entryId)) removeSeed(entryId);
  else addSeed(entryId);
}

function removeSeed(entryId: string): void {
  seeds.value = seeds.value.filter((id) => id !== entryId);
  runSeeds();
}

// The search field's clear button only clears text-search results; seed
// results stay.
function onSearchCleared(): void {
  if (query.value?.kind === 'search' || query.value?.kind === 'name') clearResults();
}

function retryResults(): void {
  if (query.value?.kind === 'search') {
    fetchResults('search_scry', { text: query.value.text });
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
  if (query.value?.kind === 'name') return `Names containing "${query.value.text}"`;
  if (query.value?.kind === 'spot') return 'Nearest to the spot you clicked';
  if (query.value?.kind === 'seeds') return seeds.value.length === 1 ? 'Similar to the seed' : 'Similar to the seeds';
  return '';
});

// --- Map ---

const mapPoints = ref<ScryPoint[]>([]);
const mapLoading = ref(false);
const mapError = ref<string | null>(null);
const mapLoaded = ref(false);
let mapRequest = 0;

async function loadMap(): Promise<void> {
  const id = ++mapRequest;
  mapLoading.value = true;
  mapError.value = null;
  try {
    const points = await invoke<ScryPoint[]>('scry_map');
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
// Results shown: in target mode, without what the track already holds.
const shownHits = computed(() =>
  targetEntryIds.value.size === 0 ? hits.value : hits.value.filter((h) => !targetEntryIds.value.has(h.entryId)),
);
const resultIds = computed(() => shownHits.value.map((h) => h.entryId));

// --- Selection and playlists ---

const selectedIds = ref<string[]>([]);

watch(entriesById, (map) => {
  if (selectedIds.value.some((id) => !map.has(id))) {
    selectedIds.value = selectedIds.value.filter((id) => map.has(id));
  }
  // Seeds whose entries were deleted are dropped and the search re-run (or the
  // results cleared when no seeds remain).
  if (seeds.value.some((id) => !map.has(id)) && map.size > 0) {
    seeds.value = seeds.value.filter((id) => map.has(id));
    runSeeds();
  }
});

function addToSelection(ids: string[]): void {
  const current = new Set(selectedIds.value);
  selectedIds.value = [...selectedIds.value, ...ids.filter((id) => !current.has(id))];
}

function removeFromSelection(ids: string[]): void {
  const drop = new Set(ids);
  if (selectedIds.value.some((id) => drop.has(id))) {
    selectedIds.value = selectedIds.value.filter((id) => !drop.has(id));
  }
}

// --- Linking the results list and the map ---

const resultsList = ref<InstanceType<typeof ScryResults> | null>(null);
// Results row under the mouse; its map point is highlighted.
const hoveredRowId = ref<string | null>(null);

watch(hits, () => {
  hoveredRowId.value = null;
});

// A map click plays the track and scrolls to its results row, if it has one.
function onMapPlay(entryId: string): void {
  togglePreview(entryId);
  resultsList.value?.reveal(entryId);
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
  return 'Scry comp';
});

// --- Adding to the target track ---

// A map lasso can catch entries the track already holds; those are skipped.
const addableIds = computed(() => selectedIds.value.filter((id) => !targetEntryIds.value.has(id)));
const adding = ref(false);

async function addToTarget(): Promise<void> {
  const t = target.value;
  const ids = addableIds.value;
  if (!t || ids.length === 0 || adding.value) return;
  adding.value = true;
  try {
    const ok = await store.command('add_clips', { trackId: t.track.id, libraryEntryIds: ids });
    if (!ok) return;
    toast.showSuccess(`Added ${ids.length} clip${ids.length === 1 ? '' : 's'} to "${t.track.name}"`);
    selectedIds.value = [];
  } finally {
    adding.value = false;
  }
}

function openDetails(entryId: string): void {
  store.editEntryId = entryId;
}

const showAnalyzer = ref(false);
</script>

<template>
  <div class="scry-view d-flex flex-column pa-3">
    <Teleport to="#app-bar-main">
      <VTextField
        v-model="searchText"
        :placeholder="searchMode === 'vibe'
          ? 'Describe a sound, e.g. tense strings building to a battle'
          : 'Search track names'"
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
        @click:clear="onSearchCleared"
      >
        <template #prepend-inner>
          <VBtn
            size="x-small"
            variant="tonal"
            rounded="xl"
            :prepend-icon="searchMode === 'vibe' ? 'mdi-waveform' : 'mdi-format-letter-case'"
            :title="searchMode === 'vibe' ? 'Searching by vibe; click to search by name' : 'Searching by name; click to search by vibe'"
            class="mr-1"
            @click.stop="toggleSearchMode"
          >
            {{ searchMode === 'vibe' ? 'Vibe' : 'Name' }}
          </VBtn>
        </template>
        <template #append-inner>
          <VBtn icon="mdi-magnify" size="x-small" variant="text" title="Search" @click="submitSearch" />
        </template>
      </VTextField>
      <VSpacer />
      <VBtn prepend-icon="mdi-creation" variant="tonal" size="small" rounded="xl" @click="showAnalyzer = true">
        Analyzer
      </VBtn>
    </Teleport>

    <div v-if="target" class="target-strip d-flex align-center ga-2 mb-2 px-2 py-1">
      <VBtn prepend-icon="mdi-arrow-left" variant="text" size="small" @click="goBack">Back</VBtn>
      <VIcon size="small" color="primary">mdi-compass-outline</VIcon>
      <span class="text-body-2 text-truncate">
        Finding tracks for <strong>{{ target.comp.name }}</strong> &rsaquo; <strong>{{ target.track.name }}</strong>
      </span>
      <VSpacer />
      <span v-if="targetIsSeamless" class="text-caption text-medium-emphasis" style="white-space: nowrap;">
        Seamless tracks hold one clip
      </span>
    </div>

    <!-- Nothing analyzed yet: explain where the data comes from. -->
    <div v-if="analyzedCount === 0" class="d-flex flex-column align-center justify-center text-center flex-grow-1 pa-8">
      <VIcon size="64" class="mb-4 text-medium-emphasis">mdi-compass-outline</VIcon>
      <div class="text-h6 mb-2">No analyzed music yet</div>
      <div class="text-body-2 text-medium-emphasis mb-4" style="max-width: 460px;">
        Scry searches and maps music the analyzer has processed. Open Analyzer,
        connect the analyzer, and run Analyze library; analyzed music tracks then appear here.
      </div>
      <VBtn prepend-icon="mdi-creation" color="primary" variant="tonal" @click="showAnalyzer = true">
        Open Analyzer
      </VBtn>
    </div>

    <template v-else>
      <div class="scry-panes flex-grow-1">
        <section class="scry-pane d-flex flex-column">
          <div v-if="resultsHeading" class="text-subtitle-2 mb-1 text-truncate">{{ resultsHeading }}</div>
          <!-- Seed and spot chips sit in the results pane, so adding or
               removing one never resizes the map. -->
          <div v-if="query?.kind === 'spot'" class="d-flex flex-wrap align-center ga-1 mb-2">
            <span class="text-caption text-medium-emphasis mr-1">Searching near</span>
            <VChip size="small" prepend-icon="mdi-crosshairs-gps" closable @click:close="clearSpot">
              Map spot
            </VChip>
          </div>
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
              Describe a sound in the search bar and press Enter (or switch it to Name), shift-click
              an empty spot on the map for the tracks nearby, or use "More like this" in the Library.
              Lasso tracks on the map to select them.
            </div>
          </div>
          <ScryResults
            v-else-if="!resultsError"
            ref="resultsList"
            v-model:selected-ids="selectedIds"
            class="scry-results-host"
            :hits="shownHits"
            :entries-by-id="entriesById"
            :seed-ids="seeds"
            :playing-id="playingId"
            @play="togglePreview"
            @add-seed="addSeed"
            @hover="hoveredRowId = $event"
            @open="openDetails"
          />
        </section>

        <section class="scry-pane d-flex flex-column">
          <VAlert v-if="mapError" type="error" variant="tonal" density="compact" class="mb-2">
            {{ mapError }}
            <template #append>
              <VBtn size="small" variant="text" :loading="mapLoading" @click="loadMap">Retry</VBtn>
            </template>
          </VAlert>
          <div class="scry-map-host flex-grow-1">
            <ScryMap
              :points="mappedPoints"
              :entries-by-id="entriesById"
              :seed-ids="seeds"
              :result-ids="resultIds"
              :selected-ids="selectedIds"
              :playing-id="playingId"
              :highlight-id="hoveredRowId"
              :spot="query?.kind === 'spot' ? spot : null"
              @play="onMapPlay"
              @spot="runSpot"
              @seed="toggleSeed"
              @clear-search="clearSpotAndSeeds"
              @clear-selection="selectedIds = []"
              @lasso="addToSelection"
              @deselect="removeFromSelection"
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
          {{ target ? addableIds.length : selectedIds.length }} selected
        </span>
        <VSpacer />
        <VBtn
          v-if="target && !targetIsSeamless"
          prepend-icon="mdi-plus"
          color="primary"
          variant="flat"
          size="small"
          :disabled="addableIds.length === 0"
          :loading="adding"
          @click="addToTarget"
        >
          Add {{ addableIds.length }} to {{ target.track.name }}
        </VBtn>
        <VBtn
          prepend-icon="mdi-playlist-plus"
          :color="target && !targetIsSeamless ? undefined : 'primary'"
          variant="tonal"
          size="small"
          @click="openPlaylistDialog('new')"
        >
          New comp
        </VBtn>
        <VBtn prepend-icon="mdi-playlist-music" variant="tonal" size="small" @click="openPlaylistDialog('existing')">
          Add to track
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
    <AutoTaggingDialog v-model="showAnalyzer" />
  </div>
</template>

<style scoped>
/* Exactly the space between the app bar and the transport bar, so the page
   never scrolls: the results list scrolls inside its pane instead. */
.scry-view {
  height: calc(100vh - var(--v-layout-top, 0px) - var(--v-layout-bottom, 0px));
  position: relative;
  overflow: hidden;
}

.target-strip {
  flex: none;
  min-width: 0;
  border-radius: 8px;
  background: rgba(var(--v-theme-primary), 0.08);
}

.scry-panes {
  display: grid;
  grid-template-columns: minmax(340px, 1fr) minmax(320px, 1.2fr);
  grid-template-rows: minmax(0, 1fr);
  gap: 16px;
  min-height: 0;
}

@media (max-width: 900px) {
  .scry-panes {
    grid-template-columns: 1fr;
    grid-template-rows: minmax(0, 1fr) minmax(0, 1fr);
  }
}

.scry-pane {
  min-height: 0;
  min-width: 0;
  overflow: hidden;
}

.scry-results-host {
  flex: 1 1 0;
  min-height: 0;
}

.scry-map-host {
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
