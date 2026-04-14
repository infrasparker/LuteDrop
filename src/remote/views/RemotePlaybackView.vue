<script setup lang="ts">
import { ref, computed } from 'vue';
import { useRemote } from '../composables/useRemote';
import RemoteCompCard from '../components/RemoteCompCard.vue';
import RemoteTrackControl from '../components/RemoteTrackControl.vue';

const props = defineProps<{
  activeTab: number;
}>();

const remote = useRemote();
const selectedCompId = ref<string | null>(null);

const currentGroup = computed(() => remote.groups.value[props.activeTab] ?? null);
const selectedComp = computed(() =>
  currentGroup.value?.comps.find((c) => c.id === selectedCompId.value) ?? null
);
</script>

<template>
  <div class="d-flex flex-column pa-3" style="height: 100%;">
    <div v-if="currentGroup" class="d-flex flex-grow-1" style="overflow: hidden;">
      <!-- Left pane: comp list -->
      <div style="width: 300px; min-width: 240px; max-width: 360px; display: flex; flex-direction: column;">
        <VList density="comfortable" class="overflow-y-auto pa-1" style="flex: 1;">
          <RemoteCompCard
            v-for="comp in currentGroup.comps"
            :key="comp.id"
            :comp="comp"
            :selected="comp.id === selectedCompId"
            @select="selectedCompId = $event"
          />
          <div v-if="currentGroup.comps.length === 0" class="text-medium-emphasis text-center pa-4 text-body-2">
            No compositions.
          </div>
        </VList>
      </div>

      <!-- Right pane: track details -->
      <VDivider vertical class="mx-2" />
      <div style="flex: 1; overflow-y: auto;" class="pl-2">
        <div v-if="selectedComp">
          <RemoteTrackControl
            v-for="track in selectedComp.tracks"
            :key="track.id"
            :track="track"
          />
          <div v-if="selectedComp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
            No tracks.
          </div>
        </div>
        <div v-else class="text-medium-emphasis text-center pa-8">
          Select a composition to see its tracks.
        </div>
      </div>
    </div>

    <div v-else class="text-medium-emphasis text-center pa-8">
      No groups yet.
    </div>
  </div>
</template>

