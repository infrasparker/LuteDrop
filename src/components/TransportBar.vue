<script setup lang="ts">
import { ref, computed } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';

const store = useAppStore();
const audio = useAudio();

const paused = ref(false);

const playingCount = computed(() => {
  let count = 0;
  for (const state of store.playbackStates.values()) {
    if (state.isPlaying) count++;
  }
  return count;
});

function setMasterVolume(v: number): void {
  audio.masterVolume.value = v;
  audio.setMasterVolume(v);
}

function stopAll(): void {
  paused.value = false;
  audio.stopAll();
}

function togglePause(): void {
  if (paused.value) {
    audio.resumeAll();
    paused.value = false;
  } else {
    audio.pauseAll();
    paused.value = true;
  }
}
</script>

<template>
  <VFooter app color="surface" elevation="4" class="pa-2 d-flex align-center gap-2">
    <VBtn
      :icon="paused ? 'mdi-play' : 'mdi-pause'"
      variant="tonal"
      size="small"
      @click="togglePause"
    />
    <VBtn
      icon="mdi-stop"
      variant="tonal"
      size="small"
      @click="stopAll"
    />

    <span class="text-caption text-medium-emphasis ml-2">
      {{ playingCount }} {{ playingCount === 1 ? 'track' : 'tracks' }} playing
    </span>

    <VSpacer />

    <VIcon>mdi-volume-high</VIcon>
    <VSlider
      :model-value="audio.masterVolume.value"
      @update:model-value="setMasterVolume"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      style="width: 120px"
    />
  </VFooter>
</template>
