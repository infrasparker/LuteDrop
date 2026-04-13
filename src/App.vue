<script setup lang="ts">
import { onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import { useToast } from './composables/useToast';
import type { AppState } from '@generated/index';
import TransportBar from './components/TransportBar.vue';

const store = useAppStore();
const toast = useToast();

// Initialize audio early to register event listeners
useAudio();

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
          prepend-icon="mdi-speaker-group"
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

    <VSnackbar v-model="toast.visible.value" :color="toast.color.value" :timeout="3000">
      {{ toast.message.value }}
    </VSnackbar>
  </VApp>
</template>
