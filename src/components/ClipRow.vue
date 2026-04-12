<script setup lang="ts">
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

async function updateVolume(volume: number): Promise<void> {
  const track = store.findTrack(props.trackId);
  if (!track) return;
  const updatedClips = track.clips.map((c) =>
    c.id === props.clip.id ? { ...c, volume } : c
  );
  await store.command('update_track', {
    compId: props.compId,
    trackId: props.trackId,
    updates: { clips: updatedClips },
  });
  audio.setClipVolume(props.clip.id, volume);
}

async function remove(): Promise<void> {
  await store.command('delete_clip', { trackId: props.trackId, clipId: props.clip.id });
}
</script>

<template>
  <div class="d-flex align-center ga-2 py-2 px-1" style="border-bottom: 1px solid rgba(232, 228, 220, 0.05);">
    <VIcon size="small" class="text-medium-emphasis drag-handle" style="cursor: grab;">mdi-drag</VIcon>
    <span class="text-body-2 flex-grow-1 text-truncate">{{ clip.name }}</span>
    <VIcon size="small" class="text-medium-emphasis">mdi-volume-high</VIcon>
    <VSlider
      :model-value="clip.volume"
      @update:model-value="updateVolume"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      style="width: 120px; flex: 0 0 120px;"
    />
    <VBtn icon="mdi-delete" variant="text" size="x-small" color="error" @click="remove" />
  </div>
</template>
