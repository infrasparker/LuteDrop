<script setup lang="ts">
import { computed, reactive } from 'vue';
import type { TuneReview } from '@generated/index';
import { useAppStore } from '../../stores/appStore';
import { useLibraryPreview } from '../../composables/useLibraryPreview';
import { useFlagRemoval } from '../../composables/useFlagRemoval';
import { mergeTop, movedMost, outRankLabel, rankMove, strongestOtherMood, topTitle } from '../../utils/moodPage';
import { moodName } from '../../utils/moods';
import MoodSong from './MoodSong.vue';

/// "Top 15 · {version} · n new · n out": first the songs that left the
/// selected column's top list (old rank → new rank; flagged high songs in
/// red), then the merged list (rank, bar, rank move, "New · {its strongest
/// other mood}", ⚑ when flagged), then the unflagged songs that moved most.
/// Rank arrows are coloured only on flagged songs.
const props = defineProps<{
  moodId: string;
  review: TuneReview | null;
  isCurrent: boolean;
  versionName: string;
  /// Songs flagged (or levelled) on the mood, and those asking for a high bar.
  flaggedIds: Set<string>;
  highIds: Set<string>;
}>();

const store = useAppStore();
const preview = useLibraryPreview();
const { removeFlag } = useFlagRemoval();

const merged = computed(() =>
  props.review ? mergeTop(props.review.top, props.review.topBefore, props.review.outRanks ?? []) : null,
);
const title = computed(() => (merged.value ? topTitle(props.versionName, merged.value, props.isCurrent) : `Top 15 · ${props.versionName}`));
const moved = computed(() => (props.review && !props.isCurrent ? movedMost(props.review) : null));
const fmt = (v: number): string => v.toFixed(2);
const pct = (v: number): string => `${Math.max(0, Math.min(1, v)) * 100}%`;

function otherMood(entryId: string): string | null {
  const e = store.findLibraryEntry(entryId);
  const id = e ? strongestOtherMood(e.moods, props.moodId) : null;
  return id === null ? null : moodName(id);
}

function moveClass(entryId: string, move: number | null): Record<string, boolean> {
  const flagged = props.flaggedIds.has(entryId);
  return { up: flagged && (move ?? 0) > 0, down: flagged && (move ?? 0) < 0 };
}

const menus = reactive<Record<string, boolean>>({});
function onRowKey(entryId: string, e: KeyboardEvent): void {
  if (e.target !== e.currentTarget) return;
  if (e.key === 'Enter') {
    e.preventDefault();
    preview.toggle(entryId);
  } else if (e.key === 'f' || e.key === 'F') {
    e.preventDefault();
    menus[entryId] = true;
  } else if (e.key === 'Delete' && props.flaggedIds.has(entryId)) {
    e.preventDefault();
    void removeFlag(entryId, props.moodId);
  }
}
</script>

