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
    <!-- Left pane: compact comp list -->
    <div style="width: 200px; min-width: 160px; max-width: 240px; display: flex; flex-direction: column;">
      <!-- Scrollable comp list -->
      <div style="flex: 1; overflow-y: auto;" class="d-flex flex-column ga-1">
        <CompCard
          v-for="comp in group.comps"
          :key="comp.id"
          :comp="comp"
          :group-id="group.id"
          :selected="comp.id === selectedCompId"
          @select="selectedCompId = $event"
        />

        <div v-if="group.comps.length === 0" class="text-medium-emphasis text-center pa-4 text-caption">
          No compositions yet.
        </div>
      </div>

      <!-- Pinned Add button at bottom -->
      <div class="pt-2">
        <VBtn
          prepend-icon="mdi-plus"
          variant="tonal"
          rounded="xl"
          block
          size="small"
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
