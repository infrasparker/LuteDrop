<script setup lang="ts">
import { computed, onMounted, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import { useToast } from './composables/useToast';
import { useRouter } from 'vue-router';
import type { AppState } from '@generated/index';
import TransportBar from './components/TransportBar.vue';
import WaveformEditorDialog from './components/WaveformEditorDialog.vue';

const store = useAppStore();
const toast = useToast();

// Initialize audio early to register event listeners
const audio = useAudio();

const router = useRouter();

const showEditorDialog = computed({
  get: () => store.editEntryId !== null,
  set: (v: boolean) => {
    if (!v) {
      // Stop library sound when closing dialog from Soundstage
      if (router.currentRoute.value.path !== '/library') {
        audio.stopLibrarySound();
        store.libraryPlayingEntryId = null;
      }
      store.editEntryId = null;
    }
  },
});

watch(() => router.currentRoute.value.path, (newPath, oldPath) => {
  if (oldPath === '/library' && newPath !== '/library') {
    audio.stopLibrarySound();
    store.libraryPlayingEntryId = null;
  }
});

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);
  await store.checkMissingFiles();

  if (store.missingEntryIds.size > 0) {
    toast.showError(`${store.missingEntryIds.size} audio file(s) not found. Check the Library view.`);
  }
});
</script>

<template>
  <VApp @contextmenu.prevent>
    <VNavigationDrawer rail expand-on-hover permanent>
      <VList nav>
        <VListItem
          prepend-icon="mdi-tune-vertical"
          title="Soundstage"
          to="/"
          color="primary"
          exact
        />
        <VListItem
          prepend-icon="mdi-bookshelf"
          title="Library"
          to="/library"
          color="primary"
        />
      </VList>
    </VNavigationDrawer>

    <VMain>
      <router-view />
    </VMain>

    <TransportBar />

    <WaveformEditorDialog
      v-if="store.editEntryId"
      v-model="showEditorDialog"
      :entry-id="store.editEntryId!"
    />

    <VSnackbar v-model="toast.visible.value" :color="toast.color.value" :timeout="3000">
      {{ toast.message.value }}
    </VSnackbar>
  </VApp>
</template>
