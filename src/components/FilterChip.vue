<script setup lang="ts">
import { computed } from 'vue';
import type { ChipState } from '../utils/moods';

/// Energy chips toggle on/off. Tags cycle include/exclude/off; moods use
/// the same cycle for high/low/off, with direction arrows instead of exclusion
/// styling. Right-click cycles backwards. Accessible labels describe the state.
const props = defineProps<{
  label: string;
  /// High/low mood preference rather than include/exclude tags.
  polarity?: boolean;
  state: ChipState;
  size?: 'x-small' | 'small';
  /// The chip's look when off ('tonal' for tags, 'outlined' for lenses).
  offVariant?: 'tonal' | 'outlined';
  /// A plain on/off chip (no excluded state).
  toggle?: boolean;
  /// What the chip means, put before the click hint in the hover text.
  detail?: string;
}>();

const emit = defineEmits<{ cycle: [direction: 1 | -1] }>();

const ariaLabel = computed(() => {
  if (props.polarity) return `${props.label}, ${props.state === 'include' ? 'high' : props.state === 'exclude' ? 'low' : 'off'}`;
  if (props.toggle) return props.state === 'include' ? `${props.label}, on` : props.label;
  if (props.state === 'include') return `${props.label}, included`;
  if (props.state === 'exclude') return `${props.label}, excluded`;
  return props.label;
});

const hint = computed(() => {
  let text: string;
  if (props.polarity) text = `${props.state === 'include' ? 'High' : props.state === 'exclude' ? 'Low' : 'Off'}. Click: off, high, low. Right-click: reverse` ;
  else if (props.toggle) text = props.state === 'include' ? 'On. Click to turn off' : 'Click to turn on';
  else if (props.state === 'include') text = 'Included. Click to exclude';
  else if (props.state === 'exclude') text = 'Excluded. Click to clear';
  else text = 'Click to include';
  if (!props.toggle && !props.polarity) text += '. Right-click to cycle backwards';
  return props.detail ? `${props.detail}. ${text}` : text;
});

const ariaPressed = computed(() => {
  if (props.state === 'include') return 'true';
  return props.state === 'exclude' && !props.toggle ? 'mixed' : 'false';
});
</script>

<template>
  <VChip
    role="button"
    :color="state === 'include' ? 'primary' : state === 'exclude' ? (polarity ? 'secondary' : 'error') : undefined"
    :variant="state === 'include' ? 'flat' : state === 'exclude' ? 'outlined' : offVariant ?? 'tonal'"
    :size="size ?? 'small'"
    :class="{ 'is-excluded': state === 'exclude' && !polarity }"
    :aria-pressed="ariaPressed"
    :aria-label="ariaLabel"
    :title="hint"
    :data-state="state"
    clickable
    @click="emit('cycle', 1)"
    @contextmenu.prevent="emit('cycle', -1)"
  >
    <span v-if="polarity && state !== 'off'" class="mr-1" aria-hidden="true">{{ state === 'include' ? '↑' : '↓' }}</span><span class="chip-label">{{ label }}</span>
  </VChip>
</template>

<style scoped>
.is-excluded .chip-label {
  text-decoration: line-through;
}
</style>
