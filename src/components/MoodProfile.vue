<script setup lang="ts">
import { computed, reactive } from 'vue';
import type { FlagBand, FlagKind, LibraryEntry, MoodFlag, MoodId } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { flagControlVisible, flagLabel, FLAG_BANDS } from '../utils/moodFlags';
import { MOOD_LEVELS, moodLayers, moodList, snapLevel, stepLevel } from '../utils/moods';

/// The mood profile of one or more music entries: one row per mood of the
/// vocabulary, then Energy, each a bar from 0 to 1 with a tick at every level.
///
/// - The bar shows the value the user set, else the analyzer's suggestion.
///   A bar the user set is drawn in the accent colour; a suggested one is
///   muted. Hovering names the exact suggestion.
/// - Clicking the bar sets the level nearest to the click. Arrow keys move a
///   focused bar one level; Home / End go to 0 / 1; Delete goes back to the
///   suggestion, as does the ↺ button.
/// - With several entries (bulk), a click sets the level on every one, and a
///   row whose entries differ shows "mixed".
/// - With one entry, a flag button next to each mood bar says the bar looks
///   wrong (a band it belongs in, or higher / lower); the analyzer learns
///   from flags when the moods are tuned. A flagged bar shows a flag marker.
/// Every change saves at once.
const props = defineProps<{ entries: LibraryEntry[] }>();

const store = useAppStore();

// Moods only exist on music; ambiance in a bulk selection is left out.
const music = computed(() => props.entries.filter((e) => e.entryType === 'music'));
const ids = computed(() => music.value.map((e) => e.id));

// A level clicked but not saved yet ('clear' = back to the suggestion), so
// the bar answers at once and quick key presses build on each other. Keyed
// by the entries and the mood, so a save still running for other entries
// never shows on the ones displayed now.
const pending = reactive(new Map<string, number | 'clear'>());
// Saves run one after another per entries and mood, so the last one wins.
const chains = new Map<string, Promise<unknown>>();
const slot = (entryIds: string[], mood: MoodId): string => `${entryIds.join(',')}|${mood}`;

type SetState = 'suggested' | 'set' | 'mixed';

interface Row {
  id: MoodId;
  name: string;
  /// The value the bar shows, or null when there is none (not analyzed, or mixed).
  value: number | null;
  /// Lowest and highest shown value across the entries (equal for one entry).
  min: number | null;
  max: number | null;
  mixed: boolean;
  setState: SetState;
  hint: string;
  /// Whether the flag button shows (one song, not Energy).
  flaggable: boolean;
  /// The flag on this song and mood, if any.
  flag: MoodFlag | null;
}

const fmt = (v: number): string => v.toFixed(2);

function range(values: number[]): string {
  if (values.length === 0) return '';
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  return lo === hi ? fmt(lo) : `${fmt(lo)}–${fmt(hi)}`;
}

const rows = computed<Row[]>(() =>
  moodList().map(({ id, name }) => {
    const p = pending.get(slot(ids.value, id));
    const layers = music.value.map((e) => {
      const { auto, user } = moodLayers(e, id);
      if (p === 'clear') return { auto, user: null };
      if (p !== undefined) return { auto, user: p };
      return { auto, user };
    });
    const shown = layers.map((l) => l.user ?? l.auto);
    const known = shown.filter((v): v is number => v !== null);
    const userCount = layers.filter((l) => l.user !== null).length;
    const autos = layers.map((l) => l.auto).filter((v): v is number => v !== null);
    const min = known.length > 0 ? Math.min(...known) : null;
    const max = known.length > 0 ? Math.max(...known) : null;
    const mixed = known.length > 0 && (min !== max || known.length !== shown.length);
    const setState: SetState = userCount === 0 ? 'suggested' : userCount === layers.length ? 'set' : 'mixed';

    let hint: string;
    const suggestion = autos.length > 0 ? `suggested ${range(autos)}` : 'no suggestion yet';
    if (setState === 'suggested') {
      hint = autos.length > 0 ? `Suggested ${range(autos)}` : 'Not analyzed yet; click to set a level';
    } else if (setState === 'set') {
      hint = `Set by you · ${suggestion}`;
    } else {
      hint = `Set by you on ${userCount} of ${layers.length} · ${suggestion}`;
    }
    if (mixed) hint = `Mixed: ${range(known)} · ${hint}`;

    const flaggable = flagControlVisible(music.value.length, id);
    const flag = flaggable ? (music.value[0]?.moods.find((m) => m.mood === id)?.flag ?? null) : null;

    return { id, name, value: mixed ? null : min, min, max, mixed, setState, hint, flaggable, flag };
  }),
);

