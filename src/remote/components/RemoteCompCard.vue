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

const borderStyle = computed(() => {
  switch (status.value) {
    case 'active': return '3px solid #4CAF50';
    case 'partial': return '3px solid #FF6D00';
    default: return '3px solid transparent';
  }
});
</script>

<template>
  <VCard
    :color="selected ? 'primary' : 'surface-light'"
    variant="flat"
    class="pa-2 text-center"
    :style="{ width: '130px', cursor: 'pointer', border: borderStyle }"
    @click="emit('select', comp.id)"
  >
    <div class="d-flex justify-center ga-1 mb-2">
      <VBtn
        :icon="status === 'active' ? 'mdi-pause' : 'mdi-play'"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="status === 'active' ? remote.stopComp(comp.id) : remote.activateComp(comp.id)"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="remote.stopComp(comp.id)"
      />
    </div>
    <div class="text-subtitle-2 font-weight-bold text-truncate">{{ comp.name }}</div>
    <div class="text-caption" style="opacity: 0.6;">{{ comp.tracks.length }} tracks</div>
  </VCard>
</template>
