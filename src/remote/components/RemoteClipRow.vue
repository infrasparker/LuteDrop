<script setup lang="ts">
import { computed } from 'vue';
import type { Clip } from '@generated/index';
import { useRemote } from '../composables/useRemote';

const props = defineProps<{
  clip: Clip;
  trackId: string;
}>();

const remote = useRemote();

const playbackState = computed(() => remote.playbackStates.get(props.trackId));
const isCurrentClip = computed(() => playbackState.value?.currentClipId === props.clip.id);
const isPlaying = computed(() => isCurrentClip.value && playbackState.value?.isPlaying);
</script>

<template>
  <div
    class="clip-row d-flex align-center ga-2 py-2 px-1"
    :class="{ 'clip-active': isCurrentClip }"
  >
    <VBtn
      :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
      variant="text"
      size="x-small"
      :color="isCurrentClip ? 'primary' : undefined"
      @click="isPlaying ? remote.pauseTrack(trackId) : remote.playTrackClip(trackId, clip.id)"
    />
    <span
      class="text-body-2 flex-grow-1 text-truncate"
      :class="{ 'text-primary': isCurrentClip }"
    >{{ clip.name }}</span>
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

<style scoped>
.clip-row {
  border-bottom: 1px solid rgba(232, 228, 220, 0.05);
  border-left: 3px solid transparent;
}
.clip-active {
  border-left: 3px solid #F0A830;
  background: rgba(240, 168, 48, 0.08);
}
</style>
