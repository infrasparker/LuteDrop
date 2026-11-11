<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import { useToast } from './composables/useToast';
import { useAutoTagEvents } from './composables/useAutoTagEvents';
import { useRouter } from 'vue-router';
import type { AppState } from '@generated/index';
import TrackEditorDialog from './components/TrackEditorDialog.vue';
import TransportBar from './components/TransportBar.vue';
import WaveformEditorDialog from './components/WaveformEditorDialog.vue';
import RemoteInfoDialog from './components/RemoteInfoDialog.vue';
import BroadcastDialog from './components/BroadcastDialog.vue';
import SettingsDialog from './components/SettingsDialog.vue';

const store = useAppStore();
const toast = useToast();
const showRemoteInfo = ref(false);
const showBroadcast = ref(false);
const showSettings = ref(false);

// Initialize audio early to register event listeners
const audio = useAudio();
useAutoTagEvents();

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

// Views that preview library sounds; leaving one stops the preview.
const PREVIEW_ROUTES = ['/library', '/scry', '/moods'];

watch(() => router.currentRoute.value.path, (newPath, oldPath) => {
  if (PREVIEW_ROUTES.includes(oldPath) && newPath !== oldPath) {
    audio.stopLibrarySound();
  }
});

// A startup backfill stored new values (ReplayGain loudness, or durations of
// entries that could not be decoded at import): reload state to show them.
async function reloadAfterBackfill(): Promise<void> {
  try {
    store.hydrate(await invoke<AppState>('load_data'));
  } catch (e) {
    console.warn('Reload after backfill failed', e);
  }
}
listen('library-loudness-updated', reloadAfterBackfill);
listen('library-durations-updated', reloadAfterBackfill);

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
          title="Soundstage" aria-label="Soundstage"
          :active="$route.path === '/'"
          color="primary"
          exact
        />
        <VListItem prepend-icon="mdi-playlist-music" to="/tracks" :active="$route.path === '/tracks'" title="Tracks" aria-label="Tracks" color="primary" />
        <VListItem
          prepend-icon="mdi-bookshelf"
          to="/library"
          title="Audio Library" aria-label="Audio Library"
          :active="$route.path === '/library'"
          color="primary"
        />
        <VListItem
          prepend-icon="mdi-compass-outline"
          to="/scry"
          :active="$route.path === '/scry'"
          color="primary"
          title="Scry"
          aria-label="Scry"
        />
        <VListItem
          prepend-icon="mdi-palette-outline"
          to="/moods"
          :active="$route.path === '/moods'"
          color="primary"
          title="Moods"
          aria-label="Moods"
        />
      </VList>

      <template #append>
        <VList nav density="compact">
          <VListItem
            prepend-icon="mdi-cog-outline"
            title="Settings"
            aria-label="Settings"
            @click="showSettings = true"
          />
          <VListItem
            prepend-icon="mdi-broadcast"
            title="Broadcast"
            aria-label="Broadcast"
            @click="showBroadcast = true"
          />
          <VListItem
            prepend-icon="mdi-cellphone-link"
            title="Phone remote"
            aria-label="Phone remote"
            @click="showRemoteInfo = true"
          />
        </VList>
      </template>
    </VNavigationDrawer>

    <VAppBar flat color="surface" density="compact">
      <div id="app-bar-main" class="d-flex align-center flex-grow-1 px-4" />
    </VAppBar>

    <VMain>
      <router-view />
    </VMain>

    <TransportBar />
    <TrackEditorDialog />

    <WaveformEditorDialog
      v-if="store.editEntryId"
      v-model="showEditorDialog"
      :entry-id="store.editEntryId!"
    />

    <RemoteInfoDialog v-model="showRemoteInfo" />
    <BroadcastDialog v-model="showBroadcast" />
    <SettingsDialog v-model="showSettings" />

    <!-- Top of the window so it never covers the floating selection bars or
         the transport. Click anywhere on it, or the x, to dismiss; an action
         button (such as Undo) runs its action instead. -->
    <VSnackbar
      v-model="toast.visible.value"
      :color="toast.color.value || undefined"
      :timeout="toast.timeout.value"
      location="top"
      @click="toast.hide()"
    >
      {{ toast.message.value }}
      <template #actions>
        <VBtn v-if="toast.action.value" variant="text" @click.stop="toast.runAction()">{{ toast.action.value.label }}</VBtn>
        <VBtn icon="mdi-close" size="small" variant="text" aria-label="Dismiss" @click.stop="toast.hide()" />
      </template>
    </VSnackbar>
  </VApp>
</template>
