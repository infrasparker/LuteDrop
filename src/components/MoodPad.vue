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
  type MoodPlace,
  type PadColumn,
} from '../utils/mood';

/// The mood pad, built like a colour picker: a size switch picks the floor
/// (Intimate / Moderate / Grand), and a pad below it crosses energy (rows,
/// Energetic on top) with brightness (columns). Dark is split into two
/// sub-columns, Sad and Scary, under a shared Dark header, so the pad is
/// 4 columns: Sad, Scary, Neutral, Bright. Each cell names its mood with a
/// few words of feel and a scene. Dark names do not change with size.
///
/// Two modes:
/// - `pick`: `current` is pinned and `auto` gets a sparkle when it differs;
///   each shows only on its own floor, and the size buttons mark which floor
///   holds them. A Dark place with no kind yet is marked on the Dark header.
///   Clicking a cell emits `pick`.
/// - `filter`: multi-select into `selected`. The switch adds "All sizes",
///   where a click acts on all three floors at once. Row and column headers
///   toggle their whole line (the Dark header toggles both of its
///   sub-columns); each cell shows how many entries sit in it.
export type PadSize = MoodStep | 'all';

const props = withDefaults(defineProps<{
  mode: 'pick' | 'filter';
  current?: MoodPlace | null;
  auto?: MoodPlace | null;
  selected?: string[];
  counts?: Map<string, number>;
}>(), { current: null, auto: null, selected: () => [], counts: () => new Map() });

const emit = defineEmits<{ pick: [cell: MoodCell]; 'update:selected': [keys: string[]] }>();

const size = defineModel<PadSize>('size', { required: true });

const isFilter = computed(() => props.mode === 'filter');
const floors = computed<MoodStep[]>(() => (size.value === 'all' ? PAD_SIZES : [size.value]));
const autoPlace = computed(() => (props.auto && props.auto.key !== props.current?.key ? props.auto : null));
const selectedSet = computed(() => new Set(props.selected));

const darkColumns = PAD_COLUMNS.filter((c) => c.kind !== null);
const otherColumns = PAD_COLUMNS.filter((c) => c.kind === null);

/// True when the place is Dark with no kind and sits on the floor in view:
/// it is marked on the Dark header instead of a cell.
const onDarkHeader = (p: MoodPlace | null): boolean => p !== null && p.cell === null && floors.value.includes(p.grand);

/// The cells one pad position stands for on the floor(s) in view.
function cellsAt(energy: MoodStep, col: PadColumn): MoodCell[] {
  return floors.value.map((g) => cellFor(g, energy, col.bright, col.kind)).filter((c): c is MoodCell => c !== null);
}

const keysOf = (cells: MoodCell[]): string[] => cells.map((c) => c.key);
const rowKeys = (energy: MoodStep): string[] => PAD_COLUMNS.flatMap((col) => keysOf(cellsAt(energy, col)));
const columnKeys = (cols: PadColumn[]): string[] =>
  cols.flatMap((col) => PAD_ROWS.flatMap((e) => keysOf(cellsAt(e, col))));

/// The cell's name; the dark names are the same on every floor, so All sizes shows them once.
const namesOf = (cells: MoodCell[]): string => [...new Set(cells.map((c) => c.name))].join(' · ');
const oneName = (cells: MoodCell[]): boolean => new Set(cells.map((c) => c.name)).size === 1;

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
  return isFilter.value ? selection(keysOf(cells)) === 'on' : cells.some((c) => c.key === props.current?.key);
}

function click(cells: MoodCell[]): void {
  if (isFilter.value) toggleKeys(keysOf(cells));
  else if (cells.length === 1) emit('pick', cells[0]);
}

/// Classes for a header; in filter mode it shows whether its whole line is selected.
function headerClass(keys: string[]): Record<string, boolean> {
  return isFilter.value ? { clickable: true, [selection(keys)]: true } : {};
}

/// A header click toggles its whole line (filter mode only).
function headerClick(keys: string[]): void {
  if (isFilter.value) toggleKeys(keys);
}

const headerTag = computed(() => (isFilter.value ? 'button' : 'span'));
const headerType = computed(() => (isFilter.value ? 'button' : undefined));
</script>

