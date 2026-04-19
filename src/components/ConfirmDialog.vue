<script setup lang="ts">
withDefaults(
  defineProps<{
    modelValue: boolean;
    title: string;
    message?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    confirmColor?: string;
    maxWidth?: number | string;
  }>(),
  {
    message: '',
    confirmLabel: 'Confirm',
    cancelLabel: 'Cancel',
    confirmColor: 'primary',
    maxWidth: 450,
  },
);

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
  confirm: [];
  cancel: [];
}>();

function onCancel(): void {
  emit('update:modelValue', false);
  emit('cancel');
}

function onConfirm(): void {
  emit('update:modelValue', false);
  emit('confirm');
}
</script>

<template>
  <VDialog :model-value="modelValue" :max-width="maxWidth" @update:model-value="(v) => emit('update:modelValue', v)">
    <VCard :title="title">
      <VCardText>
        <div v-if="message" class="mb-2">{{ message }}</div>
        <slot />
      </VCardText>
      <VDivider />
      <VCardActions>
        <VSpacer />
        <VBtn :text="cancelLabel" variant="plain" @click="onCancel" />
        <VBtn :text="confirmLabel" :color="confirmColor" variant="tonal" @click="onConfirm" />
      </VCardActions>
    </VCard>
  </VDialog>
</template>
