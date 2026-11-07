<script setup lang="ts">
// Ranked Scry results with mood lenses, Energy bands and a Role filter,
// sorted from the list's column headers (Name, Match, Energy, Search) like the
// Library's table. Each row has a checkbox, a preview button, the track name,
// its game tags, Role, the lens score ("Match", while a lens is on), Energy,
// the search score ("Search") as a short bar, and an "Add as seed" action. Clicking the row itself opens the entry's
// details.
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import type { LibraryEntry, Role, ScryHit } from '@generated/index';
import MoodFilters from './MoodFilters.vue';
import {
  LENS_COLUMN_TITLE,
  canSortBy,
  clickSort,
  emptyMoodFilters,
  lensLabel,
  lensOn as lensIsOn,
  lensScore,
  matchesMoodFilters,
  moodLayers,
  moodValue,
  sortByKey,
  type MoodFilters as MoodFilterState,
  type SortDir,
  type SortKey,
} from '../utils/moods';

const props = defineProps<{
  hits: ScryHit[];
  entriesById: Map<string, LibraryEntry>;
  selectedIds: string[];
  seedIds: string[];
  playingId: string | null;
  // Analyzer vocabulary tags; every other tag on an entry is shown as a game tag.
  vocabulary: Set<string>;
}>();

const emit = defineEmits<{
  'update:selectedIds': [ids: string[]];
  play: [entryId: string];
  addSeed: [entryId: string];
  // The row under the mouse, or null when it leaves.
  hover: [entryId: string | null];
  // The row itself was clicked (not one of its controls).
  open: [entryId: string];
}>();

let hoveredId: string | null = null;

const filters = ref<MoodFilterState>(emptyMoodFilters());
// "match" keeps the search ranking (the Search column).
const sortKey = ref<SortKey>('match');
const sortDir = ref<SortDir>('desc');
const lensOn = computed(() => lensIsOn(filters.value));

function sortBy(key: SortKey): void {
  if (!canSortBy(key, filters.value)) return;
  const picked = clickSort({ key: sortKey.value, dir: sortDir.value }, key);
  sortKey.value = picked.key;
  sortDir.value = picked.dir;
}

/// The arrow a header shows: none unless the list is sorted by it.
function sortIcon(key: SortKey): string | null {
  if (sortKey.value !== key) return null;
  return sortDir.value === 'asc' ? 'mdi-arrow-up' : 'mdi-arrow-down';
}

const ariaSort = (key: SortKey): 'ascending' | 'descending' | 'none' =>
  sortKey.value !== key ? 'none' : sortDir.value === 'asc' ? 'ascending' : 'descending';

interface Row { entry: LibraryEntry; score: number }

const rows = computed<Row[]>(() => {
  const out: Row[] = [];
  for (const hit of props.hits) {
    const entry = props.entriesById.get(hit.entryId);
    if (entry) out.push({ entry, score: hit.score });
  }
  return out;
});

const visibleRows = computed(() =>
  sortByKey(
    rows.value.filter(({ entry }) => matchesMoodFilters(entry, filters.value)),
    sortKey.value,
    sortDir.value,
    filters.value,
    (r) => r.entry,
  ),
);

function energyHint(entry: LibraryEntry): string {
  const { auto, user } = moodLayers(entry, 'energy');
  if (user !== null) return `Energy · Set by you${auto === null ? '' : ` · suggested ${auto.toFixed(2)}`}`;
  return 'Energy · Suggested';
}

// A filter change can hide the hovered row without a mouseleave.
watch(visibleRows, (list) => {
  if (hoveredId !== null && !list.some((r) => r.entry.id === hoveredId)) {
    hoveredId = null;
    emit('hover', null);
  }
});

function setHovered(id: string | null): void {
  hoveredId = id;
  emit('hover', id);
}

// Bars are relative to the best score in the current list.
const maxScore = computed(() => Math.max(0, ...rows.value.map((r) => r.score)));

function barWidth(score: number): string {
  if (maxScore.value <= 0) return '0%';
  return `${Math.round(Math.max(0, Math.min(1, score / maxScore.value)) * 100)}%`;
}

