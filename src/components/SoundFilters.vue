<script setup lang="ts">
import { computed, ref } from 'vue';
import type { LibraryEntry } from '@generated/index';
import { countByCell, MOOD_CELLS, type SoundFilters } from '../utils/mood';
import MoodPad, { type PadSize } from './MoodPad.vue';

/// The mood filter, shared by the Library and Scry results. Music only, so
/// callers hide it for ambiance. The Mood button opens the mood pad in filter
/// mode; pick any number of cells (none = any mood). `entries` are the
/// entries the counts on the pad are taken from.
const filters = defineModel<SoundFilters>({ required: true });
const props = defineProps<{ size: 'small' | 'x-small'; entries: LibraryEntry[] }>();

const padSize = ref<PadSize>('mid');
const counts = computed(() => countByCell(props.entries));

function setMoods(moods: string[]): void {
  filters.value = { ...filters.value, moods };
}

const moodLabel = computed(() => {
  const picked = MOOD_CELLS.filter((c) => filters.value.moods.includes(c.key));
  // Dark names are the same on every size, so a name can stand for several cells.
  const names = [...new Set(picked.map((c) => c.name))];
  if (names.length === 0) return 'Mood: any';
  if (names.length <= 2) return `Mood: ${names.join(', ')}`;
  return `Mood: ${picked.length} picked`;
});
</script>

<template>
  <VMenu :close-on-content-click="false" location="bottom start">
    <template #activator="{ props: activator }">
      <VBtn
        v-bind="activator"
        :size="size"
        variant="outlined"
        prepend-icon="mdi-grid"
        :color="filters.moods.length > 0 ? 'primary' : undefined"
      >
        {{ moodLabel }}
      </VBtn>
    </template>
    <VCard class="pa-3" max-width="560">
      <div class="d-flex align-center mb-2">
        <span class="text-subtitle-2">Pick moods</span>
        <span class="text-caption text-medium-emphasis ml-2">tracks in any picked mood</span>
        <VSpacer />
        <VBtn size="x-small" variant="text" :disabled="filters.moods.length === 0" @click="setMoods([])">
          Clear
        </VBtn>
      </div>
      <MoodPad
        v-model:size="padSize"
        mode="filter"
        :selected="filters.moods"
        :counts="counts"
        @update:selected="setMoods"
      />
    </VCard>
  </VMenu>
</template>
