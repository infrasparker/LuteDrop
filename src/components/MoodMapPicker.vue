<script setup lang="ts">
import { computed, ref } from 'vue';
import type { MoodStep } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { effectiveStep, squareFor, type MoodSquare } from '../utils/mood';
import MoodGrid from './MoodGrid.vue';

/// The "Mood" row of the track editor: a button naming the square the two
/// mood scales put the track in, which opens the 3×3 mood map. The track's
/// square is pinned and the analyzer's square gets a sparkle when it differs.
/// Clicking a square sets both scales to that square's steps as yours.
const props = defineProps<{
  entryIds: string[];
  brightZ: number | null;
  brightUser: MoodStep | null;
  lightZ: number | null;
  lightUser: MoodStep | null;
}>();

const store = useAppStore();
const open = ref(false);
const busy = ref(false);

const current = computed(() =>
  squareFor(effectiveStep(props.brightZ, props.brightUser), effectiveStep(props.lightZ, props.lightUser)),
);
const auto = computed(() => squareFor(effectiveStep(props.brightZ, null), effectiveStep(props.lightZ, null)));

async function pick(sq: MoodSquare): Promise<void> {
  if (busy.value || props.entryIds.length === 0) return;
  busy.value = true;
  try {
    // Both scales become yours, even one that already matched the analyzer:
    // picking a square is a decision about the whole square.
    if (props.brightUser !== sq.bright) {
      await store.command('set_entry_mood_step', { entryIds: props.entryIds, scale: 'bright', step: sq.bright });
    }
    if (props.lightUser !== sq.light) {
      await store.command('set_entry_mood_step', { entryIds: props.entryIds, scale: 'light', step: sq.light });
    }
    open.value = false;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <VMenu v-model="open" :close-on-content-click="false" location="bottom start">
    <template #activator="{ props: activator }">
      <VBtn
        v-bind="activator"
        size="small"
        variant="outlined"
        prepend-icon="mdi-grid"
        append-icon="mdi-menu-down"
        :disabled="busy"
      >
        {{ current ? current.name : 'Not placed' }}
      </VBtn>
    </template>
    <VCard class="pa-3" max-width="560">
      <div class="text-subtitle-2">
        Mood map<template v-if="current"> · this track: <strong>{{ current.name }}</strong></template>
      </div>
      <div class="text-caption text-medium-emphasis mb-2">
        Click a square to set both scales below as yours.
        <template v-if="auto && current && auto.key !== current.key">
          The analyzer would say {{ auto.name }} (✨).
        </template>
      </div>
      <MoodGrid
        :current="current?.key ?? null"
        :auto="auto?.key ?? null"
        pickable
        @pick="pick"
      />
    </VCard>
  </VMenu>
</template>
