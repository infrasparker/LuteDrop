<script setup lang="ts">
import { computed } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useBroadcast } from '../composables/useBroadcast';
import TransportScrubber from './TransportScrubber.vue';

const store = useAppStore();
const audio = useAudio();
const broadcast = useBroadcast();

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

const importProgressPercent = computed(() => {
  const p = store.importProgress;
  if (!p || p.total === 0) return 0;
  return Math.round((p.processed / p.total) * 100);
});

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
  <VFooter app color="surface" elevation="4" class="pa-2 footer-grid">
    <div class="footer-slot footer-slot-left">
      <div v-if="store.importProgress" class="import-indicator" :title="`Importing ${store.importProgress.processed} of ${store.importProgress.total}`">
        <VProgressCircular
          :model-value="importProgressPercent"
          size="20"
          width="2"
          color="primary"
        />
        <span class="import-label">
          Importing {{ store.importProgress.processed }}/{{ store.importProgress.total }}
        </span>
      </div>
    </div>

    <div class="transport-stack">
      <TransportScrubber />
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
            class="volume-slider"
          />
        </div>
      </div>
    </div>

    <div class="footer-slot footer-slot-right">
      <VChip v-if="broadcast.status.value?.enabled" size="small" variant="tonal" color="primary" prepend-icon="mdi-broadcast" title="Broadcasting">
        {{ broadcast.status.value.listenerCount === 1 ? '1 listening' : `${broadcast.status.value.listenerCount} listening` }}
      </VChip>
    </div>
  </VFooter>
</template>

<style scoped>
.footer-grid {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 12px;
}

.footer-slot {
  display: flex;
  align-items: center;
  min-width: 0;
}

.footer-slot-right {
  justify-content: flex-end;
}

.transport-stack {
  display: flex;
  flex-direction: column;
  align-items: center;
  width: 100%;
  max-width: 600px;
}

.transport-grid {
  display: grid;
  grid-template-columns: auto auto 1fr;
  align-items: center;
  width: 100%;
  max-width: 600px;
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

.volume-slider {
  min-width: 100px;
  max-width: 240px;
  flex: 1;
}

.import-indicator {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 8px;
  font-size: 0.85rem;
  color: rgba(255, 255, 255, 0.72);
  white-space: nowrap;
}

.import-label {
  font-variant-numeric: tabular-nums;
}
</style>
