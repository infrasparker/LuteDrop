<script setup lang="ts">
import { computed } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useBroadcast } from '../composables/useBroadcast';
import TransportScrubber from './TransportScrubber.vue';

const store = useAppStore();
const audio = useAudio();
const broadcast = useBroadcast();

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
  <!-- One fixed-height row: now-playing + scrubber | transport | volume.
       Every slot keeps its size whatever is playing, so the footer (and
       --v-layout-bottom, which pages size themselves from) never changes. -->
  <VFooter app :height="56" color="surface" elevation="4" class="px-3 py-0 footer-grid">
    <div class="footer-slot footer-slot-left">
      <TransportScrubber />
    </div>

    <div class="transport-center">
      <VBtn
        :icon="showPlayIcon ? 'mdi-play' : 'mdi-pause'"
        :disabled="!anyPlaying && !anyPaused"
        variant="tonal"
        size="small"
        rounded="circle"
        :title="showPlayIcon ? 'Resume' : 'Pause'"
        @click="togglePause"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="small"
        rounded="circle"
        title="Stop all"
        @click="stopAll"
      />
    </div>

    <div class="footer-slot footer-slot-right">
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
      <VChip v-if="broadcast.status.value?.enabled" size="small" variant="tonal" color="primary" prepend-icon="mdi-broadcast" title="Broadcasting" class="flex-shrink-0">
        {{ broadcast.status.value.listenerCount === 1 ? '1 listening' : `${broadcast.status.value.listenerCount} listening` }}
      </VChip>
      <VBtn
        :icon="store.masterMuted ? 'mdi-volume-off' : 'mdi-volume-high'"
        :color="store.masterMuted ? 'error' : undefined"
        variant="text"
        size="small"
        class="flex-shrink-0"
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
  </VFooter>
</template>

<style scoped>
.footer-grid {
  display: grid;
  /* Equal side columns keep the transport buttons centred. */
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
  align-items: center;
  gap: 24px;
  overflow: hidden;
}

.footer-slot {
  display: flex;
  align-items: center;
  min-width: 0;
}

.footer-slot-right {
  justify-content: flex-end;
  gap: 8px;
}

.transport-center {
  display: flex;
  gap: 8px;
}

.volume-slider {
  flex: 0 1 160px;
  min-width: 80px;
}

.import-indicator {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  font-size: 0.85rem;
  color: rgba(255, 255, 255, 0.72);
  white-space: nowrap;
}

.import-label {
  overflow: hidden;
  text-overflow: ellipsis;
  font-variant-numeric: tabular-nums;
}
</style>
