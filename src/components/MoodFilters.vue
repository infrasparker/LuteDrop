<script setup lang="ts">
import { computed, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import FilterChip from './FilterChip.vue';
import {
  chipState,
  cycleChip,
  lensOn,
  moodsOnly,
  sortForLens,
  type MoodFilters,
  type SortDir,
  type SortKey,
} from '../utils/moods';

/// The filter bar shared by the Library and Scry: a lens chip per mood of the
/// vocabulary and the Role toggle. While the vocabulary needs recalibrating,
/// a marker after the lens chips says so and opens the Moods settings.
///
/// A lens chip cycles off → include → exclude. Included moods rank the list
/// by the weakest of their scores; an excluded mood keeps only tracks scoring
/// below 0.5 in it. Including the first lens sorts by the lens score
/// (highest first); dropping the last one while sorted by it goes back to
/// Name. Sorting itself is done from the list's column headers. Lenses and
/// role apply to music only, so callers hide the bar for ambiance.
///
/// Energy has no lens chip: it is a sort key (its column header), not a mood
/// to rank by.
const filters = defineModel<MoodFilters>({ required: true });
const sort = defineModel<SortKey>('sort', { required: true });
const dir = defineModel<SortDir>('dir', { required: true });
defineProps<{ size: 'small' | 'x-small' }>();

const store = useAppStore();
const moods = computed(() => moodsOnly());

// A lens that is not a mood of the vocabulary (removed, or Energy) is dropped.
watch(moods, (list) => {
  const known = (id: string): boolean => list.some((m) => m.id === id);
  const { include, exclude } = filters.value.lenses;
  if (include.every(known) && exclude.every(known)) return;
  filters.value = { ...filters.value, lenses: { include: include.filter(known), exclude: exclude.filter(known) } };
});

watch(
  () => lensOn(filters.value),
  (isOn, wasOn) => {
    const next = sortForLens({ key: sort.value, dir: dir.value }, wasOn, isOn);
    sort.value = next.key;
    dir.value = next.dir;
  },
);

function cycleLens(id: string): void {
  filters.value = { ...filters.value, lenses: cycleChip(filters.value.lenses, id) };
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
        :state="chipState(filters.lenses, m.id)"
        :size="size"
        off-variant="outlined"
        @cycle="cycleLens(m.id)"
      />
    </div>
    <VBtn
      v-if="store.moodsNeedRecalibration"
      :size="size"
      variant="text"
      color="warning"
      prepend-icon="mdi-alert-circle-outline"
      class="text-none"
      @click="store.moodSettingsOpen = true"
    >
      Needs recalibration
      <VTooltip activator="parent" location="top" open-delay="300">
        Some mood prompts changed since the last calibration. Those moods are not scored until you recalibrate.
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
</style>
