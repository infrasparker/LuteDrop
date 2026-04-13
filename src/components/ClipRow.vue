<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import type { Clip } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';

const props = defineProps<{
  clip: Clip;
  trackId: string;
  compId: string;
}>();

const store = useAppStore();
const audio = useAudio();
const router = useRouter();

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
  router.push('/library');
}
</script>

<template>
  <div
    class="clip-row d-flex align-center ga-2 py-2 px-1"
    :class="{ 'clip-active': isCurrentClip }"
  >
    <VIcon size="small" class="text-medium-emphasis drag-handle" style="cursor: grab;">mdi-drag</VIcon>
    <VBtn
      :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
      variant="text"
      size="x-small"
      :color="isCurrentClip ? 'primary' : undefined"
      @click="isPlaying ? audio.pauseTrack(trackId) : playClip()"
    />
    <span
      class="text-body-2 flex-grow-1 text-truncate"
      :class="{ 'text-primary': isCurrentClip }"
    >{{ clip.name }}</span>
    <VIcon size="small" class="text-medium-emphasis">mdi-volume-high</VIcon>
    <VSlider
      :model-value="localVolume"
      @update:model-value="setVolume"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      style="width: 120px; flex: 0 0 120px;"
    />
    <VBtn
      icon="mdi-book-edit-outline"
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
</style>
