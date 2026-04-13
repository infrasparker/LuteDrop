<script setup lang="ts">
import { computed } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';

const store = useAppStore();
const audio = useAudio();

const playingCount = computed(() => {
  let count = 0;
  for (const state of store.playbackStates.values()) {
    if (state.isPlaying) count++;
  }
  return count;
});

const anyPlaying = computed(() => {
  for (const state of store.playbackStates.values()) {
    if (state.isPlaying) return true;
  }
  return false;
});

const anyPaused = computed(() => {
  for (const state of store.playbackStates.values()) {
    if (state.isPaused) return true;
  }
  return false;
});

const showPlayIcon = computed(() => !anyPlaying.value && anyPaused.value);

function setMasterVolume(v: number): void {
  audio.masterVolume.value = v;
  audio.setMasterVolume(v);
}

function stopAll(): void {
  audio.stopAll();
}

function togglePause(): void {
  if (anyPlaying.value) {
    audio.pauseAll();
  } else if (anyPaused.value) {
    audio.resumeAll();
  }
}
</script>

<template>
  <VFooter app color="surface" elevation="4" class="pa-2">
    <div class="d-flex align-center w-100" style="position: relative;">
      <!-- Playing count on the left — absolutely positioned so it doesn't shift the buttons -->
      <span v-if="playingCount > 0" class="text-caption text-primary" style="position: absolute; left: 0;">
        {{ playingCount }} playing
      </span>

      <!-- Centered transport controls -->
      <div class="d-flex align-center ga-2 mx-auto">
        <VBtn
          :icon="showPlayIcon ? 'mdi-play' : 'mdi-pause'"
          :disabled="!anyPlaying && !anyPaused"
          variant="tonal"
          size="small"
          rounded="circle"
          @click="togglePause"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          rounded="circle"
          @click="stopAll"
        />
      </div>

      <!-- Master volume on the right — absolutely positioned -->
      <div class="d-flex align-center ga-2" style="position: absolute; right: 0;">
        <VIcon size="small" class="text-medium-emphasis">mdi-volume-high</VIcon>
        <VSlider
          :model-value="audio.masterVolume.value"
          @update:model-value="setMasterVolume"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 150px;"
        />
      </div>
    </div>
  </VFooter>
</template>
