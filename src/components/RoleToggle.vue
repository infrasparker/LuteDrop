<script setup lang="ts">
import { ref } from 'vue';
import type { LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';
const props = defineProps<{ entry: LibraryEntry }>();
const store = useAppStore();
const busy = ref(false);
async function toggle(): Promise<void> {
  if (busy.value || props.entry.entryType !== 'music') return;
  busy.value = true;
  try { await store.command('set_entry_role', { entryIds: [props.entry.id], role: props.entry.role === 'combat' ? 'nonCombat' : 'combat' }); }
  finally { busy.value = false; }
}
</script>
<template>
  <VBtn v-if="entry.role" :icon="entry.role === 'combat' ? 'mdi-sword-cross' : 'mdi-music-note'" variant="text" size="x-small" :loading="busy" :aria-label="`${entry.role === 'combat' ? 'Combat' : 'Non-combat'}: change ${entry.name} to ${entry.role === 'combat' ? 'non-combat' : 'combat'}`" :title="`${entry.role === 'combat' ? 'Combat' : 'Non-combat'}. Click to switch role`" @click.stop="toggle" />
</template>
