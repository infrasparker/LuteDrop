<script setup lang="ts">
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import type { AppState } from '@generated/index';
import TrackControl from './TrackControl.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
}>();

const store = useAppStore();

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
    <div class="d-flex align-center mb-4">
      <VSpacer />
      <VBtn
        prepend-icon="mdi-plus"
        variant="tonal"
        rounded="pill"
        @click="importTracksToComp"
      >
        Add Track
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
