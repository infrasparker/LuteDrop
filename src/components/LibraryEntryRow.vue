<script setup lang="ts">
import type { LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';

const props = defineProps<{
  entry: LibraryEntry;
  selectable?: boolean;
  selected?: boolean;
}>();

const emit = defineEmits<{
  select: [entryId: string];
}>();

const store = useAppStore();

function formatDuration(seconds: number | null): string {
  if (seconds === null) return '--:--';
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

function handleClick(): void {
  if (props.selectable) {
    emit('select', props.entry.id);
  }
}
</script>

<template>
  <VListItem
    :class="[
      selected ? 'bg-primary-lighten-4' : '',
      store.isEntryMissing(entry.id) ? 'opacity-50' : '',
      selectable ? 'cursor-pointer' : '',
    ]"
    rounded
    @click="handleClick"
  >
    <template #prepend>
      <VIcon v-if="store.isEntryMissing(entry.id)" color="warning" class="mr-2">mdi-alert</VIcon>
      <VIcon v-else class="mr-2">mdi-music-note</VIcon>
    </template>

    <VListItemTitle class="font-weight-medium">{{ entry.name }}</VListItemTitle>

    <VListItemSubtitle>
      <span class="mr-3">{{ formatDuration(entry.duration) }}</span>
      <VChip
        v-for="tag in entry.tags"
        :key="tag"
        size="x-small"
        class="mr-1"
        variant="outlined"
      >
        {{ tag }}
      </VChip>
    </VListItemSubtitle>

    <template #append>
      <slot name="actions" />
    </template>
  </VListItem>
</template>
