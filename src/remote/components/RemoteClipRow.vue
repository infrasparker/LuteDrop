<script setup lang="ts">
import type { Clip } from '@generated/index';
import { useRemote } from '../composables/useRemote';

const props = defineProps<{
  clip: Clip;
}>();

const remote = useRemote();
</script>

<template>
  <div class="d-flex align-center ga-2 py-2 px-1" style="border-bottom: 1px solid rgba(232, 228, 220, 0.05);">
    <span class="text-body-2 flex-grow-1 text-truncate">{{ clip.name }}</span>
    <VIcon size="small" class="text-medium-emphasis">mdi-volume-high</VIcon>
    <VSlider
      :model-value="clip.volume"
      @update:model-value="(v: number) => remote.setClipVolume(clip.id, v)"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      style="width: 120px; flex: 0 0 120px;"
    />
  </div>
</template>
