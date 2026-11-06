<script setup lang="ts">
import { computed } from 'vue';
import type { MatchMode } from '../utils/moods';

/// The "any / all" switch shown once two or more lenses (or tag chips) are
/// selected. For lenses, any ranks by a track's best selected score and all
/// by its weakest; for tags, any shows entries with one of the selected tags
/// and all only entries with every one.
const mode = defineModel<MatchMode>({ required: true });
const props = defineProps<{ size: 'small' | 'x-small'; kind: 'lens' | 'tag' }>();

const hints = computed(() =>
  props.kind === 'lens'
    ? { any: 'Rank by the best of the selected moods', all: 'Rank by the weakest of the selected moods' }
    : { any: 'Entries with any selected tag', all: 'Entries with every selected tag' },
);
</script>

<template>
  <VBtnToggle v-model="mode" mandatory density="compact" variant="outlined" divided color="primary">
    <VBtn value="any" :size="size">
      Any
      <VTooltip activator="parent" location="top" open-delay="300">{{ hints.any }}</VTooltip>
    </VBtn>
    <VBtn value="all" :size="size">
      All
      <VTooltip activator="parent" location="top" open-delay="300">{{ hints.all }}</VTooltip>
    </VBtn>
  </VBtnToggle>
</template>
