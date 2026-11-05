<script setup lang="ts">
import { computed, ref } from 'vue';
import type { Pace } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import FacetStatus from './FacetStatus.vue';

/// Still / Moving toggle for one or more entries, plus its status column.
/// The shown pace is yours if set, else the analyzer's. Clicking an option
/// makes it yours; the status column offers Confirm (yours = the analyzer's)
/// or Use auto (clear yours).
///
/// The root uses `display: contents`, so inside a grid the options and the
/// status are two cells that line up with the other facet rows.
const props = defineProps<{
  entryIds: string[];
  paceAuto: Pace | null;
  paceAutoScore: number | null;
  paceUser: Pace | null;
}>();

const store = useAppStore();
const busy = ref(false);

const options: { value: Pace; label: string; icon: string }[] = [
  { value: 'still', label: 'Still', icon: 'mdi-pause' },
  { value: 'moving', label: 'Moving', icon: 'mdi-play' },
];

const labelOf = (pace: Pace | null): string | null =>
  options.find((o) => o.value === pace)?.label ?? null;

const shown = computed(() => props.paceUser ?? props.paceAuto);
const autoHint = computed(() =>
  props.paceAutoScore === null ? null : `Moving probability ${props.paceAutoScore.toFixed(2)}`,
);

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
  if (props.paceUser !== pace) void run('set_entry_pace', { pace });
}
</script>

<template>
  <div class="facet-control">
    <VBtnToggle
      :model-value="shown"
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
        :prepend-icon="opt.icon"
        @click.prevent="choose(opt.value)"
      >
        {{ opt.label }}
      </VBtn>
    </VBtnToggle>
    <FacetStatus
      :auto-label="labelOf(paceAuto)"
      :user-label="labelOf(paceUser)"
      :auto-hint="autoHint"
      :disabled="busy"
      @confirm="run('keep_auto_pace', {})"
      @use-auto="run('set_entry_pace', { pace: null })"
    />
  </div>
</template>

<style scoped>
.facet-control {
  display: contents;
}
</style>
