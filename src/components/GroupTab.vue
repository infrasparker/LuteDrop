<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Group } from '@generated/index';
import CompCard from './CompCard.vue';
import CompDetails from './CompDetails.vue';

const props = defineProps<{
  group: Group;
}>();

const emit = defineEmits<{
  'add-comp': [groupId: string];
}>();

const selectedCompId = ref<string | null>(null);

const selectedComp = computed(() =>
  props.group.comps.find((c) => c.id === selectedCompId.value) ?? null
);
</script>

<template>
  <div class="d-flex" style="height: 100%;">
    <!-- Left pane: comp list -->
    <div style="width: 300px; min-width: 240px; max-width: 360px; display: flex; flex-direction: column;">
      <!-- Scrollable comp list -->
      <VList density="comfortable" class="overflow-y-auto pa-1" style="flex: 1;">
        <CompCard
          v-for="comp in group.comps"
          :key="comp.id"
          :comp="comp"
          :group-id="group.id"
          :selected="comp.id === selectedCompId"
          @select="selectedCompId = $event"
        />

        <div v-if="group.comps.length === 0" class="text-medium-emphasis text-center pa-4 text-body-2">
          No compositions yet.
        </div>
      </VList>

      <!-- Pinned Add button at bottom -->
      <div class="pt-2">
        <VBtn
          prepend-icon="mdi-plus"
          variant="tonal"
          rounded="xl"
          block
          size="default"
          @click="emit('add-comp', group.id)"
        >
          Add Comp
        </VBtn>
      </div>
    </div>

    <!-- Right pane: comp details -->
    <VDivider vertical class="mx-2" />
    <div style="flex: 1; overflow-y: auto;" class="pl-2">
      <CompDetails
        v-if="selectedComp"
        :comp="selectedComp"
        :group-id="group.id"
      />
      <div v-else class="text-medium-emphasis text-center pa-8">
        Select a composition to see its tracks.
      </div>
    </div>
  </div>
</template>
