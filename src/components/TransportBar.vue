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
    <div class="transport-grid">
      <div class="transport-left">
        <span style="white-space: nowrap;" :class="playingCount > 0 ? 'text-primary' : 'text-medium-emphasis'">
          {{ playingCount === 1 ? '1 clip playing' : `${playingCount} clips playing` }}
        </span>
      </div>

      <div class="transport-center">
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

      <div class="transport-right">
        <VBtn
          :icon="store.masterMuted ? 'mdi-volume-off' : 'mdi-volume-high'"
          :color="store.masterMuted ? 'error' : undefined"
          variant="text"
          size="small"
          :title="store.masterMuted ? 'Unmute master' : 'Mute master'"
          @click="audio.toggleMasterMute()"
        />
        <VSlider
          :model-value="audio.masterVolume.value"
          @update:model-value="setMasterVolume"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="max-width: 120px;"
        />
      </div>
    </div>
  </VFooter>
</template>

<style scoped>
.transport-grid {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  width: 100%;
  max-width: 450px;
  margin: 0 auto;
  gap: 12px;
}

.transport-left {
  justify-self: start;
}

.transport-center {
  display: flex;
  gap: 8px;
}

.transport-right {
  display: flex;
  align-items: center;
  justify-content: end;
  gap: 8px;
}
</style>
