<script setup lang="ts">
import { useAppStore } from '../stores/appStore';
import { storeToRefs } from 'pinia';

const props = defineProps<{
  selectedTags: string[];
  untagged?: boolean;
  hasAuto?: boolean;
}>();

const emit = defineEmits<{
  'update:selectedTags': [tags: string[]];
  'update:untagged': [value: boolean];
  'update:hasAuto': [value: boolean];
}>();

const store = useAppStore();
const { tags } = storeToRefs(store);

function toggleTag(tag: string): void {
  if (props.untagged) {
    emit('update:untagged', false);
  }
  const current = props.selectedTags;
  if (current.includes(tag)) {
    emit('update:selectedTags', current.filter((t) => t !== tag));
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
    <span v-if="tags.length === 0 && !untagged" class="text-caption text-medium-emphasis align-self-center">No tags yet</span>
  </div>
</template>
