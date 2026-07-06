<script setup lang="ts">
import { ref, watch } from 'vue';

const props = withDefaults(
  defineProps<{
    modelValue: boolean;
    title: string;
    label: string;
    initial?: string;
    confirmLabel?: string;
    maxWidth?: number | string;
  }>(),
  { initial: '', confirmLabel: 'Save', maxWidth: 400 },
);

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
  submit: [value: string];
}>();

const text = ref(props.initial);

// Reset the field to `initial` each time the dialog opens.
watch(
  () => props.modelValue,
  (open) => { if (open) text.value = props.initial; },
);

function onSubmit(): void {
  const trimmed = text.value.trim();
  if (!trimmed) return;
  emit('submit', trimmed);
  emit('update:modelValue', false);
}
</script>

<template>
  <VDialog :model-value="modelValue" :max-width="maxWidth" @update:model-value="(v) => emit('update:modelValue', v)">
    <VCard :title="title">
      <VCardText>
        <VTextField
          v-model="text"
          :label="label"
          variant="outlined"
          autofocus
          @keydown.enter="onSubmit"
        />
      </VCardText>
      <VDivider />
      <VCardActions>
        <VSpacer />
        <VBtn text="Cancel" variant="plain" @click="emit('update:modelValue', false)" />
        <VBtn :text="confirmLabel" color="primary" variant="tonal" @click="onSubmit" />
      </VCardActions>
    </VCard>
  </VDialog>
</template>
