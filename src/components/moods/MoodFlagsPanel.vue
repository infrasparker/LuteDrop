<script setup lang="ts">
import { computed, reactive } from 'vue';
import type { FlagBand, FlagKind, MoodFlag, TuneFlagOutcome, TuneReview } from '@generated/index';
import { useAppStore } from '../../stores/appStore';
import { useLibraryPreview } from '../../composables/useLibraryPreview';
import { FLAG_BANDS, feedbackPreference, feedbackDisagreement, feedbackDescription } from '../../utils/moodFlags';
import { useFlagRemoval } from '../../composables/useFlagRemoval';
import { bandWord, useShortBands, type CellModel, type RowModel } from '../../utils/moodPage';
import MoodSong from './MoodSong.vue';

/// Every example stays visible, with saved and preview scores side by side.
/// Enter auditions a row; F opens its feedback menu; Delete removes its flag
/// with Undo. Direction glyphs supplement the numerical scores.
const props = defineProps<{
  moodId: string;
  /// Current first (its review is Current's own), then each version, with
  /// the songs Use would break (`regressions`).
  columns: { key: number | 'current'; name: string; review: TuneReview | null; stale: boolean; regressionIds: Set<string>; comparable: boolean }[];
  focusEntryId: string | null;
  narrow: boolean;
}>();

const store = useAppStore();
const preview = useLibraryPreview();
const { removeFlag } = useFlagRemoval();
const short = computed(() => useShortBands(props.narrow, props.columns.length - 1));

interface Row extends RowModel {
  name: string;
  flag: MoodFlag | null;
  level: number | null;
  bandLong: string;
  bandShort: string;
  bounds: ReturnType<typeof feedbackPreference>;
  cells: (CellModel & { missing: boolean; pending: boolean })[];
}

const outcomeOf = (r: TuneReview | null, id: string): TuneFlagOutcome | undefined => r?.flags.find((f) => f.song.entryId === id);

const rows = computed<Row[]>(() => {
  const out: Row[] = [];
  const baseline = props.columns[0]?.review ?? null;
  for (const e of store.libraryEntries) {
    const mv = e.moods.find((m) => m.mood === props.moodId);
    if (!mv || (mv.flag === null && mv.user === null)) continue;
    const flag = mv.flag;
    const level = flag ? null : mv.user;
    const bounds = feedbackPreference(flag, level, mv.auto);
    const base = outcomeOf(baseline, e.id);
    const curValue = base?.before ?? mv.auto ?? null;
    const cells = props.columns.map((col, i) => {
      if (i === 0) {
        return {
          state: 'holds' as const,
          value: curValue,
          regression: false,
          missing: curValue === null,
          pending: baseline === null && curValue === null,
        };
      }
      const o = outcomeOf(col.review, e.id);
      if (!o) {
        return { state: 'holds' as const, value: null, regression: false, missing: col.review !== null, pending: col.review === null };
      }
      return {
        state: !bounds || !col.comparable || curValue === null ? 'holds' as const : feedbackDisagreement(o.after,bounds) < feedbackDisagreement(curValue,bounds)-.001 ? 'closer' as const : feedbackDisagreement(o.after,bounds) > feedbackDisagreement(curValue,bounds)+.001 ? 'worse' as const : 'holds' as const,
        value: o.after,
        regression: false,
        missing: false,
        pending: false,
      };
    });
    out.push({
      entryId: e.id,
      name: e.name,
      focus: e.id === props.focusEntryId,
      flag,
      level,
      bandLong: bandWord(flag, level),
      bandShort: bandWord(flag, level, true),
      cells,
      bounds,
    });
  }
  return out.sort((a, b) => a.name.localeCompare(b.name));
});

const GLYPH: Record<string, string> = { holds: '', closer: '↗', worse: '↘', broken: '' };
const STATE_LABEL: Record<string, string> = { holds: 'score', closer: 'agrees more with feedback', worse: 'agrees less with feedback', broken: 'score' };
const fmt = (v: number): string => v.toFixed(3);

