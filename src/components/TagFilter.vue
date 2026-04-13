<script setup lang="ts">
import { useAppStore } from '../stores/appStore';
import { storeToRefs } from 'pinia';

const props = defineProps<{
  selectedTags: string[];
}>();

const emit = defineEmits<{
  'update:selectedTags': [tags: string[]];
}>();

const store = useAppStore();
const { tags } = storeToRefs(store);

function toggleTag(tag: string): void {
  const current = props.selectedTags;
  if (current.includes(tag)) {
    emit('update:selectedTags', current.filter((t) => t !== tag));
  } else {
    emit('update:selectedTags', [...current, tag]);
  }
}
</script>

<template>
  <div class="d-flex flex-wrap gap-1">
    <VChip
      v-for="tag in tags"
      :key="tag"
      :color="selectedTags.includes(tag) ? 'primary' : undefined"
      :variant="selectedTags.includes(tag) ? 'flat' : 'outlined'"
      size="small"
      clickable
      @click="toggleTag(tag)"
    >
      {{ tag }}
    </VChip>
    <span v-if="tags.length === 0" class="text-caption text-medium-emphasis">No tags yet</span>
  </div>
</template>
