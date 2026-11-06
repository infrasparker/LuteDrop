<script setup lang="ts">
import { useAppStore } from '../stores/appStore';
import { storeToRefs } from 'pinia';
import AnyAllToggle from './AnyAllToggle.vue';
import type { MatchMode } from '../utils/moods';

/// Tag chips for filtering, plus Untagged and Has auto tags. With two or more
/// tags selected an any / all switch appears (any by default): any shows
/// entries with one of the tags, all only entries with every one.

const props = defineProps<{
  selectedTags: string[];
  untagged?: boolean;
  hasAuto?: boolean;
  tagMode: MatchMode;
}>();

const emit = defineEmits<{
  'update:selectedTags': [tags: string[]];
  'update:untagged': [value: boolean];
  'update:hasAuto': [value: boolean];
  'update:tagMode': [value: MatchMode];
}>();

const store = useAppStore();
const { tags } = storeToRefs(store);

function toggleTag(tag: string): void {
  if (props.untagged) {
    emit('update:untagged', false);
  }
  const current = props.selectedTags;
  if (current.includes(tag)) {
    const next = current.filter((t) => t !== tag);
    emit('update:selectedTags', next);
    // Back to "any" once the switch hides, so it never hides a non-default state.
    if (next.length < 2 && props.tagMode !== 'any') emit('update:tagMode', 'any');
  } else {
    emit('update:selectedTags', [...current, tag]);
  }
}

function toggleHasAuto(): void {
  emit('update:hasAuto', !props.hasAuto);
}

function toggleUntagged(): void {
  const next = !props.untagged;
  if (next && props.selectedTags.length > 0) {
    emit('update:selectedTags', []);
    if (props.tagMode !== 'any') emit('update:tagMode', 'any');
  }
  emit('update:untagged', next);
}
</script>

<template>
  <div class="d-flex flex-wrap ga-1">
    <VChip
      :color="untagged ? 'primary' : undefined"
      :variant="untagged ? 'flat' : 'outlined'"
      size="small"
      clickable
      prepend-icon="mdi-tag-off"
      @click="toggleUntagged"
    >
      Untagged
    </VChip>
    <VChip
      :color="hasAuto ? 'primary' : undefined"
      :variant="hasAuto ? 'flat' : 'outlined'"
      size="small"
      clickable
      prepend-icon="mdi-creation"
      @click="toggleHasAuto"
    >
      Has auto tags
    </VChip>
    <VChip
      v-for="tag in tags"
      :key="tag"
      :color="selectedTags.includes(tag) ? 'primary' : undefined"
      :variant="selectedTags.includes(tag) ? 'flat' : 'tonal'"
      size="small"
      clickable
      @click="toggleTag(tag)"
    >
      {{ tag }}
    </VChip>
    <AnyAllToggle
      v-if="selectedTags.length >= 2"
      :model-value="tagMode"
      size="x-small"
      kind="tag"
      class="ml-1 align-self-center"
      @update:model-value="emit('update:tagMode', $event)"
    />
    <span v-if="tags.length === 0 && !untagged" class="text-caption text-medium-emphasis align-self-center">No tags yet</span>
  </div>
</template>
