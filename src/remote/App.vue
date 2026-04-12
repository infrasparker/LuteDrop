<script setup lang="ts">
import { useRemote } from './composables/useRemote';
import RemotePlaybackView from './views/RemotePlaybackView.vue';

const remote = useRemote();
</script>

<template>
  <VApp>
    <VAppBar density="compact" color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
        <span class="text-caption text-medium-emphasis ml-2">Remote</span>
        <VIcon
          :color="remote.connected.value ? 'success' : 'error'"
          size="x-small"
          class="ml-2"
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
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="remote.stopAll()"
        />
      </template>
    </VAppBar>

    <VMain>
      <RemotePlaybackView />
    </VMain>
  </VApp>
</template>
