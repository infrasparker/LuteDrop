<script setup lang="ts">
import { computed } from 'vue';
import type { MoodStep } from '@generated/index';
import {
  cellByKey,
  cellFor,
  PAD_COLUMNS,
  PAD_ROWS,
  PAD_SIZES,
  stepLabel,
  type MoodCell,
} from '../utils/mood';

/// The mood pad, built like a colour picker: a size switch picks the floor
/// (Intimate / Moderate / Grand), and a 3×3 pad below it crosses energy
/// (rows, Energetic on top) with brightness (columns, Dark → Bright). Each
/// cell names its mood with a few words of feel and a scene.
///
/// Two modes:
/// - `pick`: `current` is pinned and `auto` gets a sparkle when it differs;
///   each shows only on its own floor, and the size buttons mark which floor
///   holds them. Clicking a cell emits `pick`.
/// - `filter`: multi-select into `selected`. The switch adds "All sizes",
///   where a click acts on all three floors at once. Row and column headers
///   toggle their whole line; each cell shows how many entries sit in it.
export type PadSize = MoodStep | 'all';

const props = withDefaults(defineProps<{
  mode: 'pick' | 'filter';
  current?: string | null;
  auto?: string | null;
  selected?: string[];
  counts?: Map<string, number>;
}>(), { current: null, auto: null, selected: () => [], counts: () => new Map() });

const emit = defineEmits<{ pick: [cell: MoodCell]; 'update:selected': [keys: string[]] }>();

const size = defineModel<PadSize>('size', { required: true });

const isFilter = computed(() => props.mode === 'filter');
const floors = computed<MoodStep[]>(() => (size.value === 'all' ? PAD_SIZES : [size.value]));
const currentCell = computed(() => (props.current ? cellByKey(props.current) : null));
const autoCell = computed(() => {
  const c = props.auto ? cellByKey(props.auto) : null;
  return c && c.key !== props.current ? c : null;
});
const selectedSet = computed(() => new Set(props.selected));

/// The cells one pad position stands for on the floor(s) in view.
function cellsAt(energy: MoodStep, bright: MoodStep): MoodCell[] {
  return floors.value.map((g) => cellFor(g, energy, bright)).filter((c): c is MoodCell => c !== null);
}

const keysOf = (cells: MoodCell[]): string[] => cells.map((c) => c.key);
const rowKeys = (energy: MoodStep): string[] => PAD_COLUMNS.flatMap((b) => keysOf(cellsAt(energy, b)));
const columnKeys = (bright: MoodStep): string[] => PAD_ROWS.flatMap((e) => keysOf(cellsAt(e, bright)));

/// Selects every key when any is unselected, else clears them all.
function toggleKeys(keys: string[]): void {
  const sel = selectedSet.value;
  const allOn = keys.every((k) => sel.has(k));
  emit('update:selected', allOn
    ? props.selected.filter((k) => !keys.includes(k))
    : [...props.selected, ...keys.filter((k) => !sel.has(k))]);
}

function selection(keys: string[]): 'on' | 'partial' | 'off' {
  const n = keys.filter((k) => selectedSet.value.has(k)).length;
  if (n === 0) return 'off';
  return n === keys.length ? 'on' : 'partial';
}

function floorCount(floor: MoodStep): number {
  return props.selected.filter((k) => cellByKey(k)?.grand === floor).length;
}

function entryCount(cells: MoodCell[]): number {
  return cells.reduce((sum, c) => sum + (props.counts.get(c.key) ?? 0), 0);
}

function isOn(cells: MoodCell[]): boolean {
  return isFilter.value ? selection(keysOf(cells)) === 'on' : cells.some((c) => c.key === props.current);
}

function click(cells: MoodCell[]): void {
  if (isFilter.value) toggleKeys(keysOf(cells));
  else if (cells.length === 1) emit('pick', cells[0]);
}
</script>

