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
  <VContainer fluid class="d-flex flex-column" style="height: 100%;">
    <div class="d-flex align-center mb-4 ga-3">
      <VTabs v-model="activeTab" density="compact" v-if="remote.groups.value.length > 0">
        <VTab
          v-for="(group, i) in remote.groups.value"
          :key="group.id"
          :value="i"
          class="tab-truncate"
        >
          {{ group.name }}
        </VTab>
      </VTabs>
    </div>

    <div v-if="currentGroup" class="d-flex flex-grow-1" style="overflow: hidden;">
      <!-- Comp grid -->
      <div style="width: 300px; min-width: 300px; display: flex; flex-direction: column;" class="pr-3">
        <div style="flex: 1; overflow-y: auto;">
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
            No compositions.
          </div>
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
          Select a composition to see its tracks.
        </div>
      </div>
    </div>

    <div v-else class="text-medium-emphasis text-center pa-8">
      No groups yet.
    </div>
  </VContainer>
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
