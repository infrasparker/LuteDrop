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
      }
      store.editEntryId = null;
    }
  },
});

watch(() => router.currentRoute.value.path, (newPath, oldPath) => {
  if (oldPath === '/library' && newPath !== '/library') {
    audio.stopLibrarySound();
  }
});

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);

  if (store.missingEntryIds.size > 0) {
    toast.showError(`${store.missingEntryIds.size} audio file(s) not found. Check the Library view.`);
  }
});
</script>

<template>
  <VApp @contextmenu.prevent>
    <VNavigationDrawer rail permanent color="surface" class="nav-rail">
      <VList nav density="compact" class="mt-1">
        <VListItem
          prepend-icon="mdi-tune-vertical"
          to="/"
          :active="$route.path === '/'"
          color="primary"
          exact
        />
        <VListItem
          prepend-icon="mdi-bookshelf"
          to="/library"
          :active="$route.path === '/library'"
          color="primary"
        />
      </VList>
    </VNavigationDrawer>

    <VAppBar flat color="surface" density="compact">
      <div id="app-bar-main" class="d-flex align-center flex-grow-1 px-4" />
    </VAppBar>

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