<template>
  <div class="mood-pad">
    <VBtnToggle v-model="size" mandatory density="compact" variant="outlined" divided class="mb-2">
      <VBtn v-for="floor in PAD_SIZES" :key="floor" :value="floor" size="small">
        {{ stepLabel('grand', floor) }}
        <template v-if="!isFilter">
          <VIcon v-if="autoCell?.grand === floor" size="x-small" class="ml-1" title="The analyzer's mood is on this size">
            mdi-creation
          </VIcon>
          <VIcon v-if="currentCell?.grand === floor" size="x-small" class="ml-1" title="The track's mood is on this size">
            mdi-map-marker
          </VIcon>
        </template>
        <VBadge v-else-if="floorCount(floor) > 0" :content="floorCount(floor)" color="primary" inline />
      </VBtn>
      <VBtn v-if="isFilter" value="all" size="small">All sizes</VBtn>
    </VBtnToggle>

    <div class="pad-grid">
      <span />
      <component
        :is="isFilter ? 'button' : 'span'"
        v-for="bright in PAD_COLUMNS"
        :key="`col-${bright}`"
        :type="isFilter ? 'button' : undefined"
        class="axis"
        :class="{ clickable: isFilter, [selection(columnKeys(bright))]: isFilter }"
        :title="isFilter ? `Toggle every ${stepLabel('bright', bright)} mood` : undefined"
        @click="isFilter && toggleKeys(columnKeys(bright))"
      >
        {{ stepLabel('bright', bright) }}
      </component>

      <template v-for="energy in PAD_ROWS" :key="`row-${energy}`">
        <component
          :is="isFilter ? 'button' : 'span'"
          :type="isFilter ? 'button' : undefined"
          class="axis axis-row"
          :class="{ clickable: isFilter, [selection(rowKeys(energy))]: isFilter }"
          :title="isFilter ? `Toggle every ${stepLabel('energy', energy)} mood` : undefined"
          @click="isFilter && toggleKeys(rowKeys(energy))"
        >
          {{ stepLabel('energy', energy) }}
        </component>
        <button
          v-for="bright in PAD_COLUMNS"
          :key="`${energy}-${bright}`"
          type="button"
          class="cell"
          :class="{
            on: isOn(cellsAt(energy, bright)),
            partial: isFilter && selection(keysOf(cellsAt(energy, bright))) === 'partial',
          }"
          :aria-pressed="isFilter ? isOn(cellsAt(energy, bright)) : undefined"
          @click="click(cellsAt(energy, bright))"
        >
          <span class="name">
            <span class="name-text">{{ cellsAt(energy, bright).map((c) => c.name).join(' · ') }}</span>
            <template v-if="!isFilter">
              <VIcon
                v-if="cellsAt(energy, bright).some((c) => c.key === autoCell?.key)"
                size="x-small"
                title="The analyzer would put this track here"
              >mdi-creation</VIcon>
              <VIcon v-if="cellsAt(energy, bright).some((c) => c.key === current)" size="x-small">mdi-map-marker</VIcon>
            </template>
            <span v-else class="count" :title="`${entryCount(cellsAt(energy, bright))} entries here`">
              {{ entryCount(cellsAt(energy, bright)) }}
            </span>
          </span>
          <template v-if="size !== 'all'">
            <span class="feel">{{ cellsAt(energy, bright)[0]?.feel }}</span>
            <span class="scene">{{ cellsAt(energy, bright)[0]?.scene }}</span>
          </template>
          <span v-else class="feel">{{ stepLabel('energy', energy) }}, {{ stepLabel('bright', bright).toLowerCase() }}</span>
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.pad-grid {
  display: grid;
  grid-template-columns: auto repeat(3, minmax(0, 150px));
  gap: 4px;
  align-items: stretch;
}
.axis {
  font: inherit;
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  opacity: 0.6;
  text-align: center;
  align-self: end;
  color: inherit;
  background: transparent;
  border: none;
  border-radius: 4px;
  padding: 2px 4px;
}
.axis-row {
  align-self: center;
  text-align: right;
}
.axis.clickable {
  cursor: pointer;
}
.axis.clickable:hover {
  opacity: 1;
  background: rgba(var(--v-theme-on-surface), 0.06);
}
.axis.on {
  opacity: 1;
  color: rgb(var(--v-theme-primary));
}
.axis.partial {
  opacity: 0.85;
}
.cell {
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
  cursor: pointer;
}
.cell:hover {
  background: rgba(var(--v-theme-on-surface), 0.06);
}
.cell.on {
  border-color: rgb(var(--v-theme-primary));
  background: rgba(var(--v-theme-primary), 0.14);
}
/* Selected on some floors but not all (All sizes view). */
.cell.partial {
  border-color: rgb(var(--v-theme-primary));
  border-style: dashed;
}
.name {
  font-weight: 600;
  font-size: 0.85rem;
  display: flex;
  align-items: center;
  gap: 4px;
}
.name-text {
  flex: 1 1 auto;
  min-width: 0;
}
.count {
  font-weight: 400;
  font-size: 0.7rem;
  opacity: 0.6;
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