<template>
  <div class="mood-pad">
    <VBtnToggle v-model="size" mandatory density="compact" variant="outlined" divided class="mb-2">
      <VBtn v-for="floor in PAD_SIZES" :key="floor" :value="floor" size="small">
        {{ stepLabel('grand', floor) }}
        <template v-if="!isFilter">
          <VIcon v-if="autoPlace?.grand === floor" size="x-small" class="ml-1" title="The analyzer's mood is on this size">
            mdi-creation
          </VIcon>
          <VIcon v-if="current?.grand === floor" size="x-small" class="ml-1" title="The track's mood is on this size">
            mdi-map-marker
          </VIcon>
        </template>
        <VBadge v-else-if="floorCount(floor) > 0" :content="floorCount(floor)" color="primary" inline />
      </VBtn>
      <VBtn v-if="isFilter" value="all" size="small">All sizes</VBtn>
    </VBtnToggle>

    <div class="pad-grid">
      <!-- Two header rows: Dark spans its Sad and Scary sub-columns; Neutral and Bright span both rows. -->
      <span class="corner" />
      <component
        :is="headerTag"
        :type="headerType"
        class="axis dark-header"
        :class="headerClass(columnKeys(darkColumns))"
        :title="isFilter ? 'Toggle every Dark mood' : undefined"
        @click="headerClick(columnKeys(darkColumns))"
      >
        Dark
        <template v-if="!isFilter">
          <VIcon v-if="onDarkHeader(autoPlace)" size="x-small" title="The analyzer says Dark, with no Sad or Scary yet">
            mdi-creation
          </VIcon>
          <VIcon v-if="onDarkHeader(current)" size="x-small" title="This track is Dark, with no Sad or Scary yet">
            mdi-map-marker
          </VIcon>
        </template>
      </component>
      <component
        :is="headerTag"
        v-for="col in otherColumns"
        :key="`col-${col.label}`"
        :type="headerType"
        class="axis tall-header"
        :class="headerClass(columnKeys([col]))"
        :title="isFilter ? `Toggle every ${col.label} mood` : undefined"
        @click="headerClick(columnKeys([col]))"
      >
        {{ col.label }}
      </component>
      <component
        :is="headerTag"
        v-for="col in darkColumns"
        :key="`col-${col.label}`"
        :type="headerType"
        class="axis"
        :class="headerClass(columnKeys([col]))"
        :title="isFilter ? `Toggle every ${col.label} mood` : undefined"
        @click="headerClick(columnKeys([col]))"
      >
        {{ col.label }}
      </component>

      <template v-for="energy in PAD_ROWS" :key="`row-${energy}`">
        <component
          :is="headerTag"
          :type="headerType"
          class="axis axis-row"
          :class="headerClass(rowKeys(energy))"
          :title="isFilter ? `Toggle every ${stepLabel('energy', energy)} mood` : undefined"
          @click="headerClick(rowKeys(energy))"
        >
          {{ stepLabel('energy', energy) }}
        </component>
        <button
          v-for="col in PAD_COLUMNS"
          :key="`${energy}-${col.label}`"
          type="button"
          class="cell"
          :class="{
            on: isOn(cellsAt(energy, col)),
            partial: isFilter && selection(keysOf(cellsAt(energy, col))) === 'partial',
          }"
          :aria-pressed="isFilter ? isOn(cellsAt(energy, col)) : undefined"
          @click="click(cellsAt(energy, col))"
        >
          <span class="name">
            <span class="name-text">{{ namesOf(cellsAt(energy, col)) }}</span>
            <template v-if="!isFilter">
              <VIcon
                v-if="cellsAt(energy, col).some((c) => c.key === autoPlace?.key)"
                size="x-small"
                title="The analyzer would put this track here"
              >mdi-creation</VIcon>
              <VIcon v-if="cellsAt(energy, col).some((c) => c.key === current?.key)" size="x-small">mdi-map-marker</VIcon>
            </template>
            <span v-else class="count" :title="`${entryCount(cellsAt(energy, col))} entries here`">
              {{ entryCount(cellsAt(energy, col)) }}
            </span>
          </span>
          <template v-if="oneName(cellsAt(energy, col))">
            <span class="feel">{{ cellsAt(energy, col)[0]?.feel }}</span>
            <span class="scene">{{ cellsAt(energy, col)[0]?.scene }}</span>
          </template>
          <span v-else class="feel">{{ stepLabel('energy', energy) }}, {{ col.label.toLowerCase() }}</span>
        </button>
      </template>
    </div>
  </div>
</template>

<style scoped>
.pad-grid {
  display: grid;
  grid-template-columns: auto repeat(4, minmax(0, 140px));
  gap: 4px;
  align-items: stretch;
}
.corner {
  grid-row: span 2;
}
.dark-header {
  grid-column: span 2;
  border-bottom: 1px solid rgba(var(--v-border-color), var(--v-border-opacity));
  border-radius: 4px 4px 0 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 4px;
}
.tall-header {
  grid-row: span 2;
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
