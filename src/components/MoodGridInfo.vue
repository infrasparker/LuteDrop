<script setup lang="ts">
import { computed, ref } from 'vue';
import type { MoodStep } from '@generated/index';
import { effectiveStep, squareFor } from '../utils/mood';
import MoodGrid from './MoodGrid.vue';

/// An ⓘ button beside the mood rows that opens the 3×3 mood map, with this
/// track's square (as shown now, your steps winning) highlighted and the
/// analyzer's own square marked when it differs.
const props = defineProps<{
  brightZ: number | null;
  brightUser: MoodStep | null;
  lightZ: number | null;
  lightUser: MoodStep | null;
}>();

const current = computed(
  () => squareFor(effectiveStep(props.brightZ, props.brightUser), effectiveStep(props.lightZ, props.lightUser)),
);
// Hover opens the map; a click always opens it too (never toggles it shut).
const open = ref(false);
const auto = computed(() => squareFor(effectiveStep(props.brightZ, null), effectiveStep(props.lightZ, null)));
</script>

<template>
  <VMenu v-model="open" open-on-hover :open-on-click="false" :close-on-content-click="false" location="end" open-delay="200">
    <template #activator="{ props: activator }">
      <VBtn
        v-bind="activator"
        icon="mdi-information-outline"
        size="x-small"
        variant="text"
        aria-label="Show the mood map"
        @click="open = true"
      />
    </template>
    <VCard class="pa-3" max-width="560">
      <div class="text-subtitle-2 mb-1">
        Mood map<template v-if="current"> · this track: <strong>{{ current.name }}</strong></template>
      </div>
      <div v-if="auto && current && auto.key !== current.key" class="text-caption text-medium-emphasis mb-2">
        The analyzer would say {{ auto.name }} (✨).
      </div>
      <MoodGrid :current="current?.key ?? null" :auto="auto?.key ?? null" />
    </VCard>
  </VMenu>
</template>
