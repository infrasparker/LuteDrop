<script setup lang="ts">
withDefaults(defineProps<{ label: string; title?: string }>(), { title: 'Drag to reorder' });
</script>

<template>
  <button type="button" class="mixer-drag-handle" :aria-label="label" :title="title" @click.stop>
    <VIcon size="18">mdi-drag-vertical</VIcon>
  </button>
</template>
