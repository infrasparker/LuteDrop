<script setup lang="ts">
import { computed, ref } from 'vue';
import type { MoodScale, MoodStep } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { effectiveStep, STEP_LABELS, STEPS } from '../utils/mood';

/// Three-step toggle for one mood scale (Dark/Neutral/Bright or
/// Heavy/Neutral/Light). The shown step is the user's if set, else the
/// analyzer's, which is outlined with a creation icon and a z-score tooltip.
/// Clicking a step stores it as the user's choice; Reset hands it back to the analyzer.
const props = defineProps<{
  entryIds: string[];
  scale: MoodScale;
  z: number | null;
  user: MoodStep | null;
}>();

const store = useAppStore();
const busy = ref(false);

const step = computed(() => effectiveStep(props.z, props.user));
const isAuto = computed(() => props.user === null && step.value !== null);
const zLabel = computed(() =>
  props.z === null ? 'Auto' : `Auto · z = ${props.z >= 0 ? '+' : ''}${props.z.toFixed(2)}`,
);
const labels = computed(() => STEP_LABELS[props.scale]);

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
</script>

<template>
  <div class="d-flex align-center ga-2">
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
        :prepend-icon="isAuto && step === value ? 'mdi-creation' : undefined"
        :variant="isAuto && step === value ? 'outlined' : undefined"
        @click.prevent="choose(value)"
      >
        {{ labels[i] }}
        <VTooltip
          v-if="isAuto && step === value"
          activator="parent"
          location="top"
          open-delay="300"
        >{{ zLabel }}</VTooltip>
      </VBtn>
    </VBtnToggle>
    <VBtn
      v-if="user !== null"
      size="small"
      variant="text"
      :disabled="busy"
      @click="setStep(null)"
    >
      Reset
    </VBtn>
  </div>
</template>