function save(mood: MoodId, level: number | null): void {
  if (ids.value.length === 0) return;
  const entryIds = ids.value;
  const key = slot(entryIds, mood);
  pending.set(key, level === null ? 'clear' : level);
  const run = (chains.get(key) ?? Promise.resolve()).then(() =>
    store.command('set_entry_mood_level', { entryIds, mood, level }),
  );
  chains.set(key, run);
  void run.finally(() => {
    // Only the newest save for these entries and mood clears the pending value.
    if (chains.get(key) === run) {
      chains.delete(key);
      pending.delete(key);
    }
  });
}

/// Flag, or remove the flag from, the one song shown. A failure (such as a
/// higher / lower flag on a song with no suggestion yet) shows as a toast.
function setFlag(mood: MoodId, kind: FlagKind, band: FlagBand | null): void {
  const entryId = ids.value[0];
  if (entryId === undefined) return;
  void store.command('set_mood_flag', { entryId, mood, kind, band });
}
function clearFlag(mood: MoodId): void {
  const entryId = ids.value[0];
  if (entryId === undefined) return;
  void store.command('clear_mood_flag', { entryId, mood });
}

function onBarClick(row: Row, e: MouseEvent): void {
  const bar = e.currentTarget as HTMLElement;
  const rect = bar.getBoundingClientRect();
  if (rect.width <= 0) return;
  const fraction = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  save(row.id, snapLevel(fraction));
}

function onBarKey(row: Row, e: KeyboardEvent): void {
  // A mixed row moves from the middle of its range.
  const from = row.value ?? (row.min !== null && row.max !== null ? (row.min + row.max) / 2 : null);
  let level: number | null | undefined;
  if (e.key === 'ArrowRight' || e.key === 'ArrowUp') level = stepLevel(from, 1);
  else if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') level = stepLevel(from, -1);
  else if (e.key === 'Home') level = 0;
  else if (e.key === 'End') level = 1;
  else if (e.key === 'Delete' || e.key === 'Backspace') level = null;
  if (level === undefined) return;
  e.preventDefault();
  // Keys that change nothing do not save.
  if (level === null ? row.setState === 'suggested' : row.setState === 'set' && row.value === level) return;
  save(row.id, level);
}

const pct = (v: number): string => `${Math.round(v * 1000) / 10}%`;
</script>

<template>
  <div class="mood-profile">
    <div class="legend d-flex align-center ga-3 mb-1">
      <span class="d-inline-flex align-center ga-1"><span class="swatch is-suggested" />Suggested</span>
      <span class="d-inline-flex align-center ga-1"><span class="swatch is-set" />Set by you</span>
    </div>
    <div v-for="row in rows" :key="row.id" class="mood-row" :class="{ 'is-energy': row.id === 'energy' }">
      <span class="mood-name">{{ row.name }}</span>
      <div
        class="mood-bar"
        :class="`state-${row.setState}`"
        role="slider"
        tabindex="0"
        :aria-label="row.name"
        aria-valuemin="0"
        aria-valuemax="1"
        :aria-valuenow="row.value ?? undefined"
        :aria-valuetext="row.mixed ? 'mixed' : row.value === null ? 'no value' : fmt(row.value)"
        @click="onBarClick(row, $event)"
        @keydown="onBarKey(row, $event)"
      >
        <div class="mood-track">
          <div
            v-if="row.mixed && row.min !== null && row.max !== null"
            class="mood-range"
            :style="{ left: pct(row.min), width: pct(row.max - row.min) }"
          />
          <div v-else-if="row.value !== null" class="mood-fill" :style="{ width: pct(row.value) }" />
        </div>
        <span
          v-for="level in MOOD_LEVELS"
          :key="level"
          class="mood-tick"
          :class="{ 'is-on': !row.mixed && row.value !== null && Math.abs(row.value - level) < 1e-9 }"
          :style="{ left: pct(level) }"
        />
        <VTooltip activator="parent" location="top" open-delay="300">{{ row.hint }}</VTooltip>
      </div>
      <span class="mood-value" :class="{ 'text-medium-emphasis': row.setState === 'suggested' }">
        {{ row.mixed ? 'mixed' : row.value === null ? '—' : fmt(row.value) }}
      </span>
      <VBtn
        v-if="row.setState !== 'suggested'"
        icon
        size="x-small"
        variant="text"
        density="comfortable"
        :aria-label="`${row.name}: back to the suggestion`"
        @click="save(row.id, null)"
      >
        <VIcon size="small">mdi-restore</VIcon>
        <VTooltip activator="parent" location="top" open-delay="300">Back to the suggestion</VTooltip>
      </VBtn>
      <span v-else class="reset-placeholder" />
      <VMenu v-if="row.flaggable" location="bottom end">
        <template #activator="{ props: menuProps }">
          <VBtn
            v-bind="menuProps"
            icon
            size="x-small"
            variant="text"
            density="comfortable"
            :color="row.flag ? 'warning' : undefined"
            :class="{ 'flag-idle': !row.flag, 'flag-stale': row.flag?.stale }"
            :aria-label="row.flag ? `${row.name}: flagged ${flagLabel(row.flag)}` : `${row.name}: flag this bar`"
          >
            <VIcon size="small">{{ row.flag ? 'mdi-flag' : 'mdi-flag-outline' }}</VIcon>
            <VTooltip activator="parent" location="top" open-delay="300">
              {{ row.flag ? flagLabel(row.flag) : 'Flag this bar as wrong' }}
            </VTooltip>
          </VBtn>
        </template>
        <VList density="compact">
          <VListItem v-for="b in FLAG_BANDS" :key="b.band" :title="b.label" @click="setFlag(row.id, 'band', b.band)" />
          <VDivider />
          <VListItem title="Higher" @click="setFlag(row.id, 'higher', null)" />
          <VListItem title="Lower" @click="setFlag(row.id, 'lower', null)" />
          <template v-if="row.flag">
            <VDivider />
            <VListItem title="Remove flag" @click="clearFlag(row.id)" />
          </template>
        </VList>
      </VMenu>
      <span v-else class="reset-placeholder" />
    </div>
  </div>