const selectedSet = computed(() => new Set(props.selectedIds));
const seedSet = computed(() => new Set(props.seedIds));

function gameTags(entry: LibraryEntry): string[] {
  return entry.tags.filter((t) => !props.vocabulary.has(t));
}

function toggle(id: string): void {
  if (selectedSet.value.has(id)) {
    emit('update:selectedIds', props.selectedIds.filter((x) => x !== id));
  } else {
    emit('update:selectedIds', [...props.selectedIds, id]);
  }
}

const allVisibleSelected = computed(
  () => visibleRows.value.length > 0 && visibleRows.value.every((r) => selectedSet.value.has(r.entry.id)),
);
const someVisibleSelected = computed(() => visibleRows.value.some((r) => selectedSet.value.has(r.entry.id)));

function toggleAllVisible(): void {
  const visibleIds = visibleRows.value.map((r) => r.entry.id);
  if (allVisibleSelected.value) {
    const drop = new Set(visibleIds);
    emit('update:selectedIds', props.selectedIds.filter((id) => !drop.has(id)));
  } else {
    const next = [...props.selectedIds];
    for (const id of visibleIds) if (!selectedSet.value.has(id)) next.push(id);
    emit('update:selectedIds', next);
  }
}

/// Music always has a role (ambiance has none and shows no role cell).
function roleLabel(role: Role): string {
  return role === 'combat' ? 'Combat' : 'Non-combat';
}

// --- Reveal a row (e.g. after a map click) ---

const listEl = ref<HTMLDivElement | null>(null);
const flashId = ref<string | null>(null);
let flashTimer: ReturnType<typeof setTimeout> | undefined;

// Scrolls the row into view and briefly highlights it. Returns false when the
// track is not among the rows the filters currently show.
async function reveal(entryId: string): Promise<boolean> {
  if (!visibleRows.value.some((r) => r.entry.id === entryId)) return false;
  // Restart the flash animation when the same row is revealed again.
  flashId.value = null;
  await nextTick();
  const row = listEl.value?.querySelector<HTMLElement>(`[data-entry-id="${CSS.escape(entryId)}"]`);
  if (!row) return false;
  row.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  flashId.value = entryId;
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => {
    flashId.value = null;
  }, 1600);
  return true;
}

defineExpose({ reveal });

onBeforeUnmount(() => {
  clearTimeout(flashTimer);
  emit('hover', null);
});
</script>

