<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import type { Clip } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { open } from '@tauri-apps/plugin-dialog';

const props = defineProps<{
  clip: Clip;
  trackId: string;
  compId: string;
}>();

const store = useAppStore();
const audio = useAudio();

const playbackState = computed(() => store.playbackStates.get(props.trackId));
const isCurrentClip = computed(() => playbackState.value?.currentClipId === props.clip.id);
const isPlaying = computed(() => isCurrentClip.value && playbackState.value?.isPlaying);

// Local ref for smooth slider — audio engine persists to DB directly
const localVolume = ref(props.clip.volume);
watch(() => props.clip.volume, (v) => { localVolume.value = v; });

function playClip(): void {
  audio.playTrackClip(props.trackId, props.clip.id);
}

function setVolume(v: number): void {
  localVolume.value = v;
  audio.setClipVolume(props.clip.id, v);
}

async function remove(): Promise<void> {
  await store.command('delete_clip', { trackId: props.trackId, clipId: props.clip.id });
}

function editLibraryEntry(): void {
  store.editEntryId = props.clip.libraryEntryId;
}

const isMissing = computed(() => store.isEntryMissing(props.clip.libraryEntryId));

const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];

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
    class="clip-row d-flex align-center ga-2 py-2 px-1"
    :class="{ 'clip-active': isCurrentClip, 'clip-missing': isMissing }"
  >
    <VIcon size="small" class="text-medium-emphasis drag-handle" style="cursor: grab;">mdi-drag</VIcon>
    <VBtn
      :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
      variant="text"
      size="x-small"
      :color="isCurrentClip ? 'primary' : undefined"
      @click="isPlaying ? audio.pauseTrack(trackId) : playClip()"
    />
    <VBtn
      v-if="isMissing"
      icon="mdi-alert"
      variant="text"
      size="x-small"
      color="warning"
      title="File not found — click to relocate"
      @click.stop="relocateMissing"
    />
    <span
      class="text-body-2 flex-grow-1 text-truncate"
      :class="{ 'text-primary': isCurrentClip }"
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
    <VBtn icon="mdi-delete" variant="text" size="x-small" color="error" @click="remove" />
  </div>
</template>

<style scoped>
.clip-row {
  border-bottom: 1px solid rgba(232, 228, 220, 0.05);
  border-left: 3px solid transparent;
}
.clip-active {
  border-left: 3px solid #F0A830;
  background: rgba(240, 168, 48, 0.08);
}
.clip-missing {
  border-left: 3px solid rgb(var(--v-theme-warning));
  background: rgba(var(--v-theme-warning), 0.08);
}
</style>
