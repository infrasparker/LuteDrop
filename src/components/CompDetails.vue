<script setup lang="ts">
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import type { AppState } from '@generated/index';
import TrackControl from './TrackControl.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
}>();

const store = useAppStore();
const audio = useAudio();

async function deleteComp(): Promise<void> {
  await audio.stopComp(props.comp.id);
  await store.command('delete_comp', { groupId: props.groupId, compId: props.comp.id });
}

async function importTracksToComp(): Promise<void> {
  const paths = await open({
    multiple: true,
    filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'] }],
  });
  if (!paths) return;
  const pathList = Array.isArray(paths) ? paths : [paths];
  for (const filePath of pathList) {
    const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';
    // Add track, get back state to find the new track's ID
    const stateAfterTrack = await invoke<AppState>('add_track', { compId: props.comp.id, name: fileName });
    store.hydrate(stateAfterTrack);
    // Find the newly added track (it's the last one in the comp)
    const comp = store.findComp(props.comp.id);
    if (comp && comp.tracks.length > 0) {
      const newTrack = comp.tracks[comp.tracks.length - 1];
      await store.command('add_clip', { trackId: newTrack.id, name: fileName, filePath });
    }
  }
}
</script>

<template>
  <div>
    <div class="d-flex align-center mb-3">
      <span class="text-h6 font-weight-bold">{{ comp.name }}</span>
      <VSpacer />
      <VBtn
        prepend-icon="mdi-plus"
        variant="tonal"
        size="small"
        class="mr-2"
        @click="importTracksToComp"
      >
        Add Track
      </VBtn>
      <VBtn
        prepend-icon="mdi-delete"
        variant="tonal"
        color="error"
        size="small"
        @click="deleteComp"
      >
        Delete
      </VBtn>
    </div>

    <TrackControl
      v-for="track in comp.tracks"
      :key="track.id"
      :track="track"
      :comp-id="comp.id"
    />

    <div v-if="comp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
      No tracks yet. Click "Add Track" to import audio files.
    </div>
  </div>
</template>
