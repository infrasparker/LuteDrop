<script setup lang="ts">
import { computed, ref } from 'vue';
import type { MoodScale, MoodStep } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { effectiveStep, STEP_LABELS, STEPS } from '../utils/mood';
import FacetStatus from './FacetStatus.vue';

/// Three-step toggle for one mood scale (Dark/Neutral/Bright,
/// Intimate/Moderate/Grand or Calm/Steady/Energetic), plus its status column.
/// The shown step is the confirmed one if set, else the step for the analyzer's
/// z-score. Clicking a step confirms it; Confirm keeps the suggested step; Use
/// suggestion clears the confirmed step.
///
/// The root uses `display: contents`, so inside a grid the options and the
/// status are two cells that line up with the other facet rows.
const props = defineProps<{
  entryIds: string[];
  scale: MoodScale;
  z: number | null;
  user: MoodStep | null;
}>();

const store = useAppStore();
const busy = ref(false);

const step = computed(() => effectiveStep(props.z, props.user));
const autoStep = computed(() => effectiveStep(props.z, null));
const labels = computed(() => STEP_LABELS[props.scale]);
const labelOf = (value: MoodStep | null): string | null =>
  value === null ? null : labels.value[STEPS.indexOf(value)];
const autoHint = computed(() =>
  props.z === null ? null : `z = ${props.z >= 0 ? '+' : ''}${props.z.toFixed(2)}`,
);

async function setStep(value: MoodStep | null): Promise<void> {
  if (busy.value || props.entryIds.length === 0) return;
  busy.value = true;
  try {
    await store.command('set_entry_mood_step', {
      entryIds: props.entryIds,
      scale: props.scale,
      step: value,
    });
  } finally {
    busy.value = false;
  }
}

function choose(value: MoodStep): void {
  if (props.user !== value) void setStep(value);
}

function confirm(): void {
  if (autoStep.value !== null) void setStep(autoStep.value);
}
</script>

<template>
  <div class="facet-control">
    <VBtnToggle
      :model-value="step"
      density="compact"
      variant="outlined"
      divided
      color="primary"
      :disabled="busy"
      @update:model-value="() => {}"
    >
      <VBtn
        v-for="(value, i) in STEPS"
        :key="value"
        :value="value"
        size="small"
        @click.prevent="choose(value)"
      >
        {{ labels[i] }}
      </VBtn>
    </VBtnToggle>
    <FacetStatus
      :auto-label="labelOf(autoStep)"
      :user-label="labelOf(user)"
      :auto-hint="autoHint"
      :disabled="busy"
      @confirm="confirm"
      @use-suggestion="setStep(null)"
    />
  </div>
</template>

<style scoped>
.facet-control {
  display: contents;
}
</style>