<template>
  <div class="scry-results d-flex flex-column">
    <div class="d-flex flex-wrap align-center ga-2 mb-2">
      <VCheckboxBtn
        :model-value="allVisibleSelected"
        :indeterminate="someVisibleSelected && !allVisibleSelected"
        :disabled="visibleRows.length === 0"
        density="compact"
        class="flex-grow-0"
        title="Select all shown"
        aria-label="Select all shown results"
        @update:model-value="toggleAllVisible"
      />
      <MoodFilters
        v-model="filters"
        v-model:sort="sortKey"
        v-model:dir="sortDir"
        size="x-small"
        default-sort="match"
        class="flex-grow-1"
        style="min-width: 0;"
      />
      <VSpacer />
      <span class="text-caption text-medium-emphasis" style="white-space: nowrap;">
        {{ visibleRows.length }} of {{ rows.length }}
      </span>
    </div>

    <!-- Column headers: the only sort control, laid out like the rows below. -->
    <div v-if="rows.length > 0" class="scry-head d-flex align-center ga-1 px-1 text-caption" role="row">
      <span class="head-lead" />
      <button type="button" class="head-btn flex-grow-1 text-left" :aria-sort="ariaSort('name')" @click="sortBy('name')">
        Name<VIcon v-if="sortIcon('name')" size="x-small">{{ sortIcon('name') }}</VIcon>
      </button>
      <span class="role-placeholder" />
      <button
        v-if="lensOn"
        type="button"
        class="head-btn lens-head text-left"
        :title="`${LENS_COLUMN_TITLE}: ${lensLabel(filters)}`"
        :aria-sort="ariaSort('lens')"
        @click="sortBy('lens')"
      >
        <span class="text-truncate">{{ LENS_COLUMN_TITLE }}</span>
        <VIcon v-if="sortIcon('lens')" size="x-small">{{ sortIcon('lens') }}</VIcon>
      </button>
      <!-- While a lens ranks the list, Energy is not a sort: a plain label. -->
      <span v-if="lensOn" class="head-static energy-head">Energy</span>
      <button v-else type="button" class="head-btn energy-head" :aria-sort="ariaSort('energy')" @click="sortBy('energy')">
        Energy<VIcon v-if="sortIcon('energy')" size="x-small">{{ sortIcon('energy') }}</VIcon>
      </button>
      <button
        type="button"
        class="head-btn match-head"
        title="Search: how close to the search or seeds"
        :aria-sort="ariaSort('match')"
        @click="sortBy('match')"
      >
        Search<VIcon v-if="sortIcon('match')" size="x-small">{{ sortIcon('match') }}</VIcon>
      </button>
      <span class="head-trail" />
    </div>

    <div
      ref="listEl"
      class="scry-results-list flex-grow-1"
      :class="{ 'has-selection': selectedIds.length > 0 }"
      @mouseleave="setHovered(null)"
    >
      <div
        v-for="{ entry, score } in visibleRows"
        :key="entry.id"
        :data-entry-id="entry.id"
        class="scry-row d-flex align-center ga-1 px-1"
        :class="{
          'is-playing': playingId === entry.id,
          'is-selected': selectedSet.has(entry.id),
          'is-flash': flashId === entry.id,
        }"
        @mouseenter="setHovered(entry.id)"
        @click="emit('open', entry.id)"
      >
        <!-- The wrapper keeps checkbox clicks from opening details. -->
        <span class="d-flex flex-grow-0" @click.stop>
          <VCheckboxBtn
            :model-value="selectedSet.has(entry.id)"
            density="compact"
            class="flex-grow-0"
            :aria-label="`Select ${entry.name}`"
            @update:model-value="toggle(entry.id)"
          />
        </span>
        <VBtn
          :icon="playingId === entry.id ? 'mdi-stop' : 'mdi-play'"
          variant="text"
          size="small"
          :title="playingId === entry.id ? 'Stop' : 'Play'"
          @click.stop="emit('play', entry.id)"
        />
        <div class="flex-grow-1" style="min-width: 0;">
          <div class="text-body-2 text-truncate" :title="entry.name">{{ entry.name }}</div>
          <div v-if="gameTags(entry).length > 0" class="d-flex ga-1 overflow-hidden">
            <VChip v-for="tag in gameTags(entry)" :key="tag" size="x-small" variant="tonal" label>{{ tag }}</VChip>
          </div>
        </div>
        <VIcon
          v-if="entry.role"
          size="small"
          :color="entry.roleUser === null ? undefined : 'primary'"
          :title="roleLabel(entry.role) + (entry.roleUser === null ? ' · Suggested' : '')"
        >
          {{ entry.role === 'combat' ? 'mdi-sword-cross' : 'mdi-music-note' }}
        </VIcon>
        <span v-else class="role-placeholder" />
        <div v-if="lensOn" class="lens-cell" :title="lensLabel(filters)">
          <template v-if="lensScore(entry, filters) !== null">
            <div class="lens-bar">
              <div class="lens-fill" :style="{ width: `${(lensScore(entry, filters) ?? 0) * 100}%` }" />
            </div>
            <span class="num">{{ (lensScore(entry, filters) ?? 0).toFixed(2) }}</span>
          </template>
          <span v-else class="num text-medium-emphasis">—</span>
        </div>
        <span
          class="num energy-cell"
          :class="moodLayers(entry, 'energy').user === null ? 'text-medium-emphasis' : 'text-primary'"
          :title="energyHint(entry)"
        >
          <VIcon size="x-small">mdi-lightning-bolt</VIcon>{{ moodValue(entry, 'energy')?.toFixed(2) ?? '—' }}
        </span>
        <div class="score-bar" :class="{ 'is-muted': lensOn }" :title="`Search ${score.toFixed(3)}`">
          <div class="score-fill" :style="{ width: barWidth(score) }" />
        </div>
        <VBtn
          icon="mdi-seed-plus-outline"
          variant="text"
          size="small"
          :disabled="seedSet.has(entry.id)"
          title="Add as seed"
          @click.stop="emit('addSeed', entry.id)"
        />
      </div>
      <div v-if="rows.length === 0" class="text-body-2 text-medium-emphasis pa-6 text-center">
        No results.
      </div>
      <div v-else-if="visibleRows.length === 0" class="text-caption text-medium-emphasis pa-4 text-center">
        No results match these filters.
      </div>
    </div>
  </div>
