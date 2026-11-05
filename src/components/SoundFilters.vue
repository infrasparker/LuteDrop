<script setup lang="ts">
import { computed } from 'vue';
import { MOOD_SQUARES, type SoundFilters } from '../utils/mood';
import MoodGrid from './MoodGrid.vue';

/// Mood-map and Pace filters, shared by the Library and Scry results. Music
/// only, so callers hide it for ambiance. The Mood button opens the 3×3 map;
/// pick any number of squares (none = any mood).
const filters = defineModel<SoundFilters>({ required: true });
defineProps<{ size: 'small' | 'x-small' }>();

function update<K extends keyof SoundFilters>(key: K, value: SoundFilters[K]): void {
  filters.value = { ...filters.value, [key]: value };
}

function toggleMood(key: string): void {
  const moods = filters.value.moods;
  update('moods', moods.includes(key) ? moods.filter((k) => k !== key) : [...moods, key]);
}

const moodLabel = computed(() => {
  const names = MOOD_SQUARES.filter((s) => filters.value.moods.includes(s.key)).map((s) => s.name);
  if (names.length === 0) return 'Mood: any';
  if (names.length <= 2) return `Mood: ${names.join(', ')}`;
  return `Mood: ${names.length} picked`;
});
</script>

<template>
  <div class="d-flex flex-wrap align-center ga-3">
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
          <span class="text-caption text-medium-emphasis ml-2">tracks in any picked square</span>
          <VSpacer />
          <VBtn size="x-small" variant="text" :disabled="filters.moods.length === 0" @click="update('moods', [])">
            Clear
          </VBtn>
        </div>
        <MoodGrid selectable :selected="filters.moods" @toggle="toggleMood" />
      </VCard>
    </VMenu>
    <VBtnToggle
      :model-value="filters.pace"
      mandatory
      density="compact"
      variant="outlined"
      divided
      @update:model-value="(v) => update('pace', v)"
    >
      <VBtn value="all" :size="size">Any pace</VBtn>
      <VBtn value="still" :size="size" prepend-icon="mdi-pause">Still</VBtn>
      <VBtn value="moving" :size="size" prepend-icon="mdi-play">Moving</VBtn>
      <VBtn value="unset" :size="size">Unset</VBtn>
    </VBtnToggle>
  </div>
</template>
