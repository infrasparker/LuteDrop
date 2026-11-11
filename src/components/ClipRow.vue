<script setup lang="ts">
import { computed, ref } from 'vue';
import ClipRemovalDialog from './ClipRemovalDialog.vue';
import ReorderGrip from './ReorderGrip.vue';
import type { Clip } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { open } from '@tauri-apps/plugin-dialog';
import { AUDIO_EXTENSIONS } from '../constants';
import { useLocalVolume } from '../shared/useLocalVolume';

const props = defineProps<{
  clip: Clip;
  trackId: string;
}>();

const store = useAppStore();
const audio = useAudio();

const playbackState = computed(() => store.playbackStates.get(props.trackId));
const isCurrentClip = computed(() => playbackState.value?.currentClipId === props.clip.id);
const isPlaying = computed(() => isCurrentClip.value && playbackState.value?.isPlaying);

// Local ref for smooth slider — audio engine persists to DB directly
const { local: localVolume, set: setVolume } = useLocalVolume(() => props.clip.volume, (v) => audio.setClipVolume(props.clip.id, v));

function playClip(): void {
  audio.playTrackClip(props.trackId, props.clip.id);
}

const showRemove = ref(false);

// A press that starts on a control (e.g. a slider drag released elsewhere in
// the row) makes the browser fire `click` on the row; ignore that click.
let pressStartedOnControl = false;

function onRowPointerDown(e: PointerEvent): void {
  pressStartedOnControl = !!(e.target as Element).closest('.v-slider, button, .v-btn, .drag-handle, input');
}

function onRowClick(): void {
  if (pressStartedOnControl) {
    pressStartedOnControl = false;
    return;
  }
  editLibraryEntry();
}

function editLibraryEntry(): void {
  store.editEntryId = props.clip.libraryEntryId;
}

const isMissing = computed(() => store.isEntryMissing(props.clip.libraryEntryId));

async function relocateMissing(): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
  });
  if (!path || Array.isArray(path)) return;
  await store.command('relocate_library_entry', {
    entryId: props.clip.libraryEntryId,
    newFilePath: path,
  });
}

const isMuted = computed(() => store.isClipMuted(props.clip.id));

function toggleMute(): void {
  audio.toggleClipMute(props.clip.id);
}
</script>

<template>
  <div
    class="clip-row mixer-surface d-flex align-center ga-2 py-2 px-1"
    :class="{ 'mixer-active': isCurrentClip, 'mixer-missing': isMissing }"
    @pointerdown="onRowPointerDown"
    @click="onRowClick"
  >
    <ReorderGrip class="drag-handle" :label="`Drag clip ${clip.name} to reorder`" />
    <VBtn
      v-if="isMissing"
      icon="mdi-alert"
      variant="text"
      size="x-small"
      color="warning"
      title="File not found — click to relocate"
      @click.stop="relocateMissing"
    />
    <VBtn
      v-else
      :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
      :aria-label="`${isPlaying ? 'Pause' : 'Play'} ${clip.name}`"
      variant="text"
      size="x-small"
      :color="isCurrentClip ? 'primary' : undefined"
      @click.stop="isPlaying ? audio.pauseTrack(trackId) : playClip()"
    />
    <span
      class="text-body-2 flex-grow-1 text-truncate"
      :style="{ opacity: isMuted || isMissing ? 0.5 : 1 }"
    >{{ clip.name }}</span>
    <VBtn
      :icon="isMuted ? 'mdi-volume-off' : 'mdi-volume-high'"
      :color="isMuted ? 'error' : undefined"
      variant="text"
      size="x-small"
      :title="isMuted ? 'Unmute clip' : 'Mute clip'"
      @click.stop="toggleMute"
    />
    <VSlider
      :model-value="localVolume"
      @update:model-value="setVolume"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      @click.stop
      style="width: 120px; flex: 0 0 120px;"
      :style="{ opacity: isMuted ? 0.4 : 1 }"
    />
    <VBtn
      icon="mdi-pencil"
      variant="text"
      size="x-small"
      title="Edit in Library"
      @click.stop="editLibraryEntry"
    />
    <ClipRemovalDialog v-model="showRemove" :track-id="trackId" :clip="clip" />
    <VBtn icon="mdi-delete" variant="text" size="x-small" color="error" :aria-label="`Remove ${clip.name}`" @click.stop="showRemove = true" />
  </div>
</template>

<style scoped>
.clip-row {
  cursor: pointer;
}
</style>
