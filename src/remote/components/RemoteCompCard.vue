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
  'comp-card': true,
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
  <VCard
    variant="flat"
    class="comp-card pa-2 text-center"
    :class="cardClass"
    style="width: 130px; cursor: pointer;"
    @click="emit('select', comp.id)"
  >
    <div class="d-flex justify-center ga-1 mb-2">
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
        @click.stop="remote.stopComp(comp.id)"
      />
    </div>
    <div class="text-subtitle-2 font-weight-bold text-truncate">{{ comp.name }}</div>
    <div class="text-caption" style="opacity: 0.6;">{{ comp.tracks.length }} tracks</div>
  </VCard>
</template>

<style scoped>
.comp-card {
  border-left: 3px solid transparent;
  background: rgb(var(--v-theme-surface-light));
}
.comp-selected {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
}
.comp-active {
  background: rgba(240, 168, 48, 0.12) !important;
  border-left: 3px solid #F0A830;
}
.comp-partial {
  background: rgba(240, 168, 48, 0.06) !important;
  border-left: 3px solid rgba(240, 168, 48, 0.5);
}
.comp-paused {
  border-left: 3px solid rgba(240, 168, 48, 0.5);
}
</style>
