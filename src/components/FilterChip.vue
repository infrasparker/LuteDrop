<script setup lang="ts">
import { computed } from 'vue';
import type { ChipState } from '../utils/moods';

/// A tri-state filter chip: off → include → exclude → off on each click.
/// Included is filled in the accent colour; excluded is a red outline with
/// the label struck through. The state is also in aria-pressed and the
/// aria-label, so it does not rely on colour alone.
const props = defineProps<{
  label: string;
  state: ChipState;
  size?: 'x-small' | 'small';
  /// The chip's look when off ('tonal' for tags, 'outlined' for lenses).
  offVariant?: 'tonal' | 'outlined';
}>();

const emit = defineEmits<{ cycle: [] }>();

const ariaLabel = computed(() => {
  if (props.state === 'include') return `${props.label}, included`;
  if (props.state === 'exclude') return `${props.label}, excluded`;
  return props.label;
});

const hint = computed(() => {
  if (props.state === 'include') return 'Included. Click to exclude';
  if (props.state === 'exclude') return 'Excluded. Click to clear';
  return 'Click to include';
});
</script>

<template>
  <VChip
    :color="state === 'include' ? 'primary' : state === 'exclude' ? 'error' : undefined"
    :variant="state === 'include' ? 'flat' : state === 'exclude' ? 'outlined' : offVariant ?? 'tonal'"
    :size="size ?? 'small'"
    :class="{ 'is-excluded': state === 'exclude' }"
    :aria-pressed="state === 'include' ? 'true' : state === 'exclude' ? 'mixed' : 'false'"
    :aria-label="ariaLabel"
    :title="hint"
    :data-state="state"
    clickable
    @click="emit('cycle')"
  >
    <span class="chip-label">{{ label }}</span>
  </VChip>
</template>

<style scoped>
.is-excluded .chip-label {
  text-decoration: line-through;
}
</style>
