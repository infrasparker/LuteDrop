<script setup lang="ts">
import { computed } from 'vue';
import type { Clip } from '@generated/index';
import { useRemote } from '../composables/useRemote';
import { useLocalVolume } from '../../shared/useLocalVolume';

const props = defineProps<{
  clip: Clip;
  trackId: string;
}>();

const remote = useRemote();

const playbackState = computed(() => remote.playbackStates.get(props.trackId));
const isCurrentClip = computed(() => playbackState.value?.currentClipId === props.clip.id);
const isPlaying = computed(() => isCurrentClip.value && playbackState.value?.isPlaying);
const isMissing = computed(() => remote.isEntryMissing(props.clip.libraryEntryId));

const { local: localVolume, set: setVolume } = useLocalVolume(() => props.clip.volume, (v) => remote.setClipVolume(props.clip.id, v));
</script>

<template>
  <div
    class="clip-row mixer-surface d-flex align-center ga-2 py-2 px-1"
    :class="{ 'mixer-active': isCurrentClip, 'mixer-missing': isMissing }"
  >
    <VBtn
      v-if="isMissing"
      icon="mdi-alert"
      variant="text"
      size="x-small"
      color="warning"
      title="File not found"
    />
    <VBtn
      v-else
      :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
      variant="text"
      size="x-small"
      :color="isCurrentClip ? 'primary' : undefined"
      @click="isPlaying ? remote.pauseTrack(trackId) : remote.playTrackClip(trackId, clip.id)"
    />
    <span
      class="text-body-2 flex-grow-1 text-truncate"

      :style="{ opacity: isMissing ? 0.5 : 1 }"
    >{{ clip.name }}</span>
    <VBtn
      icon="mdi-volume-high"
      variant="text"
      size="x-small"
      class="d-none d-md-flex"
    />
    <VSlider
      :model-value="localVolume"
      @update:model-value="setVolume"
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
.clip-row { margin-top: 6px; }
</style>
