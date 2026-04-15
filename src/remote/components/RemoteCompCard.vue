<script setup lang="ts">
import { computed } from 'vue';
import type { Comp } from '@generated/index';
import { useRemote } from '../composables/useRemote';

const props = defineProps<{
  comp: Comp;
  selected: boolean;
}>();

const emit = defineEmits<{
  select: [compId: string];
}>();

const remote = useRemote();
const status = computed(() => remote.getCompStatus(props.comp.id));

const cardClass = computed(() => ({
  'comp-selected': props.selected,
  'comp-active': status.value === 'active',
  'comp-partial': status.value === 'partial',
  'comp-paused': status.value === 'paused',
}));

function toggleComp(): void {
  if (status.value === 'active' || status.value === 'partial') {
    remote.pauseComp(props.comp.id);
  } else if (status.value === 'paused') {
    remote.resumeComp(props.comp.id);
  } else {
    remote.activateComp(props.comp.id);
  }
}
</script>

<template>
  <VListItem
    :class="cardClass"
    @click="emit('select', comp.id)"
    rounded="lg"
    class="comp-item mb-1"
  >
    <template #prepend>
      <div class="d-flex align-center ga-1 mr-2">
        <VBtn
          :icon="status === 'active' || status === 'partial' ? 'mdi-pause' : 'mdi-play'"
          variant="tonal"
          size="small"
          rounded="circle"
          @click.stop="toggleComp"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          rounded="circle"
          :disabled="status === 'inactive'"
          @click.stop="remote.stopComp(comp.id)"
        />
      </div>
    </template>

    <VListItemTitle class="text-body-1">
      {{ comp.name }}
    </VListItemTitle>
    <VListItemSubtitle>
      {{ comp.tracks.length }} {{ comp.tracks.length === 1 ? 'track' : 'tracks' }}
    </VListItemSubtitle>
  </VListItem>
</template>

<style scoped>
.comp-item {
  border-left: 3px solid transparent;
  cursor: pointer;
}
.comp-selected {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
}
.comp-active {
  background: rgba(240, 168, 48, 0.08) !important;
  border-left: 3px solid #F0A830;
}
.comp-active.comp-selected {
  background: rgba(240, 168, 48, 0.14) !important;
}
.comp-partial {
  background: rgba(240, 168, 48, 0.05) !important;
  border-left: 3px solid rgba(240, 168, 48, 0.5);
}
.comp-paused {
  border-left: 3px solid rgba(240, 168, 48, 0.5);
}
</style>
