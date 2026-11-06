<script setup lang="ts">
import { computed, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import type { MoodId } from '@generated/index';
import AnyAllToggle from './AnyAllToggle.vue';
import {
  DEFAULT_DIR,
  moodsOnly,
  SORT_LABELS,
  type MatchMode,
  type MoodFilters,
  type SortDir,
  type SortKey,
} from '../utils/moods';

/// The filter bar shared by the Library and Scry: a lens button per mood of
/// the vocabulary (any number at once), the any / all switch once
/// two or more are on, the Role toggle, and the Sort menu with its direction.
/// While the vocabulary needs recalibrating, a marker after the lens buttons
/// says so and opens the Moods settings.
///
/// A lens ranks the list by that mood's score; picking the first lens
/// switches the sort to "Lens score", and clearing the last one goes back to
/// the first of `sortKeys`. Lenses and role apply to music only, so callers
/// hide the bar for ambiance.
///
/// Energy has no lens button: it is a sort key (the Sort menu and the
/// Library's Energy column), not a mood to rank by.
const filters = defineModel<MoodFilters>({ required: true });
const sort = defineModel<SortKey>('sort', { required: true });
const dir = defineModel<SortDir>('dir', { required: true });
const props = defineProps<{
  size: 'small' | 'x-small';
  /// Sort options offered without a lens, in menu order ("Lens score" is
  /// added in front while a lens is on).
  sortKeys: SortKey[];
}>();

const store = useAppStore();
const moods = computed(() => moodsOnly());

// A lens that is not a mood of the vocabulary (removed, or Energy) is dropped.
watch(moods, (list) => {
  const kept = filters.value.lenses.filter((id) => list.some((m) => m.id === id));
  if (kept.length !== filters.value.lenses.length) setLenses(kept);
});

const menuKeys = computed<SortKey[]>(() =>
  filters.value.lenses.length > 0 ? ['lens', ...props.sortKeys.filter((k) => k !== 'lens')] : props.sortKeys,
);

function pickSort(key: SortKey): void {
  if (sort.value === key) return;
  sort.value = key;
  dir.value = DEFAULT_DIR[key];
}

watch(
  () => filters.value.lenses.length > 0,
  (on) => {
    if (on) pickSort('lens');
    else if (sort.value === 'lens') pickSort(props.sortKeys[0]);
  },
);

function setLenses(lenses: MoodId[]): void {
  // Back to "any" when fewer than two are left, so the switch never hides a
  // non-default state.
  filters.value = { ...filters.value, lenses, mode: lenses.length < 2 ? 'any' : filters.value.mode };
}

function setMode(mode: MatchMode): void {
  filters.value = { ...filters.value, mode };
}

function setRole(role: MoodFilters['role']): void {
  filters.value = { ...filters.value, role };
}

function flipDir(): void {
  dir.value = dir.value === 'asc' ? 'desc' : 'asc';
}

const dirHint = computed(() => {
  const asc = dir.value === 'asc';
  if (sort.value === 'name') return asc ? 'A to Z' : 'Z to A';
  if (sort.value === 'match') return asc ? 'Worst match first' : 'Best match first';
  return asc ? 'Lowest first' : 'Highest first';
});
</script>

<template>
  <div class="d-flex flex-wrap align-center ga-2">
    <VBtnToggle
      :model-value="filters.lenses"
      multiple
      density="compact"
      variant="outlined"
      divided
      color="primary"
      class="lens-buttons"
      @update:model-value="setLenses"
    >
      <VBtn v-for="m in moods" :key="m.id" :value="m.id" :size="size">
        {{ m.name }}
      </VBtn>
    </VBtnToggle>
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
    <AnyAllToggle
      v-if="filters.lenses.length >= 2"
      :model-value="filters.mode"
      :size="size"
      kind="lens"
      @update:model-value="setMode"
    />
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
    <div class="d-flex align-center">
      <VMenu location="bottom start">
        <template #activator="{ props: activator }">
          <VBtn v-bind="activator" :size="size" variant="outlined" prepend-icon="mdi-sort" append-icon="mdi-menu-down">
            Sort: {{ SORT_LABELS[sort] }}
          </VBtn>
        </template>
        <VList density="compact">
          <VListItem
            v-for="key in menuKeys"
            :key="key"
            :title="SORT_LABELS[key]"
            :active="key === sort"
            color="primary"
            @click="pickSort(key)"
          />
        </VList>
      </VMenu>
      <VBtn
        icon
        :size="size"
        variant="text"
        density="comfortable"
        :aria-label="`Sort direction: ${dirHint}`"
        @click="flipDir"
      >
        <VIcon size="small">{{ dir === 'asc' ? 'mdi-arrow-up' : 'mdi-arrow-down' }}</VIcon>
        <VTooltip activator="parent" location="top" open-delay="300">{{ dirHint }}</VTooltip>
      </VBtn>
    </div>
  </div>
</template>

<style scoped>
/* The lens buttons on one line when there is room; in a narrow pane (Scry) they
   wrap onto a second line rather than scroll. */
.lens-buttons {
  max-width: 100%;
  flex-wrap: wrap;
  height: auto;
}
</style>
