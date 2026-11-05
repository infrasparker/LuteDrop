<script setup lang="ts">
import { computed, ref } from 'vue';
import type { Role, TagSource } from '@generated/index';
import { useAppStore } from '../stores/appStore';

/// Combat / Non-combat toggle for one or more entries. An auto role is
/// outlined with a creation icon and a score tooltip, and offers Keep (turns it
/// into the user's choice). Clear stores "no role" as the user's decision, so
/// later analyzer runs leave the entry unset.
const props = defineProps<{
  entryIds: string[];
  role: Role | null;
  roleSource: TagSource | null;
  roleScore: number | null;
}>();

const store = useAppStore();
const busy = ref(false);

const isAuto = computed(() => props.role !== null && props.roleSource === 'auto');
const scoreLabel = computed(() =>
  props.roleScore === null ? 'Auto' : `Auto · ${props.roleScore.toFixed(2)}`,
);

const options: { value: Role; label: string; icon: string }[] = [
  { value: 'combat', label: 'Combat', icon: 'mdi-sword-cross' },
  { value: 'nonCombat', label: 'Non-combat', icon: 'mdi-music-note' },
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

function choose(role: Role): void {
  // Clicking the auto role itself confirms it.
  if (isAuto.value && props.role === role) {
    void run('keep_auto_role', {});
  } else if (props.role !== role || props.roleSource !== 'user') {
    void run('set_entry_role', { role });
  }
}

function clear(): void {
  void run('set_entry_role', { role: null });
}
</script>

<template>
  <div class="d-flex align-center ga-2">
    <VBtnToggle
      :model-value="role"
      density="compact"
      variant="outlined"
      divided
      color="primary"
      :disabled="busy"
    >
      <VBtn
        v-for="opt in options"
        :key="opt.value"
        :value="opt.value"
        size="small"
        :prepend-icon="isAuto && role === opt.value ? 'mdi-creation' : opt.icon"
        :variant="isAuto && role === opt.value ? 'outlined' : undefined"
        @click.prevent="choose(opt.value)"
      >
        {{ opt.label }}
        <VTooltip
          v-if="isAuto && role === opt.value"
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
      @click="run('keep_auto_role', {})"
    >
      Keep
    </VBtn>
    <VBtn
      icon="mdi-close"
      size="small"
      variant="text"
      title="Clear role"
      :disabled="busy || role === null"
      @click="clear"
    />
  </div>
</template>
