<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import { useToast } from './composables/useToast';
import type { AppState } from '@generated/index';
import TransportBar from './components/TransportBar.vue';

const store = useAppStore();
const toast = useToast();
const serverUrl = ref('');

// Initialize audio early to register event listeners
useAudio();

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);
  await store.checkMissingFiles();

  if (store.missingEntryIds.size > 0) {
    toast.showError(`${store.missingEntryIds.size} audio file(s) not found. Check the Library view.`);
  }

  try {
    const info = await invoke<{ ip: string; port: number }>('get_server_info');
    serverUrl.value = `http://${info.ip}:${info.port}`;
  } catch (e) {
    console.warn('Could not get server info:', e);
  }
});
</script>

<template>
  <VApp @contextmenu.prevent>
    <VAppBar color="surface" density="compact" elevation="1">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
        <span v-if="serverUrl" class="text-caption text-medium-emphasis ml-3">
          Remote: {{ serverUrl }}
        </span>
      </VAppBarTitle>
    </VAppBar>

    <VNavigationDrawer rail expand-on-hover permanent>
      <VList nav>
        <VListItem
          prepend-icon="mdi-mixing-console"
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
