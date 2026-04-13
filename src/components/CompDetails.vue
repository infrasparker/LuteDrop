<script setup lang="ts">
import { ref } from 'vue';
import type { Comp } from '@generated/index';
import { useTrackImport } from '../composables/useTrackImport';
import TrackControl from './TrackControl.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
}>();

const { importTracks } = useTrackImport();

// Library browser dialog state (will be implemented in Task 12)
const showLibraryBrowser = ref(false);
const libraryBrowserTrackId = ref('');

function openLibraryBrowser(trackId: string): void {
  libraryBrowserTrackId.value = trackId;
  showLibraryBrowser.value = true;
}
</script>

<template>
  <div>
    <div class="d-flex align-center mb-4">
      <VSpacer />
      <VBtn
        prepend-icon="mdi-plus"
        variant="tonal"
        rounded="xl"
        @click="importTracks(comp.id)"
      >
        Add Track
      </VBtn>
    </div>

    <TrackControl
      v-for="track in comp.tracks"
      :key="track.id"
      :track="track"
      :comp-id="comp.id"
      @open-library-browser="openLibraryBrowser"
    />

    <div v-if="comp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
      No tracks yet. Click "Add Track" to import audio files.
    </div>

  </div>
</template>
