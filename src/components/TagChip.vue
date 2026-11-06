<script setup lang="ts">
import { computed } from 'vue';
import type { AutoTag } from '@generated/index';
import { useAppStore } from '../stores/appStore';

/// One tag on a library entry. A confirmed tag is a plain chip. A suggested
/// tag (shown only because the analyzer suggested it) is outlined with a
/// sparkle and a score tooltip, and offers ✓ (vote yes: confirm it) next to its ✕.
/// ✕ on any chip votes no, so later analysis runs don't bring it back: with
/// `closable` the parent removes it from the list (and saves the list), else
/// an auto chip rejects itself directly. The auto chip sets `closable` to false
/// explicitly: inside a VCombobox with `closable-chips`, Vuetify would otherwise
/// add its own close button, which hides the chip without saving anything.
const props = withDefaults(defineProps<{
  tag: string;
  auto?: AutoTag | null;
  entryId: string;
  closable?: boolean;
  /** Show the ✓ button on auto chips. */
  confirmable?: boolean;
}>(), { auto: null, confirmable: true });

const emit = defineEmits<{
  remove: [];
}>();

const store = useAppStore();

const scoreLabel = computed(() => (props.auto ? `Suggested tag · ${props.auto.score.toFixed(2)}` : ''));

function keep(): void {
  store.command('keep_auto_tags', { entryIds: [props.entryId], tags: [props.tag] });
}

function reject(): void {
  if (props.closable) emit('remove');
  else store.command('reject_auto_tags', { entryIds: [props.entryId], tags: [props.tag] });
}
</script>

<template>
  <VChip
    v-if="!auto"
    size="small"
    :closable="closable"
    close-label="Remove tag (vote no)"
    @click:close.stop="emit('remove')"
  >
    {{ tag }}
  </VChip>
  <VChip
    v-else
    size="small"
    variant="outlined"
    prepend-icon="mdi-creation"
    :closable="false"
    @click.stop
  >
    {{ tag }}
    <VTooltip activator="parent" location="top" open-delay="300">{{ scoreLabel }}</VTooltip>
    <VIcon
      v-if="confirmable"
      size="small"
      class="chip-action ml-1"
      title="Yes: confirm this tag"
      @mousedown.stop.prevent
      @click.stop="keep"
    >mdi-check</VIcon>
    <VIcon
      size="small"
      class="chip-action"
      title="No: remove this tag"
      @mousedown.stop.prevent
      @click.stop="reject"
    >mdi-close</VIcon>
  </VChip>
</template>

<style scoped>
.chip-action {
  cursor: pointer;
  opacity: 0.7;
}
.chip-action:hover {
  opacity: 1;
}
</style>
