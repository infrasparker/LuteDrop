<script setup lang="ts">
import { computed } from 'vue';
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';

const props = defineProps<{
  comp: Comp;
  selected: boolean;
}>();

const emit = defineEmits<{
  select: [compId: string];
}>();

const store = useAppStore();
const audio = useAudio();

const status = computed(() => store.getCompStatus(props.comp.id));

const borderStyle = computed(() => {
  switch (status.value) {
    case 'active': return '2px solid #FF6D00';
    case 'partial': return '2px solid #BF360C';
    default: return '2px solid transparent';
  }
});

function toggleComp(): void {
  if (status.value === 'active') {
    audio.stopComp(props.comp.id);
  } else {
    audio.activateComp(props.comp.id);
  }
}

function stopComp(): void {
  audio.stopComp(props.comp.id);
}
</script>

<template>
  <VCard
    :color="selected ? 'primary' : 'surface-light'"
    variant="flat"
    class="comp-card pa-2 text-center"
    :style="{ width: '130px', cursor: 'pointer', border: borderStyle }"
    @click="emit('select', comp.id)"
  >
    <div class="d-flex justify-center ga-1 mb-2">
      <VBtn
        :icon="status === 'active' ? 'mdi-pause' : 'mdi-play'"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="toggleComp"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="stopComp"
      />
    </div>
    <div class="text-subtitle-2 font-weight-bold text-truncate">{{ comp.name }}</div>
    <div class="text-caption" style="opacity: 0.6;">{{ comp.tracks.length }} tracks</div>
  </VCard>
</template>