<template>
  <section class="top-panel" :aria-label="title">
    <div class="panel-title">{{ title }}</div>
    <div v-if="!review || !merged" class="text-body-2 text-medium-emphasis px-2">…</div>
    <template v-else>
      <template v-if="!isCurrent && merged.out.length > 0">
        <div class="sub-title">Out</div>
        <ul class="top-list">
          <li
            v-for="o in merged.out"
            :key="o.entryId"
            class="song-row"
            :class="{ 'out-high': highIds.has(o.entryId) }"
            tabindex="0"
            @keydown="onRowKey(o.entryId, $event)"
          >
            <span class="rank" />
            <MoodSong v-model:menu="menus[o.entryId]" :entry-id="o.entryId" :mood-id="moodId" :name="o.name" />
            <span class="flag-slot">
              <VIcon v-if="flaggedIds.has(o.entryId)" size="x-small" :color="highIds.has(o.entryId) ? 'error' : 'warning'" aria-label="Flagged">mdi-flag</VIcon>
            </span>
            <span class="tag-slot move">{{ outRankLabel(o) }}</span>
            <span class="bar" aria-hidden="true" />
            <span class="val" />
          </li>
        </ul>
        <div class="out-rule" role="separator" />
      </template>

      <div v-if="!isCurrent && merged.same" class="text-body-2 text-medium-emphasis px-2 pb-1">Same as Current</div>
      <ol v-else class="top-list">
        <li
          v-for="r in merged.rows"
          :key="r.entryId"
          class="song-row"
          tabindex="0"
          @keydown="onRowKey(r.entryId, $event)"
        >
          <span class="rank">{{ r.rank }}</span>
          <MoodSong v-model:menu="menus[r.entryId]" :entry-id="r.entryId" :mood-id="moodId" :name="r.name" />
          <!-- Empty grid cell: the song's own flag button now shows that it is flagged. -->
          <span class="flag-slot" />
          <span class="tag-slot">
            <span v-if="!isCurrent && r.isNew" class="new-tag">
              New<template v-if="otherMood(r.entryId)"> · {{ otherMood(r.entryId) }}</template>
            </span>
            <span v-else-if="!isCurrent" class="move" :class="moveClass(r.entryId, r.move)">{{ rankMove(r.move) }}</span>
          </span>
          <span class="bar" aria-hidden="true"><span class="fill" :style="{ width: pct(r.bar) }" /></span>
          <span class="val">{{ fmt(r.bar) }}</span>
        </li>
      </ol>

      <template v-if="moved && (moved.up.length > 0 || moved.down.length > 0)">
        <div class="sub-title">Moved most</div>
        <ul class="top-list">
          <li
            v-for="m in [...moved.up, ...moved.down]"
            :key="m.song.entryId"
            class="song-row"
            tabindex="0"
            @keydown="onRowKey(m.song.entryId, $event)"
          >
            <span class="rank">{{ m.after > m.before ? '↑' : '↓' }}</span>
            <MoodSong v-model:menu="menus[m.song.entryId]" :entry-id="m.song.entryId" :mood-id="moodId" :name="m.song.name" />
            <span class="flag-slot" />
            <span class="tag-slot move">{{ fmt(m.before) }} → {{ fmt(m.after) }}</span>
            <span class="bar" aria-hidden="true" />
            <span class="val" />
          </li>
        </ul>
      </template>
    </template>
  </section>
</template>

<style scoped>
.top-panel {
  min-height: 0;
  overflow: auto;
}
.panel-title {
  font-weight: 600;
  font-size: 0.875rem;
  padding: 6px 8px;
  position: sticky;
  top: 0;
  z-index: 2;
  background: rgb(var(--v-theme-surface));
}
.sub-title {
  font-size: 0.75rem;
  color: rgba(var(--v-theme-on-surface), 0.6);
  padding: 4px 8px 0;
}
.out-rule {
  margin: 4px 8px;
  border-top: 1px solid rgba(var(--v-theme-on-surface), 0.12);
}
.top-list {
  list-style: none;
  margin: 0;
  padding: 0;
  font-size: 0.8125rem;
}
.song-row {
  display: grid;
  grid-template-columns: 22px minmax(0, 1fr) 16px minmax(0, max-content) 56px 34px;
  align-items: center;
  column-gap: 6px;
  height: 24px;
  padding: 0 8px;
  outline: none;
}
.song-row :deep(.song-name) {
  min-height: 24px;
  padding: 0;
}
.song-row :deep(.song-actions .v-btn) {
  min-height: 24px;
  height: 24px;
}
.song-row:hover {
  background: rgba(var(--v-theme-on-surface), 0.04);
}
.song-row:focus-visible {
  box-shadow: inset 0 0 0 2px rgba(var(--v-theme-primary), 0.6);
}
.tag-slot {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.rank {
  text-align: right;
  color: rgba(var(--v-theme-on-surface), 0.6);
  font-variant-numeric: tabular-nums;
}
.move,
.val {
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  font-size: 0.8125rem;
  color: rgba(var(--v-theme-on-surface), 0.7);
}
.val {
  text-align: right;
}
.move.up {
  color: #66bb6a;
}
.move.down {
  color: #ef5350;
}
.new-tag {
  font-size: 0.75rem;
  padding: 0 6px;
  border-radius: 8px;
  background: rgba(var(--v-theme-primary), 0.14);
  white-space: nowrap;
}
.bar {
  height: 6px;
  border-radius: 3px;
  background: rgba(var(--v-theme-on-surface), 0.08);
  overflow: hidden;
}
.bar:empty {
  background: transparent;
}
.fill {
  display: block;
  height: 100%;
  background: rgba(var(--v-theme-on-surface), 0.45);
}
.out-high {
  color: #ef5350;
  font-weight: 700;
}
</style>
