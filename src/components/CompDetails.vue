<script setup lang="ts">
import type { Comp } from '@generated/index';
import { useTrackImport } from '../composables/useTrackImport';
import TrackControl from './TrackControl.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
}>();

const { importTracks } = useTrackImport();
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
    />

    <div v-if="comp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
      No tracks yet. Click "Add Track" to import audio files.
    </div>

  </div>
</template>
