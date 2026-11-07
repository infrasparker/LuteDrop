<script setup lang="ts">
import { computed, ref } from 'vue';
import type { Role } from '@generated/index';
import { useAppStore } from '../stores/appStore';

/// Combat / Non-combat toggle for one or more entries. Exactly one option is
/// always selected: the role the user set, else Non-combat by default.
/// Clicking an option sets it.
const props = defineProps<{
  entryIds: string[];
  roleUser: Role | null;
}>();

const store = useAppStore();
const busy = ref(false);

const options: { value: Role; label: string; icon: string }[] = [
  { value: 'combat', label: 'Combat', icon: 'mdi-sword-cross' },
  { value: 'nonCombat', label: 'Non-combat', icon: 'mdi-music-note' },
];

const shown = computed<Role>(() => props.roleUser ?? 'nonCombat');

async function choose(role: Role): Promise<void> {
  if (busy.value || props.entryIds.length === 0 || role === shown.value) return;
  busy.value = true;
  try {
    await store.command('set_entry_role', { entryIds: props.entryIds, role });
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <VBtnToggle
    :model-value="shown"
    class="role-control"
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
</template>

<style scoped>
/* The same text size as the other controls in the editor. */
.role-control :deep(.v-btn) {
  font-size: 0.8125rem;
}
</style>
