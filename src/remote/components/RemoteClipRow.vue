<script setup lang="ts">
import type { Clip } from '@generated/index';
import { useRemote } from '../composables/useRemote';

const props = defineProps<{
  clip: Clip;
}>();

const remote = useRemote();
</script>

<template>
  <div class="d-flex align-center ga-2 py-1">
    <span class="text-body-2 flex-grow-1 text-truncate" style="max-width: 200px;">
      {{ clip.name }}
    </span>
    <VSlider
      :model-value="clip.volume"
      @update:model-value="(v: number) => remote.setClipVolume(clip.id, v)"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      style="max-width: 100px"
      prepend-icon="mdi-volume-high"
    />
  </div>
</template>
