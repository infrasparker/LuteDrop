<script setup lang="ts">
import type { SoundFilters } from '../utils/mood';

/// Bright/Dark, Light/Heavy and Pace filter toggles, shared by the Library
/// and Scry results. Music only, so callers hide it for ambiance.
const filters = defineModel<SoundFilters>({ required: true });
defineProps<{ size: 'small' | 'x-small' }>();

function update<K extends keyof SoundFilters>(key: K, value: SoundFilters[K]): void {
  filters.value = { ...filters.value, [key]: value };
}
</script>

<template>
  <div class="d-flex flex-wrap align-center ga-3">
    <VBtnToggle
      :model-value="filters.bright"
      mandatory
      density="compact"
      variant="outlined"
      divided
      @update:model-value="(v) => update('bright', v)"
    >
      <VBtn value="all" :size="size">Bright ↔ Dark: any</VBtn>
      <VBtn value="low" :size="size">Dark</VBtn>
      <VBtn value="mid" :size="size">Neutral</VBtn>
      <VBtn value="high" :size="size">Bright</VBtn>
    </VBtnToggle>
    <VBtnToggle
      :model-value="filters.light"
      mandatory
      density="compact"
      variant="outlined"
      divided
      @update:model-value="(v) => update('light', v)"
    >
      <VBtn value="all" :size="size">Light ↔ Heavy: any</VBtn>
      <VBtn value="low" :size="size">Heavy</VBtn>
      <VBtn value="mid" :size="size">Neutral</VBtn>
      <VBtn value="high" :size="size">Light</VBtn>
    </VBtnToggle>
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
