<script setup lang="ts">
import { ref, computed } from 'vue';
import { useDisplay } from 'vuetify';
import { useRemote } from '../composables/useRemote';
import RemoteCompCard from '../components/RemoteCompCard.vue';
import RemoteTrackControl from '../components/RemoteTrackControl.vue';

const props = defineProps<{
  activeTab: number;
}>();

const remote = useRemote();
const { mdAndUp } = useDisplay();
const selectedCompId = ref<string | null>(null);

const currentGroup = computed(() => remote.groups.value[props.activeTab] ?? null);
const selectedComp = computed(() =>
  currentGroup.value?.comps.find((c) => c.id === selectedCompId.value) ?? null
);

const showCompPane = computed(() => mdAndUp.value || !selectedComp.value);
const showTrackPane = computed(() => mdAndUp.value || !!selectedComp.value);

function selectComp(compId: string): void {
  selectedCompId.value = compId;
}

function backToComps(): void {
  selectedCompId.value = null;
}
</script>

<template>
  <div class="d-flex flex-column pa-3" style="height: 100%;">
    <div v-if="currentGroup" class="d-flex flex-column flex-md-row flex-grow-1" style="overflow: hidden;">
      <!-- Comp list -->
      <div
        v-show="showCompPane"
        class="comp-pane d-flex flex-column"
      >
        <VList density="comfortable" class="overflow-y-auto pa-1" style="flex: 1;">
          <RemoteCompCard
            v-for="comp in currentGroup.comps"
            :key="comp.id"
            :comp="comp"
            :selected="comp.id === selectedCompId"
            @select="selectComp"
          />
          <div v-if="currentGroup.comps.length === 0" class="text-medium-emphasis text-center pa-4 text-body-2">
            No compositions.
          </div>
        </VList>
      </div>

      <!-- Track details -->
      <VDivider vertical class="mx-2 d-none d-md-flex" />
      <div
        v-show="showTrackPane"
        class="track-pane d-flex flex-column flex-grow-1 overflow-y-auto"
      >
        <div v-if="selectedComp">
          <VBtn
            variant="text"
            size="small"
            prepend-icon="mdi-arrow-left"
            class="mb-1 d-md-none"
            @click="backToComps"
          >
            Comps
          </VBtn>
          <div class="text-subtitle-1 font-weight-bold mb-2 d-md-none px-2">{{ selectedComp.name }}</div>
          <RemoteTrackControl
            v-for="track in selectedComp.tracks"
            :key="track.id"
            :track="track"
          />
          <div v-if="selectedComp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
            No tracks.
          </div>
        </div>
        <div v-else class="text-medium-emphasis text-center pa-8 d-none d-md-block">
          Select a composition to see its tracks.
        </div>
      </div>
    </div>

    <div v-else class="text-medium-emphasis text-center pa-8">
      No groups yet.
    </div>
  </div>
</template>

<style scoped>
.comp-pane {
  min-width: 0;
}

.track-pane {
  min-width: 0;
}

/* Desktop: fixed-width comp pane for side-by-side layout */
@media (min-width: 960px) {
  .comp-pane {
    width: 300px;
    min-width: 240px;
    max-width: 360px;
  }
}
</style>
