<script setup lang="ts">
import { computed, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useAppStore } from '../stores/appStore';
import FilterChip from './FilterChip.vue';
import {
  ENERGY_BANDS,
  cycleMood,
  lensOn,
  moodsOnly,
  sortForLens,
  toggleIn,
  type EnergyBand,
  type MoodFilters,
  type SortDir,
  type SortKey,
} from '../utils/moods';

/// The filter bar shared by the Library and Scry: a lens chip per mood of the
/// vocabulary, then the three Energy band chips (Low, Mid, High), then the
/// Role toggle. While the vocabulary needs recalibrating, a marker after the
/// chips says so and opens the Moods page.
///
/// Moods cycle Off/High/Low (right-click reverses). Their signed scores are
/// combined with a geometric mean to rank the list, never to exclude tracks.
/// Energy bands and role remain filters. The first mood selects Match sorting;
/// clearing the last one restores the view's default sort.
const filters = defineModel<MoodFilters>({ required: true });
const sort = defineModel<SortKey>('sort', { required: true });
const dir = defineModel<SortDir>('dir', { required: true });
const props = defineProps<{
  size: 'small' | 'x-small';
  /// The view's own sort, used again once the last lens is cleared: Name in
  /// the Library, Match (search ranking) in Scry.
  defaultSort: SortKey;
}>();

const store = useAppStore();
const router = useRouter();
const moods = computed(() => moodsOnly());

// A lens that is not a mood of the vocabulary (removed, or Energy) is dropped.
watch(moods, (list) => {
  const known = (id: string): boolean => list.some((m) => m.id === id);
  const lenses = filters.value.lenses;
  const lowLenses = filters.value.lowLenses ?? [];
  if ([...lenses, ...lowLenses].every(known)) return;
  filters.value = { ...filters.value, lenses: lenses.filter(known), lowLenses: lowLenses.filter(known) };
});

watch(
  () => lensOn(filters.value),
  (isOn, wasOn) => {
    const next = sortForLens({ key: sort.value, dir: dir.value }, wasOn, isOn, props.defaultSort);
    sort.value = next.key;
    dir.value = next.dir;
  },
);

function toggleLens(id: string, direction: 1 | -1): void {
  filters.value = cycleMood(filters.value, id, direction);
}

function toggleBand(id: EnergyBand): void {
  filters.value = { ...filters.value, bands: toggleIn(filters.value.bands, id) };
}

function setRole(role: MoodFilters['role']): void {
  filters.value = { ...filters.value, role };
}
</script>

<template>
  <div class="d-flex flex-wrap align-center ga-2">
    <div class="d-flex flex-wrap align-center ga-1 lens-chips" role="group" aria-label="Mood lenses">
      <FilterChip
        v-for="m in moods"
        :key="m.id"
        :label="m.name"
        :state="filters.lenses.includes(m.id) ? 'include' : filters.lowLenses?.includes(m.id) ? 'exclude' : 'off'"
        :size="size"
        off-variant="outlined"
        polarity
        :detail="'Ranks by the geometric mean of High scores and Low complements; no cutoff'"
        @cycle="(direction) => toggleLens(m.id, direction)"
      />
    </div>
    <div class="d-flex align-center ga-1 energy-bands" role="group" aria-label="Energy bands">
      <VIcon :size="size" class="text-medium-emphasis" title="Energy" aria-hidden="true">mdi-lightning-bolt</VIcon>
      <FilterChip
        v-for="b in ENERGY_BANDS"
        :key="b.id"
        :label="b.name"
        :detail="b.hint"
        :state="filters.bands.includes(b.id) ? 'include' : 'off'"
        :size="size"
        off-variant="outlined"
        toggle
        @cycle="toggleBand(b.id)"
      />
    </div>
    <VBtn
      v-if="store.moodsNeedRecalibration"
      :size="size"
      variant="text"
      color="warning"
      prepend-icon="mdi-alert-circle-outline"
      class="text-none"
      @click="router.push('/moods')"
    >
      Needs recalibration
      <VTooltip activator="parent" location="top" open-delay="300">
        Some mood prompts changed since the last calibration. Those moods are not scored until the next analysis run or a Recalibrate.
      </VTooltip>
    </VBtn>
    <VBtnToggle
      :model-value="filters.role"
      mandatory
      density="compact"
      variant="outlined"
      divided
      @update:model-value="setRole"
    >
      <VBtn value="all" :size="size">Any role</VBtn>
      <VBtn value="combat" :size="size" prepend-icon="mdi-sword-cross">Combat</VBtn>
      <VBtn value="nonCombat" :size="size" prepend-icon="mdi-music-note">Non-combat</VBtn>
    </VBtnToggle>
  </div>
</template>

<style scoped>
.lens-chips {
  max-width: 100%;
}

/* Set apart from the mood chips: they filter, the moods rank. */
.energy-bands {
  padding-left: 8px;
  border-left: 1px solid rgba(var(--v-theme-on-surface), 0.2);
}
</style>
