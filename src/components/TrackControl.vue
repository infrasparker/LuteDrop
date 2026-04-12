<script setup lang="ts">
import { computed } from 'vue';
import type { Track } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useTrackUpdates } from '../composables/useTrackUpdates';
import TrackSettings from './TrackSettings.vue';
import TrackClipList from './TrackClipList.vue';

const props = defineProps<{
  track: Track;
  compId: string;
}>();

const store = useAppStore();
const audio = useAudio();
const { updateField } = useTrackUpdates(() => props.track.id, () => props.compId);

const playbackState = computed(() => store.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);
const isSeamless = computed(() => props.track.mode.type === 'seamless');

function playTrack(): void {
  audio.playTrack(props.track.id);
}

function pauseTrack(): void {
  audio.pauseTrack(props.track.id);
}

function stopTrack(): void {
  audio.stopTrack(props.track.id);
}

async function setVolume(v: number): Promise<void> {
  audio.setTrackVolume(props.track.id, v);
  await updateField({ volume: v });
}

async function deleteTrack(): Promise<void> {
  await audio.stopTrack(props.track.id);
  await store.command('delete_track', { compId: props.compId, trackId: props.track.id });
}
</script>

<template>
  <div>
    <VExpansionPanels>
      <VExpansionPanel color="surface-light" rounded="0" elevation="0">
        <template #title>
          <div class="d-flex align-center ga-2" style="width: 100%;">
            <VBtn
              :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
              :color="isPlaying ? 'secondary' : 'primary'"
              variant="tonal"
              size="small"
              rounded="circle"
              @click.stop="isPlaying ? pauseTrack() : playTrack()"
            />
            <VBtn
              icon="mdi-stop"
              variant="tonal"
              size="small"
              rounded="circle"
              @click.stop="stopTrack"
            />

            <VTextField
              :model-value="track.name"
              @update:model-value="(v: string) => updateField({ name: v })"
              @click.stop
              @keydown.stop
              variant="underlined"
              density="compact"
              hide-details
              class="text-subtitle-2 font-weight-bold"
              style="flex: 0 1 200px; min-width: 80px;"
            />

            <span class="text-caption text-medium-emphasis" style="white-space: nowrap;">
              {{ track.clips.length }} clip{{ track.clips.length !== 1 ? 's' : '' }}
              &middot; {{ track.mode.type }}
            </span>

            <VSlider
              :model-value="track.volume"
              @update:model-value="setVolume"
              @click.stop
              @mousedown.stop
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
          <div class="pt-2">
          <TrackSettings :track="track" :comp-id="compId" />
          <TrackClipList :track="track" :comp-id="compId" :is-seamless="isSeamless" />

          <div class="d-flex justify-end mt-3">
            <VBtn
              icon="mdi-delete"
              variant="tonal"
              color="error"
              @click="deleteTrack"
            />
          </div>
          </div>
        </template>
      </VExpansionPanel>
    </VExpansionPanels>
  </div>
</template>

<style>
/* Global styles for SortableJS (not scoped — SortableJS adds classes directly to DOM) */
.sortable-ghost {
  visibility: hidden;
}

.sortable-fallback {
  opacity: 0.9;
  background: #1E1E24;
  border-radius: 4px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
}

/* Force grabbing cursor on everything while dragging */
body.sortable-dragging,
body.sortable-dragging * {
  cursor: grabbing !important;
}
</style>
