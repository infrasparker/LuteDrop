<script setup lang="ts">
import { ref, computed, watch } from 'vue';
import { isGroupActive } from '../shared/playback';
import { useRemote } from './composables/useRemote';
import RemotePlaybackView from './views/RemotePlaybackView.vue';

const remote = useRemote();

const localMasterVolume = ref(remote.masterVolume.value);
watch(remote.masterVolume, (v) => { localMasterVolume.value = v; });

function setMasterVolume(v: number): void {
  localMasterVolume.value = v;
  remote.masterVolume.value = v;
  remote.setMasterVolume(v);
}

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

const anyPlaying = computed(() => {
  for (const state of remote.playbackStates.values()) {
    if (state.isPlaying) return true;
  }
  return false;
});

const anyPaused = computed(() => {
  for (const state of remote.playbackStates.values()) {
    if (state.isPaused) return true;
  }
  return false;
});

const showPlayIcon = computed(() => !anyPlaying.value && anyPaused.value);

function togglePause(): void {
  if (anyPlaying.value) {
    remote.pauseAll();
  } else if (anyPaused.value) {
    remote.resumeAll();
  }
}

function stopAll(): void {
  remote.stopAll();
}
</script>

<template>
  <VApp @contextmenu.prevent>
    <VAppBar flat color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
        <VIcon
          :color="remote.connected.value ? 'success' : 'error'"
          size="x-small"
          class="ml-3"
        >
          {{ remote.connected.value ? 'mdi-wifi' : 'mdi-wifi-off' }}
        </VIcon>
      </VAppBarTitle>

      <VSpacer />

      <VBtn
        :icon="showPlayIcon ? 'mdi-play' : 'mdi-pause'"
        :disabled="!anyPlaying && !anyPaused"
        variant="tonal"
        size="small"
        rounded="circle"
        class="ml-1"
        @click="togglePause"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="small"
        rounded="circle"
        class="ml-1"
        @click="stopAll"
      />
      <VBtn
        icon="mdi-volume-high"
        variant="text"
        size="small"
        class="ml-1"
      />
      <VSlider
        :model-value="localMasterVolume"
        @update:model-value="setMasterVolume"
        :min="0"
        :max="1"
        :step="0.01"
        hide-details
        density="compact"
        style="flex: 0 0 120px;"
        class="mr-4"
      />

      <template #extension>
        <VTabs v-model="activeTab" color="primary" density="compact" v-if="remote.groups.value.length > 0" class="app-bar-tabs" show-arrows>
          <VTab
            v-for="(group, i) in remote.groups.value"
            :key="group.id"
            :value="i"
            :class="{ 'tab-active': isGroupActive(group, remote.playbackStates) }"
            size="small"
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
.tab-active {
  border-left: 3px solid #F0A830;
  background: rgba(240, 168, 48, 0.08);
}
.app-bar-tabs {
  height: 100%;
}
</style>
