<script setup lang="ts">
import { computed, ref } from 'vue';
import type { Pace, TagSource } from '@generated/index';
import { useAppStore } from '../stores/appStore';

/// Still / Moving toggle for one or more entries. An auto pace is outlined with
/// a creation icon and a score tooltip, and offers Keep (turns it into the
/// user's choice). Clear stores "no pace" as the user's decision, so later
/// analyzer runs leave the entry unset.
const props = defineProps<{
  entryIds: string[];
  pace: Pace | null;
  paceSource: TagSource | null;
  paceScore: number | null;
}>();

const store = useAppStore();
const busy = ref(false);

const isAuto = computed(() => props.pace !== null && props.paceSource === 'auto');
const scoreLabel = computed(() =>
  props.paceScore === null ? 'Auto' : `Auto · ${props.paceScore.toFixed(2)}`,
);

const options: { value: Pace; label: string; icon: string }[] = [
  { value: 'still', label: 'Still', icon: 'mdi-pause' },
  { value: 'moving', label: 'Moving', icon: 'mdi-play' },
];

async function run(cmd: string, args: Record<string, unknown>): Promise<void> {
  if (busy.value || props.entryIds.length === 0) return;
  busy.value = true;
  try {
    await store.command(cmd, { entryIds: props.entryIds, ...args });
  } finally {
    busy.value = false;
  }
}

function choose(pace: Pace): void {
  // Clicking the auto pace itself confirms it.
  if (isAuto.value && props.pace === pace) {
    void run('keep_auto_pace', {});
  } else if (props.pace !== pace || props.paceSource !== 'user') {
    void run('set_entry_pace', { pace });
  }
}

function clear(): void {
  void run('set_entry_pace', { pace: null });
}
</script>

<template>
  <div class="d-flex align-center ga-2">
    <VBtnToggle
      :model-value="pace"
      density="compact"
      variant="outlined"
      divided
      color="primary"
      :disabled="busy"
      @update:model-value="() => {}"
    >
      <VBtn
        v-for="opt in options"
        :key="opt.value"
        :value="opt.value"
        size="small"
        :prepend-icon="isAuto && pace === opt.value ? 'mdi-creation' : opt.icon"
        :variant="isAuto && pace === opt.value ? 'outlined' : undefined"
        @click.prevent="choose(opt.value)"
      >
        {{ opt.label }}
        <VTooltip
          v-if="isAuto && pace === opt.value"
          activator="parent"
          location="top"
          open-delay="300"
        >{{ scoreLabel }}</VTooltip>
      </VBtn>
    </VBtnToggle>
    <VBtn
      v-if="isAuto"
      prepend-icon="mdi-check"
      size="small"
      variant="tonal"
      :disabled="busy"
      @click="run('keep_auto_pace', {})"
    >
      Keep
    </VBtn>
    <span v-if="pace === null && paceSource === 'user'" class="text-caption text-medium-emphasis">
      No pace (yours)
    </span>
    <VBtn
      icon="mdi-close"
      size="small"
      variant="text"
      title="Clear pace"
      :disabled="busy || (pace === null && paceSource === 'user')"
      @click="clear"
    />
  </div>
</template>
