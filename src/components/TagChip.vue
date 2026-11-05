<script setup lang="ts">
import { computed } from 'vue';
import type { AutoTag } from '@generated/index';
import { useAppStore } from '../stores/appStore';

/// One tag on a library entry. A user tag is a plain chip. An auto tag is
/// outlined with a creation icon and a score tooltip, and clicking it opens a
/// Keep / Remove menu: Keep turns it into a normal tag, Remove rejects it so
/// later runs don't re-add it.
const props = withDefaults(defineProps<{
  tag: string;
  auto?: AutoTag | null;
  entryId: string;
  closable?: boolean;
  /** Show the Keep / Remove menu on click (auto chips only). */
  menu?: boolean;
}>(), { auto: null, menu: true });

const emit = defineEmits<{
  remove: [];
}>();

const store = useAppStore();

const scoreLabel = computed(() => (props.auto ? `Auto · ${props.auto.score.toFixed(2)}` : ''));

function keep(): void {
  store.command('keep_auto_tags', { entryIds: [props.entryId], tags: [props.tag] });
}

function reject(): void {
  store.command('reject_auto_tags', { entryIds: [props.entryId], tags: [props.tag] });
}
</script>

<template>
  <VChip
    v-if="!auto"
    size="small"
    :closable="closable"
    @click:close.stop="emit('remove')"
  >
    {{ tag }}
  </VChip>
  <VChip
    v-else
    size="small"
    variant="outlined"
    prepend-icon="mdi-creation"
    class="auto-chip"
    :closable="closable"
    @click.stop
    @click:close.stop="emit('remove')"
  >
    {{ tag }}
    <VTooltip activator="parent" location="top" open-delay="300">{{ scoreLabel }}</VTooltip>
    <VMenu v-if="menu" activator="parent" :close-on-content-click="true">
      <VList density="compact">
        <VListItem prepend-icon="mdi-check" title="Keep" @click="keep" />
        <VListItem prepend-icon="mdi-close" title="Remove" @click="reject" />
      </VList>
    </VMenu>
  </VChip>
</template>

<style scoped>
.auto-chip {
  cursor: pointer;
}
</style>
