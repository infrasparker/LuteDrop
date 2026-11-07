<script setup lang="ts">
/// One tag on a library entry. With `closable`, ✕ asks the parent to remove
/// it from the list (and save the list).
defineProps<{
  tag: string;
  closable?: boolean;
}>();

const emit = defineEmits<{
  remove: [];
}>();
</script>

<template>
  <VChip
    size="small"
    :closable="closable"
    close-label="Remove tag"
    @click:close.stop="emit('remove')"
  >
    {{ tag }}
  </VChip>
</template>
