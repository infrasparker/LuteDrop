<script setup lang="ts">
import { computed, ref, watch } from 'vue';

const props = withDefaults(
  defineProps<{
    modelValue: boolean;
    title: string;
    label: string;
    initial?: string;
    confirmLabel?: string;
    maxWidth?: number | string;
    /// Says what is wrong with the text (shown under the field, and the
    /// dialog stays open), or null when it is fine.
    validate?: (text: string) => string | null;
  }>(),
  { initial: '', confirmLabel: 'Save', maxWidth: 400, validate: undefined },
);

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
  submit: [value: string];
}>();

const text = ref(props.initial);
const problem = computed(() => (props.validate && text.value.trim() ? props.validate(text.value) : null));

// Reset the field to `initial` each time the dialog opens.
watch(
  () => props.modelValue,
  (open) => { if (open) text.value = props.initial; },
);

function onSubmit(): void {
  const trimmed = text.value.trim();
  if (!trimmed || problem.value) return;
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
          :error-messages="problem ?? undefined"
          @keydown.enter="onSubmit"
        />
      </VCardText>
      <VDivider />
      <VCardActions>
        <VSpacer />
        <VBtn text="Cancel" variant="plain" @click="emit('update:modelValue', false)" />
        <VBtn :text="confirmLabel" color="primary" variant="tonal" :disabled="problem !== null" @click="onSubmit" />
      </VCardActions>
    </VCard>
  </VDialog>
</template>
