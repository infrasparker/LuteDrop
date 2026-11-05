<script setup lang="ts">
// Ranked Scry results with Role and Pace filters. Each row has a checkbox,
// a preview button, the track name, its game tags, Role, the score as a short
// bar, and an "Add as seed" action. Clicking the row itself opens the entry's
// details.
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import type { LibraryEntry, Role, VibeHit } from '@generated/index';
import SoundFilters from './SoundFilters.vue';
import { emptySoundFilters, matchesSoundFilters, type SoundFilters as SoundFilterState } from '../utils/mood';

const props = defineProps<{
  hits: VibeHit[];
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

const roleFilter = ref<'all' | Role | 'unset'>('all');
const soundFilters = ref<SoundFilterState>(emptySoundFilters());

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
  rows.value.filter(({ entry }) => {
    const matchesRole =
      roleFilter.value === 'all' ||
      (roleFilter.value === 'unset' ? entry.role === null : entry.role === roleFilter.value);
    return matchesRole && matchesSoundFilters(entry, soundFilters.value);
  }),
);

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

function roleLabel(role: Role | null): string {
  if (role === 'combat') return 'Combat';
  if (role === 'nonCombat') return 'Non-combat';
  return 'No role';
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
  <div class="vibe-results d-flex flex-column">
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
      <VBtnToggle v-model="roleFilter" mandatory density="compact" variant="outlined" divided>
        <VBtn value="all" size="x-small">All roles</VBtn>
        <VBtn value="combat" size="x-small" prepend-icon="mdi-sword-cross">Combat</VBtn>
        <VBtn value="nonCombat" size="x-small" prepend-icon="mdi-music-note">Non-combat</VBtn>
        <VBtn value="unset" size="x-small">Unset</VBtn>
      </VBtnToggle>
      <SoundFilters v-model="soundFilters" size="x-small" />
      <VSpacer />
      <span class="text-caption text-medium-emphasis" style="white-space: nowrap;">
        {{ visibleRows.length }} of {{ rows.length }}
      </span>
    </div>

    <div ref="listEl" class="vibe-results-list flex-grow-1" @mouseleave="setHovered(null)">
      <div
        v-for="{ entry, score } in visibleRows"
        :key="entry.id"
        :data-entry-id="entry.id"
        class="vibe-row d-flex align-center ga-1 px-1"
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
          :color="entry.roleSource === 'auto' ? undefined : 'primary'"
          :title="roleLabel(entry.role) + (entry.roleSource === 'auto' ? ' · Auto' : '')"
        >
          {{ entry.role === 'combat' ? 'mdi-sword-cross' : 'mdi-music-note' }}
        </VIcon>
        <span v-else class="role-placeholder" />
        <div class="score-bar" :title="`Score ${score.toFixed(3)}`">
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
.vibe-results {
  min-height: 0;
  height: 100%;
}

.vibe-results-list {
  min-height: 0;
  overflow-y: auto;
}

.vibe-row {
  min-height: 44px;
  cursor: pointer;
  border-bottom: 1px solid rgba(var(--v-theme-on-surface), 0.06);
  border-left: 3px solid transparent;
}

.vibe-row.is-selected {
  background: rgba(var(--v-theme-primary), 0.06);
}

.vibe-row:hover {
  background: rgba(var(--v-theme-on-surface), 0.04);
}

.vibe-row.is-selected:hover {
  background: rgba(var(--v-theme-primary), 0.06);
}

.vibe-row.is-playing {
  border-left-color: rgb(var(--v-theme-primary));
}

.vibe-row.is-flash {
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
</style>