</template>

<style scoped>
/* Caption size set here: Vuetify 4 has no `text-caption` class, and at body
   size "Search ↓" and "Energy" overflow their columns into each other. */
.scry-head {
  font-size: 0.75rem;
  min-height: 28px;
  border-bottom: 1px solid rgba(var(--v-theme-on-surface), 0.12);
  border-left: 3px solid transparent;
  color: rgba(var(--v-theme-on-surface), var(--v-medium-emphasis-opacity));
}

/* Checkbox + play button, and the seed button, of a row. */
.head-lead {
  flex: 0 0 76px;
}
.head-trail {
  flex: 0 0 36px;
}

.head-btn {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  min-width: 0;
  font: inherit;
  color: inherit;
  background: none;
  border: 0;
  padding: 0;
  cursor: pointer;
  white-space: nowrap;
}
.head-btn:hover,
.head-btn[aria-sort='ascending'],
.head-btn[aria-sort='descending'] {
  color: rgb(var(--v-theme-on-surface));
}

.head-static {
  display: inline-flex;
  align-items: center;
  white-space: nowrap;
}

.lens-head {
  flex: 0 0 92px;
  overflow: hidden;
}
.energy-head {
  flex: 0 0 44px;
  justify-content: flex-end;
}
.match-head {
  flex: 0 0 56px;
  justify-content: flex-end;
}

.scry-results {
  min-height: 0;
  height: 100%;
}

.scry-results-list {
  min-height: 0;
  overflow-y: auto;
}

/* The selection bar floats over the bottom of the view; this lets the last
   rows scroll up from under it without shrinking the panes. */
.scry-results-list.has-selection {
  padding-bottom: 64px;
}

.scry-row {
  min-height: 44px;
  cursor: pointer;
  border-bottom: 1px solid rgba(var(--v-theme-on-surface), 0.06);
  border-left: 3px solid transparent;
}

.scry-row.is-selected {
  background: rgba(var(--v-theme-primary), 0.06);
}

.scry-row:hover {
  background: rgba(var(--v-theme-on-surface), 0.04);
}

.scry-row.is-selected:hover {
  background: rgba(var(--v-theme-primary), 0.06);
}

.scry-row.is-playing {
  border-left-color: rgb(var(--v-theme-primary));
}

.scry-row.is-flash {
  animation: row-flash 1.6s ease-out;
}

@keyframes row-flash {
  0%,
  30% {
    background: rgba(var(--v-theme-primary), 0.28);
  }
  100% {
    background: transparent;
  }
}

.role-placeholder {
  display: inline-block;
  width: 20px;
}

.lens-cell {
  flex: 0 0 92px;
  display: flex;
  align-items: center;
  gap: 4px;
}

.lens-bar {
  flex: 1 1 auto;
  height: 6px;
  border-radius: 3px;
  background: rgba(var(--v-theme-on-surface), 0.1);
  overflow: hidden;
}

.lens-fill {
  height: 100%;
  background: rgb(var(--v-theme-primary));
}

.num {
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.energy-cell {
  flex: 0 0 44px;
  text-align: right;
}

.score-bar {
  flex: 0 0 56px;
  height: 6px;
  border-radius: 3px;
  background: rgba(var(--v-theme-on-surface), 0.1);
  overflow: hidden;
}

.score-fill {
  height: 100%;
  background: rgb(var(--v-theme-primary));
}

/* While a lens ranks the list, the match bar is muted so the lens bar reads as the ranking. */
.score-bar.is-muted .score-fill {
  background: rgba(var(--v-theme-on-surface), 0.45);
}
</style>
