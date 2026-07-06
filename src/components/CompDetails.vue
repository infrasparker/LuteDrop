<script setup lang="ts">
import { ref } from 'vue';
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import TrackControl from './TrackControl.vue';
import LibraryBrowser from './LibraryBrowser.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
}>();

const store = useAppStore();

// Add track dialog state
const showAddTrackDialog = ref(false);
const newTrackName = ref('');

function openAddTrackDialog(): void {
  newTrackName.value = '';
  showAddTrackDialog.value = true;
}

async function addTrack(): Promise<void> {
  const name = newTrackName.value.trim() || 'New Track';
  await store.command('add_track', { compId: props.comp.id, name });
  showAddTrackDialog.value = false;
}

// Library browser dialog state
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
        @click="openAddTrackDialog"
      >
        Add Track
      </VBtn>
    </div>

    <TrackControl
      v-for="track in comp.tracks"
      :key="track.id"
      :track="track"
      @open-library-browser="openLibraryBrowser"
    />

    <div v-if="comp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
      No tracks yet. Click "Add Track" to get started.
    </div>

    <!-- Add Track dialog -->
    <VDialog v-model="showAddTrackDialog" max-width="400">
      <VCard title="Add Track">
        <VCardText>
          <VTextField
            v-model="newTrackName"
            label="Track Name"
            placeholder="New Track"
            variant="outlined"
            autofocus
            @keydown.enter="addTrack"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showAddTrackDialog = false" />
          <VBtn text="Add" color="primary" variant="tonal" @click="addTrack" />
        </VCardActions>
      </VCard>
    </VDialog>

    <LibraryBrowser
      v-model="showLibraryBrowser"
      :track-id="libraryBrowserTrackId"
    />
  </div>
</template>
