<script setup lang="ts">
import { ref, computed } from 'vue';
import { useRemote } from '../composables/useRemote';
import RemoteCompCard from '../components/RemoteCompCard.vue';
import RemoteTrackControl from '../components/RemoteTrackControl.vue';

const remote = useRemote();
const activeTab = ref(0);
const selectedCompId = ref<string | null>(null);

const currentGroup = computed(() => remote.groups.value[activeTab.value] ?? null);
const selectedComp = computed(() =>
  currentGroup.value?.comps.find((c) => c.id === selectedCompId.value) ?? null
);
</script>

<template>
  <VContainer fluid>
    <VTabs v-model="activeTab" density="compact" v-if="remote.groups.value.length > 0" class="mb-4">
      <VTab v-for="(group, i) in remote.groups.value" :key="group.id" :value="i">
        {{ group.name }}
      </VTab>
    </VTabs>

    <div v-if="currentGroup" class="d-flex" style="height: calc(100vh - 120px);">
      <!-- Comp grid -->
      <div style="width: 300px; min-width: 300px; overflow-y: auto;" class="pr-3">
        <div class="d-flex flex-wrap ga-2">
          <RemoteCompCard
            v-for="comp in currentGroup.comps"
            :key="comp.id"
            :comp="comp"
            :selected="comp.id === selectedCompId"
            @select="selectedCompId = $event"
          />
        </div>
        <div v-if="currentGroup.comps.length === 0" class="text-medium-emphasis text-center pa-4">
          No comps.
        </div>
      </div>

      <!-- Track details -->
      <VDivider vertical class="mx-2" />
      <div style="flex: 1; overflow-y: auto;" class="pl-3">
        <div v-if="selectedComp">
          <span class="text-h6 font-weight-bold mb-3 d-block">{{ selectedComp.name }}</span>
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
          Select a composition.
        </div>
      </div>
    </div>

    <div v-else class="text-medium-emphasis text-center pa-8">
      No groups yet.
    </div>
  </VContainer>
</template>
