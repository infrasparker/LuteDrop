<script setup lang="ts">
// The Find similar button in a track's clip header, with a tooltip explaining
// why it is disabled.
defineProps<{ disabled: boolean }>();
defineEmits<{ click: [] }>();
</script>

<template>
  <VTooltip :disabled="!disabled" text="No music clips to find similar tracks for" location="top">
    <template #activator="{ props: tipProps }">
      <span v-bind="tipProps" class="mr-2">
        <VBtn
          prepend-icon="mdi-compass-outline"
          variant="tonal"
          rounded="xl"
          :disabled="disabled"
          @click="$emit('click')"
        >
          Find similar
        </VBtn>
      </span>
    </template>
  </VTooltip>
</template>
