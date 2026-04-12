<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import type { AppState } from '@generated/index';
import PlaybackView from './views/PlaybackView.vue';

const store = useAppStore();
const audio = useAudio();
const masterVolume = ref(1);

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);
});

function setMasterVolume(v: number): void {
  masterVolume.value = v;
  audio.setMasterVolume(v);
}

function stopAll(): void {
  audio.stopAll();
}
</script>

<template>
  <VApp>
    <VAppBar density="compact" color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
      </VAppBarTitle>

      <template #append>
        <VIcon class="mr-1">mdi-volume-high</VIcon>
        <VSlider
          :model-value="masterVolume"
          @update:model-value="setMasterVolume"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 120px"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="stopAll"
        />
      </template>
    </VAppBar>

    <VMain>
      <PlaybackView />
    </VMain>
  </VApp>
</template>
