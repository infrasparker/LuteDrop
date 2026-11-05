<script setup lang="ts">
import { MOOD_SQUARES, type MoodSquare } from '../utils/mood';

/// The 3×3 mood map: Bright → Dark down the rows, Airy → Heavy across the
/// columns, each square named with a few words of feel and a scene.
///
/// Used two ways:
/// - as a key (`current` / `auto` set): the track's shown square is
///   highlighted, and the analyzer's square gets a sparkle when it differs;
/// - as a filter (`selectable`): clicking a square toggles it in `selected`.
const props = withDefaults(defineProps<{
  current?: string | null;
  auto?: string | null;
  selected?: string[];
  selectable?: boolean;
}>(), { current: null, auto: null, selected: () => [], selectable: false });

const emit = defineEmits<{ toggle: [key: string] }>();

function isOn(sq: MoodSquare): boolean {
  return props.selectable ? props.selected.includes(sq.key) : props.current === sq.key;
}

function click(sq: MoodSquare): void {
  if (props.selectable) emit('toggle', sq.key);
}
</script>

<template>
  <div class="mood-map">
    <span />
    <span class="axis">Airy</span>
    <span class="axis">Neutral</span>
    <span class="axis">Heavy</span>
    <template v-for="(row, r) in [0, 1, 2]" :key="row">
      <span class="axis axis-row">{{ ['Bright', 'Neutral', 'Dark'][r] }}</span>
      <component
        :is="selectable ? 'button' : 'div'"
        v-for="sq in MOOD_SQUARES.slice(r * 3, r * 3 + 3)"
        :key="sq.key"
        :type="selectable ? 'button' : undefined"
        class="square"
        :class="{ on: isOn(sq), clickable: selectable }"
        :aria-pressed="selectable ? isOn(sq) : undefined"
        @click="click(sq)"
      >
        <span class="name">
          {{ sq.name }}
          <VIcon
            v-if="!selectable && auto === sq.key && auto !== current"
            size="x-small"
            title="The analyzer would put this track here"
          >mdi-creation</VIcon>
          <VIcon v-if="!selectable && current === sq.key" size="x-small">mdi-map-marker</VIcon>
        </span>
        <span class="feel">{{ sq.feel }}</span>
        <span class="scene">{{ sq.scene }}</span>
      </component>
    </template>
  </div>
</template>

<style scoped>
.mood-map {
  display: grid;
  grid-template-columns: auto repeat(3, minmax(0, 150px));
  gap: 4px;
  align-items: stretch;
}
.axis {
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  opacity: 0.6;
  text-align: center;
  align-self: end;
}
.axis-row {
  align-self: center;
  text-align: right;
  padding-right: 4px;
}
.square {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 8px;
  border: 1px solid rgba(var(--v-border-color), var(--v-border-opacity));
  border-radius: 6px;
  text-align: left;
  font: inherit;
  color: inherit;
  background: transparent;
}
.square.clickable {
  cursor: pointer;
}
.square.clickable:hover {
  background: rgba(var(--v-theme-on-surface), 0.06);
}
.square.on {
  border-color: rgb(var(--v-theme-primary));
  background: rgba(var(--v-theme-primary), 0.14);
}
.name {
  font-weight: 600;
  font-size: 0.85rem;
  display: flex;
  align-items: center;
  gap: 4px;
}
.feel {
  font-size: 0.72rem;
  opacity: 0.85;
}
.scene {
  font-size: 0.7rem;
  font-style: italic;
  opacity: 0.6;
}
</style>