</template>

<style scoped>
.mood-profile {
  display: grid;
  grid-template-columns: 96px minmax(160px, 1fr) 48px 28px 28px;
  align-items: center;
  column-gap: 12px;
  row-gap: 2px;
  font-size: 0.8125rem;
}
.legend,
.mood-row {
  grid-column: 1 / -1;
}
.legend {
  font-size: 0.75rem;
  opacity: 0.75;
}
.mood-row {
  display: grid;
  grid-template-columns: subgrid;
  align-items: center;
  min-height: 26px;
}
/* Energy is a different kind of score: a little space sets it apart. */
.mood-row.is-energy {
  margin-top: 6px;
}
.mood-name {
  white-space: nowrap;
}
.mood-bar {
  position: relative;
  height: 20px;
  cursor: pointer;
  outline: none;
  border-radius: 4px;
}
.mood-bar:focus-visible {
  box-shadow: 0 0 0 2px rgba(var(--v-theme-primary), 0.6);
}
.mood-track {
  position: absolute;
  left: 0;
  right: 0;
  top: 7px;
  height: 6px;
  border-radius: 3px;
  background: rgba(var(--v-theme-on-surface), 0.1);
  overflow: hidden;
}
.mood-fill,
.mood-range {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
}
/* Suggested: muted. Set by you: the accent colour. Set on some of the
   selected entries only: the accent colour, faded. */
.state-suggested .mood-fill,
.state-suggested .mood-range,
.swatch.is-suggested {
  background: rgba(var(--v-theme-on-surface), 0.38);
}
.state-set .mood-fill,
.state-set .mood-range,
.swatch.is-set {
  background: rgb(var(--v-theme-primary));
}
.state-mixed .mood-fill,
.state-mixed .mood-range {
  background: rgba(var(--v-theme-primary), 0.55);
}
.mood-range {
  min-width: 3px;
}
.mood-tick {
  position: absolute;
  top: 4px;
  width: 2px;
  height: 12px;
  margin-left: -1px;
  border-radius: 1px;
  background: rgba(var(--v-theme-on-surface), 0.25);
  pointer-events: none;
}
.mood-tick.is-on {
  background: rgb(var(--v-theme-on-surface));
}
.mood-bar:hover .mood-tick {
  background: rgba(var(--v-theme-on-surface), 0.5);
}
.swatch {
  display: inline-block;
  width: 14px;
  height: 6px;
  border-radius: 3px;
}
.mood-value {
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.flag-idle {
  opacity: 0.45;
}
.flag-idle:hover,
.flag-idle:focus-visible {
  opacity: 1;
}
.flag-stale {
  opacity: 0.5;
}
.reset-placeholder {
  display: inline-block;
  width: 28px;
}
</style>
