<script setup lang="ts">
import { computed, ref } from 'vue';
import type { Role } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import FacetStatus from './FacetStatus.vue';

/// Combat / Non-combat toggle for one or more entries, plus its status column.
/// The shown role is the confirmed one if set, else the analyzer's suggestion.
/// Clicking an option confirms it; the status column offers Confirm (keep the
/// suggestion) or Use suggestion (clear the confirmed role).
///
/// The root uses `display: contents`, so inside a grid the options and the
/// status are two cells that line up with the other facet rows.
const props = defineProps<{
  entryIds: string[];
  roleAuto: Role | null;
  roleAutoScore: number | null;
  roleUser: Role | null;
}>();

const store = useAppStore();
const busy = ref(false);

const options: { value: Role; label: string; icon: string }[] = [
  { value: 'combat', label: 'Combat', icon: 'mdi-sword-cross' },
  { value: 'nonCombat', label: 'Non-combat', icon: 'mdi-music-note' },
];

const labelOf = (role: Role | null): string | null =>
  options.find((o) => o.value === role)?.label ?? null;

const shown = computed(() => props.roleUser ?? props.roleAuto);
const autoHint = computed(() =>
  props.roleAutoScore === null ? null : `Combat probability ${props.roleAutoScore.toFixed(2)}`,
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

function choose(role: Role): void {
  if (props.roleUser !== role) void run('set_entry_role', { role });
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
      :auto-label="labelOf(roleAuto)"
      :user-label="labelOf(roleUser)"
      :auto-hint="autoHint"
      :disabled="busy"
      @confirm="run('keep_auto_role', {})"
      @use-suggestion="run('set_entry_role', { role: null })"
    />
  </div>
</template>

<style scoped>
.facet-control {
  display: contents;
}
</style>
