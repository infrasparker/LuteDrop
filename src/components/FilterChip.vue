<script setup lang="ts">
import { computed } from 'vue';
import type { ChipState } from '../utils/moods';

/// A filter chip. With `toggle` it is a plain on/off chip (mood lenses and
/// Energy bands): `state` is 'include' when on, 'off' when off. Without it it
/// is tri-state (tags): off → include → exclude → off on each click.
/// On/included is filled in the accent colour; excluded is a red outline with
/// the label struck through. The state is also in aria-pressed and the
/// aria-label, so it does not rely on colour alone.
const props = defineProps<{
  label: string;
  state: ChipState;
  size?: 'x-small' | 'small';
  /// The chip's look when off ('tonal' for tags, 'outlined' for lenses).
  offVariant?: 'tonal' | 'outlined';
  /// A plain on/off chip (no excluded state).
  toggle?: boolean;
  /// What the chip means, put before the click hint in the hover text.
  detail?: string;
}>();

const emit = defineEmits<{ cycle: [] }>();

const ariaLabel = computed(() => {
  if (props.toggle) return props.state === 'include' ? `${props.label}, on` : props.label;
  if (props.state === 'include') return `${props.label}, included`;
  if (props.state === 'exclude') return `${props.label}, excluded`;
  return props.label;
});

const hint = computed(() => {
  let text: string;
  if (props.toggle) text = props.state === 'include' ? 'On. Click to turn off' : 'Click to turn on';
  else if (props.state === 'include') text = 'Included. Click to exclude';
  else if (props.state === 'exclude') text = 'Excluded. Click to clear';
  else text = 'Click to include';
  return props.detail ? `${props.detail}. ${text}` : text;
});

const ariaPressed = computed(() => {
  if (props.state === 'include') return 'true';
  return props.state === 'exclude' && !props.toggle ? 'mixed' : 'false';
});
</script>

<template>
  <VChip
    :color="state === 'include' ? 'primary' : state === 'exclude' ? 'error' : undefined"
    :variant="state === 'include' ? 'flat' : state === 'exclude' ? 'outlined' : offVariant ?? 'tonal'"
    :size="size ?? 'small'"
    :class="{ 'is-excluded': state === 'exclude' }"
    :aria-pressed="ariaPressed"
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
