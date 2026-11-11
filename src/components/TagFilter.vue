<script setup lang="ts">
import { useAppStore } from '../stores/appStore';
import { storeToRefs } from 'pinia';
import FilterChip from './FilterChip.vue';
import { chipState, cycleChip, emptyChipSelection, type ChipSelection } from '../utils/moods';

/// Tag chips for filtering, plus Untagged. A tag chip
/// cycles off → include → exclude: an entry passes with at least one
/// included tag (when any is included) and none of the excluded ones.
/// Untagged is a plain on/off constraint on top.

const props = defineProps<{
  selection: ChipSelection;
  untagged?: boolean;
}>();

const emit = defineEmits<{
  'update:selection': [value: ChipSelection];
  'update:untagged': [value: boolean];
}>();

const store = useAppStore();
const { tags } = storeToRefs(store);

function cycleTag(tag: string, direction: 1 | -1): void {
  if (props.untagged) emit('update:untagged', false);
  emit('update:selection', cycleChip(props.selection, tag, direction));
}

function toggleUntagged(): void {
  const next = !props.untagged;
  if (next && (props.selection.include.length > 0 || props.selection.exclude.length > 0)) {
    emit('update:selection', emptyChipSelection());
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
    <FilterChip
      v-for="tag in tags"
      :key="tag"
      :label="tag"
      :state="chipState(selection, tag)"
      @cycle="(direction) => cycleTag(tag, direction)"
    />
    <span v-if="tags.length === 0 && !untagged" class="text-caption text-medium-emphasis align-self-center">No tags yet</span>
  </div>
</template>
