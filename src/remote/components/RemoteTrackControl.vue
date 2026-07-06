<script setup lang="ts">
import { computed } from 'vue';
import type { Track } from '@generated/index';
import { useRemote } from '../composables/useRemote';
import { useLocalVolume } from '../../shared/useLocalVolume';
import RemoteClipRow from './RemoteClipRow.vue';

const props = defineProps<{
  track: Track;
}>();

const remote = useRemote();
const playbackState = computed(() => remote.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);
const isActive = computed(() => isPlaying.value || (playbackState.value?.isPaused ?? false));

const { local: localVolume, set: setVolume } = useLocalVolume(() => props.track.volume, (v) => remote.setTrackVolume(props.track.id, v));

const currentClipName = computed(() => {
  const clipId = playbackState.value?.currentClipId;
  if (!clipId) return null;
  return props.track.clips.find((c) => c.id === clipId)?.name ?? null;
});
</script>

<template>
  <div :class="{ 'track-wrapper': true, 'track-active': isActive }">
    <VExpansionPanels>
      <VExpansionPanel
        color="surface-light"
        rounded="0"
        elevation="0"
      >
        <template #title>
          <div class="d-flex align-center ga-2" style="width: 100%;">
            <VBtn
              :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
              variant="tonal"
              size="small"
              rounded="circle"
              @click.stop="isPlaying ? remote.pauseTrack(track.id) : remote.playTrack(track.id)"
            />
            <VBtn
              icon="mdi-stop"
              variant="tonal"
              size="small"
              rounded="circle"
              @click.stop="remote.stopTrack(track.id)"
            />

            <span class="text-subtitle-2 font-weight-bold text-truncate">{{ track.name }}</span>

            <VSpacer />

            <span v-if="currentClipName" class="text-caption text-primary d-none d-md-inline" style="white-space: nowrap;">
              {{ currentClipName }}
            </span>
            <span class="text-caption text-medium-emphasis d-none d-md-inline" style="white-space: nowrap;">
              {{ track.clips.length }} clip{{ track.clips.length !== 1 ? 's' : '' }}
            </span>

            <VSlider
              :model-value="localVolume"
              @update:model-value="setVolume"
              @click.stop
              @mousedown.stop
              @touchstart.stop
              :min="0"
              :max="1"
              :step="0.01"
              hide-details
              density="compact"
              class="mr-4"
              style="flex: 0 0 120px;"
            />
          </div>
        </template>

        <template #text>
          <div v-if="track.clips.length > 0" class="pt-2">
            <RemoteClipRow
              v-for="clip in track.clips"
              :key="clip.id"
              :clip="clip"
              :track-id="track.id"
            />
          </div>
          <div v-else class="text-medium-emphasis text-center pa-4">
            No clips.
          </div>
        </template>
      </VExpansionPanel>
    </VExpansionPanels>
  </div>
</template>

<style scoped>
.track-wrapper {
  border-left: 3px solid transparent;
}
.track-active {
  border-left: 3px solid #F0A830;
}
.track-active :deep(.v-expansion-panel-title) {
  background: rgba(240, 168, 48, 0.08);
}
</style>
