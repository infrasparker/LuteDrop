<script setup lang="ts">
import { ref, watch } from 'vue';
import type { Group } from '@generated/index';
import { useRemote } from './composables/useRemote';
import RemotePlaybackView from './views/RemotePlaybackView.vue';

const remote = useRemote();

function isGroupActive(group: Group): boolean {
  return group.comps.some((comp) =>
    comp.tracks.some((track) => {
      const s = remote.playbackStates.get(track.id);
      return s?.isPlaying || s?.isPaused;
    })
  );
}
const paused = ref(false);
const activeTab = ref(0);

const toastVisible = ref(false);
const toastMessage = ref('');
const toastColor = ref('error');

function showToast(message: string, color = 'error'): void {
  toastMessage.value = message;
  toastColor.value = color;
  toastVisible.value = true;
}

watch(remote.connected, (connected, wasConnected) => {
  if (wasConnected && !connected) {
    showToast('Disconnected from server', 'error');
  } else if (!wasConnected && connected) {
    showToast('Connected to server', 'success');
  }
});

function togglePause(): void {
  if (paused.value) {
    remote.resumeAll();
    paused.value = false;
  } else {
    remote.pauseAll();
    paused.value = true;
  }
}

function stopAll(): void {
  paused.value = false;
  remote.stopAll();
}
</script>

<template>
  <VApp @contextmenu.prevent>
    <VAppBar color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop Remote</span>
        <VIcon
          :color="remote.connected.value ? 'success' : 'error'"
          size="x-small"
          class="ml-3"
        >
          {{ remote.connected.value ? 'mdi-wifi' : 'mdi-wifi-off' }}
        </VIcon>
      </VAppBarTitle>

      <template #append>
        <VIcon class="mr-1">mdi-volume-high</VIcon>
        <VSlider
          :model-value="remote.masterVolume.value"
          @update:model-value="(v: number) => { remote.masterVolume.value = v; remote.setMasterVolume(v); }"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 120px"
        />
        <VBtn
          :icon="paused ? 'mdi-play' : 'mdi-pause'"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="togglePause"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="stopAll"
        />
      </template>
      <template #extension>
        <VTabs v-model="activeTab" color="primary" v-if="remote.groups.value.length > 0">
          <VTab
            v-for="(group, i) in remote.groups.value"
            :key="group.id"
            :value="i"
            class="tab-truncate"
            :style="isGroupActive(group) ? { color: '#FF6D00' } : {}"
          >
            {{ group.name }}
          </VTab>
        </VTabs>
      </template>
    </VAppBar>

    <VMain class="d-flex flex-column" style="height: 100vh;">
      <RemotePlaybackView :active-tab="activeTab" class="flex-grow-1" style="overflow: hidden;" />
    </VMain>

    <VSnackbar v-model="toastVisible" :color="toastColor" :timeout="3000">
      {{ toastMessage }}
    </VSnackbar>
  </VApp>
</template>

<style>
.tab-truncate {
  max-width: 150px !important;
}
.tab-truncate .v-btn__content {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  display: block !important;
}
</style>
