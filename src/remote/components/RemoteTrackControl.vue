<script setup lang="ts">
import { computed } from 'vue';
import type { Track } from '@generated/index';
import { useRemote } from '../composables/useRemote';
import RemoteClipRow from './RemoteClipRow.vue';

const props = defineProps<{
  track: Track;
}>();

const remote = useRemote();
const playbackState = computed(() => remote.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);
</script>

<template>
  <VCard class="mb-2" color="surface-light">
    <div class="d-flex align-center ga-2 pa-2">
      <VBtn
        :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
        :color="isPlaying ? 'secondary' : 'primary'"
        variant="tonal"
        size="small"
        rounded="circle"
        @click="isPlaying ? remote.pauseTrack(track.id) : remote.playTrack(track.id)"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="small"
        rounded="circle"
        @click="remote.stopTrack(track.id)"
      />

      <span class="text-subtitle-2 font-weight-bold flex-grow-1">{{ track.name }}</span>

      <span class="text-caption text-medium-emphasis">
        {{ track.clips.length }} clip{{ track.clips.length !== 1 ? 's' : '' }}
        &middot; {{ track.mode.type }}
      </span>

      <VSlider
        :model-value="track.volume"
        @update:model-value="(v: number) => remote.setTrackVolume(track.id, v)"
        :min="0"
        :max="1"
        :step="0.01"
        hide-details
        density="compact"
        style="max-width: 120px"
      />
    </div>

    <!-- Clip volumes -->
    <div v-if="track.clips.length > 0" class="px-3 pb-2">
      <RemoteClipRow
        v-for="clip in track.clips"
        :key="clip.id"
        :clip="clip"
      />
    </div>
  </VCard>
</template>