function preferenceDetail(value: number, preference: ReturnType<typeof feedbackPreference>): string {
  return `${feedbackDescription(preference)} · disagreement ${feedbackDisagreement(value,preference).toFixed(3)}`;
}

const menus = reactive<Record<string, boolean>>({});

function onRowKey(r: Row, e: KeyboardEvent): void {
  if (e.target !== e.currentTarget) return;
  if (e.key === 'Enter') {
    e.preventDefault();
    preview.toggle(r.entryId);
  } else if (e.key === 'f' || e.key === 'F') {
    e.preventDefault();
    menus[r.entryId] = true;
  } else if (e.key === 'Delete' && r.flag) {
    e.preventDefault();
    void removeFlag(r.entryId, props.moodId);
  }
}

function setFlag(entryId: string, kind: FlagKind, band: FlagBand | null): void {
  void store.command('set_mood_flag', { entryId, mood: props.moodId, kind, band });
}

const gridColumns = computed(
  () => `minmax(120px, 1.6fr) ${short.value ? '56px' : 'minmax(96px, 0.9fr)'} repeat(${props.columns.length}, 88px)`,
);
</script>

<template>
  <section class="flags-panel" aria-label="Examples">
    <div class="panel-title">Examples</div>
    <div v-if="rows.length === 0" class="empty text-body-2 text-medium-emphasis">No examples yet.</div>
    <div v-else class="table" role="table" :style="{ gridTemplateColumns: gridColumns }">
      <div class="head" role="row">
        <span role="columnheader">Song</span>
        <span role="columnheader">Feedback</span>
        <span v-for="c in columns" :key="c.key" role="columnheader" class="num" :title="c.name">{{ c.key === 'current' ? 'Saved score' : 'Preview score' }}</span>
      </div>
      <div
        v-for="r in rows"
        :key="r.entryId"
        class="song-row"
        :class="{ stale: r.flag?.stale }"
        role="row"
        tabindex="0"
        :aria-label="`${r.name}, ${r.bandLong}`"
        @keydown="onRowKey(r, $event)"
      >
        <span class="song-cell" role="cell">
          <!-- Fixed-width slot on every row, so the focus dot never shifts a row out of line. -->
          <span class="focus-dot" :aria-label="r.focus ? 'Focus' : undefined">
            <template v-if="r.focus">●<VTooltip activator="parent" location="top">Focus</VTooltip></template>
          </span>
          <MoodSong v-model:menu="menus[r.entryId]" :entry-id="r.entryId" :mood-id="moodId" :name="r.name" />
          <VTooltip v-if="r.flag?.stale" activator="parent" location="top" open-delay="300">Set under an earlier description; Preview still uses this example at full weight</VTooltip>
        </span>
        <span class="band-cell" role="cell" :aria-label="r.bandLong">
          <VTooltip v-if="r.bounds" activator="parent" location="top">{{ feedbackDescription(r.bounds) }}</VTooltip>
          {{ short ? r.bandShort : r.bandLong }}
          <VTooltip v-if="short" activator="parent" location="top">{{ r.bandLong }}</VTooltip>
          <span v-if="r.flag?.stale" class="stale-actions">
            <button type="button" @click="store.command('keep_mood_flag', { entryId: r.entryId, mood: moodId })">Keep</button>
            <VMenu location="bottom end">
              <template #activator="{ props: menuProps }">
                <button v-bind="menuProps" type="button">Change</button>
              </template>
              <VList density="compact">
                <VListItem v-for="b in FLAG_BANDS" :key="b.band" :title="b.menuLabel" @click="setFlag(r.entryId, 'band', b.band)" />
                <VDivider />
                <VListItem title="Higher" @click="setFlag(r.entryId, 'higher', null)" />
                <VListItem title="Lower" @click="setFlag(r.entryId, 'lower', null)" />
              </VList>
            </VMenu>
            <button type="button" @click="removeFlag(r.entryId, moodId)">Remove</button>
          </span>
        </span>
        <span
          v-for="(c, i) in r.cells"
          :key="i"
          role="cell"
          class="num value"
          :class="[`is-${c.state}`, { regression: c.regression, dim: columns[i]?.stale }]"
          :aria-label="c.value === null ? (c.missing ? 'Not analyzed' : '') : `${fmt(c.value)} ${c.regression ? 'newly outside preference' : STATE_LABEL[c.state]}`"
        >
          <template v-if="c.value !== null">
            {{ fmt(c.value) }} <span class="glyph">{{ GLYPH[c.state] }}</span>
            <VTooltip activator="parent" location="top">{{ preferenceDetail(c.value, r.bounds) }}</VTooltip>

          </template>
          <template v-else-if="c.missing">—<VTooltip activator="parent" location="top">Not analyzed</VTooltip></template>
        </span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.flags-panel {
  min-height: 0;
  overflow: auto;
}
.panel-title {
  font-weight: 600;
  font-size: 0.875rem;
  padding: 6px 8px;
  position: sticky;
  top: 0;
  left: 0;
  z-index: 3;
  background: rgb(var(--v-theme-surface));
}
.empty {
  padding: 8px;
}
.table {
  display: grid;
  column-gap: 0;
  font-size: 0.875rem;
  min-width: min-content;
}
.head,
.song-row {
  display: grid;
  grid-column: 1 / -1;
  grid-template-columns: subgrid;
}
.head > span {
  position: sticky;
  top: 32px;
  z-index: 2;
  background: rgb(var(--v-theme-surface));
  font-size: 0.75rem;
  color: rgba(var(--v-theme-on-surface), 0.6);
  padding: 2px 4px;
  border-bottom: 1px solid rgba(var(--v-theme-on-surface), 0.12);
}
.song-row > span {
  padding: 0 4px;
  min-height: 30px;
  display: flex;
  align-items: center;
  border-bottom: 1px solid rgba(var(--v-theme-on-surface), 0.05);
}
.song-row:focus-visible {
  box-shadow: inset 0 0 0 2px rgba(var(--v-theme-primary), 0.6);
}
.song-row:hover > span {
  background: rgba(var(--v-theme-on-surface), 0.04);
}
.song-row {
  outline: none;
}
.song-cell {
  min-width: 0;
  gap: 4px;
}
.song-row.stale .song-cell,
.song-row.stale .band-cell {
  opacity: 0.55;
}
.song-row.stale:hover .song-cell,
.song-row.stale:focus-within .song-cell,
.song-row.stale:hover .band-cell,
.song-row.stale:focus-within .band-cell {
  opacity: 1;
}
.focus-dot {
  color: rgb(var(--v-theme-primary));
  font-size: 0.75rem;
  flex: 0 0 10px;
  width: 10px;
}
.band-cell {
  white-space: nowrap;
  gap: 6px;
  position: relative;
}
.stale-actions {
  display: none;
  gap: 6px;
}
.stale-actions button {
  appearance: none;
  background: transparent;
  border: 0;
  color: rgb(var(--v-theme-primary));
  min-height: 28px;
  padding: 0 2px;
}
.song-row:hover .stale-actions,
.song-row:focus-within .stale-actions {
  display: inline-flex;
}
.num {
  justify-content: flex-end;
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.glyph {
  display: inline-block;
  width: 1em;
  text-align: center;
  margin-left: 2px;
}
/* Glyph and colour, never colour alone: green holds, amber closer, red worse. */
.value.is-holds .glyph {
  color: #66bb6a;
}
.value.is-closer .glyph {
  color: rgb(var(--v-theme-primary));
}
.value.is-worse {
  color: rgb(var(--v-theme-primary));
}
.value.is-broken .glyph {
  color: rgb(var(--v-theme-primary));
}
.value.regression {
  font-weight: 700;
}
.value.dim {
  opacity: 0.45;
}
</style>
