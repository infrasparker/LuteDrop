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
    <!-- Left pane: group header + comp grid -->
    <div style="width: 300px; min-width: 300px; display: flex; flex-direction: column;" class="pr-3">
      <!-- Scrollable comp area -->
      <div style="flex: 1; overflow-y: auto;">
        <div class="d-flex flex-wrap ga-2">
          <CompCard
            v-for="comp in group.comps"
            :key="comp.id"
            :comp="comp"
            :group-id="group.id"
            :selected="comp.id === selectedCompId"
            @select="selectedCompId = $event"
          />
        </div>

        <div v-if="group.comps.length === 0" class="text-medium-emphasis text-center pa-4">
          No compositions yet.
        </div>
      </div>

      <!-- Pinned Add Composition button at bottom -->
      <div class="pt-3">
        <VBtn
          prepend-icon="mdi-plus"
          variant="tonal"
          rounded="xl"
          block
          @click="emit('add-comp', group.id)"
        >
          Add Composition
        </VBtn>
      </div>
    </div>

    <!-- Right pane: comp details -->
    <VDivider vertical class="mx-2" />
    <div style="flex: 1; overflow-y: auto;" class="pl-3">
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
