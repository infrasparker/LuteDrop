<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import type { TrackPlaybackState } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';
import { formatDuration } from '../shared/format';

// Now-playing + scrubber for the one thing that is playing or paused: a
// library preview or a single track. It always renders at the same size so
// the footer never changes height: when there is nothing to scrub (nothing
// active, several tracks active — a scene, where one position bar would be
// ambiguous — or a gap between clips) it shows a muted status line and a
// disabled slider instead.

const store = useAppStore();
const audio = useAudio();

interface ScrubTarget {
  state: TrackPlaybackState;
  /// Present for a track; absent for a library preview.
  trackId: string | null;
  clipId: string | null;
  entryId: string;
  name: string;
  /// Playable range in file seconds (trim-aware). `end` is null when the
  /// entry's duration is unknown — the slider is then disabled.
  start: number;
  end: number | null;
}

const target = computed<ScrubTarget | null>(() => {
  let only: TrackPlaybackState | null = null;
  for (const s of store.playbackStates.values()) {
    if (!s.isPlaying && !s.isPaused) continue;
    if (only) return null; // more than one thing active
    only = s;
  }
  if (!only) return null;

  if (only.trackId === LIBRARY_PLAYBACK_ID) {
    const entryId = store.libraryPlayingEntryId;
    const entry = entryId ? store.findLibraryEntry(entryId) : undefined;
    if (!entry) return null;
    return {
      state: only, trackId: null, clipId: null, entryId: entry.id, name: entry.name,
      start: entry.trimStart ?? 0,
      end: entry.trimEnd ?? entry.duration,
    };
  }

  // During a Gap transition the track counts as playing but has no sound
  // yet (position 0, previous clip id): nothing to scrub until it starts.
  if (!only.hasSound) return null;
  const track = store.findTrack(only.trackId);
  const clip = track?.clips.find((c) => c.id === only.currentClipId);
  if (!track || !clip) return null;
  const entry = store.findLibraryEntry(clip.libraryEntryId);
  let end = clip.trimEnd ?? entry?.duration ?? null;
  // A seamless track loops at loop_end, so the file past it never plays:
  // end the bar there (same rule as the engine: loopEnd 0 means unset).
  if (track.mode.type === 'seamless' && track.mode.loopEnd > 0) {
    end = end === null ? track.mode.loopEnd : Math.min(end, track.mode.loopEnd);
  }
  return {
    state: only, trackId: only.trackId, clipId: clip.id, entryId: clip.libraryEntryId, name: clip.name,
    start: clip.trimStart ?? 0,
    end,
  };
});

// While the pointer is down on the slider, `dragValue` drives the thumb and
// backend updates don't move it (see the seeking section below).
const dragging = ref(false);
const dragValue = ref(0);

// --- Smooth position: interpolate between the ~50 ms backend updates ---

const displayTime = ref(0);
let syncTime = 0; // last position reported by the backend (or our seek)
let syncAt = 0; // performance.now() when syncTime was taken
let rafId: number | null = null;

function clampToRange(t: number): number {
  const tg = target.value;
  if (!tg) return t;
  const hi = tg.end ?? Infinity;
  return Math.min(Math.max(t, tg.start), hi);
}

function tick(): void {
  const tg = target.value;
  if (tg?.state.isPlaying && !dragging.value) {
    displayTime.value = clampToRange(syncTime + (performance.now() - syncAt) / 1000);
  }
  rafId = tg?.state.isPlaying ? requestAnimationFrame(tick) : null;
}

function ensureTicking(): void {
  if (rafId === null && target.value?.state.isPlaying) rafId = requestAnimationFrame(tick);
}

// Resync on every backend update for the active item.
watch(
  () => target.value?.state,
  (s) => {
    if (!s) return;
    syncTime = s.currentTime;
    syncAt = performance.now();
    if (!dragging.value) displayTime.value = clampToRange(s.currentTime);
    ensureTicking();
  },
  { immediate: true },
);

onUnmounted(() => {
  if (rafId !== null) cancelAnimationFrame(rafId);
  removeReleaseListeners();
});

// --- Dragging / seeking ---

// Vuetify's slider updates the model before its `start` event, so we track
// the pointer ourselves: while it is down, slider updates only move the
// thumb locally; the seek fires once on `end`. Keyboard changes (pointer up)
// seek immediately.
//
// Vuetify only emits `end` for presses that land on the slider itself, so a
// press elsewhere in the wrapper (or a release Vuetify misses) would leave us
// stuck in `dragging`. A one-time window listener ends the drag on release
// no matter where it happens.
let dragStartValue = 0;

function onPointerDown(e: PointerEvent): void {
  if (e.button !== 0 || target.value?.end == null) return;
  dragging.value = true;
  dragValue.value = displayTime.value;
  dragStartValue = displayTime.value;
  removeReleaseListeners(); // in case an earlier drag never cleaned up
  window.addEventListener('pointerup', onWindowRelease);
  window.addEventListener('pointercancel', onWindowRelease);
  window.addEventListener('blur', onWindowRelease);
}

function removeReleaseListeners(): void {
  window.removeEventListener('pointerup', onWindowRelease);
  window.removeEventListener('pointercancel', onWindowRelease);
  window.removeEventListener('blur', onWindowRelease);
}

function onWindowRelease(): void {
  removeReleaseListeners();
  // Vuetify ends its drag on the `mouseup` that follows this `pointerup`, so
  // wait a tick to let its `end` (which seeks) run first. If it did, we are
  // no longer dragging and there is nothing left to do.
  setTimeout(() => {
    if (!dragging.value) return;
    dragging.value = false;
    // Vuetify never saw this drag end: seek to wherever the thumb was moved,
    // or snap back to the live position if it never moved.
    if (dragValue.value !== dragStartValue) seek(dragValue.value);
  }, 0);
}

function onSliderInput(v: number): void {
  if (dragging.value) {
    dragValue.value = v;
  } else {
    seek(v);
  }
}

function onSliderEnd(v: number): void {
  if (!dragging.value) return;
  dragging.value = false;
  removeReleaseListeners();
  seek(v);
}

async function seek(position: number): Promise<void> {
  const tg = target.value;
  if (!tg || tg.end === null) return;
  const pos = clampToRange(position);
  const wasPaused = tg.state.isPaused;

  // Optimistic: jump the bar now; the next backend update resyncs it.
  displayTime.value = pos;
  syncTime = pos;
  syncAt = performance.now();

  // One backend call per seek: with `paused` the engine starts the new
  // position and pauses it under one lock, so a paused item stays silent
  // (no play-then-pause burst, no click).
  if (tg.trackId) {
    await audio.playTrackFrom(tg.trackId, pos, tg.clipId ?? undefined, wasPaused);
    return;
  }

  const entry = store.findLibraryEntry(tg.entryId);
  if (!entry) return;
  // Same rule as the waveform editor's click-to-play: only keep the loop
  // region when the seek lands before its end.
  const loopEnd = entry.loopEnd && entry.loopEnd > 0 ? entry.loopEnd : null;
  const keepLoop = loopEnd === null || pos < loopEnd;
  await audio.playLibrarySound(
    entry.id, entry.filePath, pos, entry.volume,
    entry.trimStart, entry.trimEnd,
    keepLoop ? entry.loopStart : null,
    keepLoop ? entry.loopEnd : null,
    wasPaused,
  );
}

// --- Display ---

const sliderValue = computed(() => (dragging.value ? dragValue.value : displayTime.value));
const elapsedLabel = computed(() => {
  const tg = target.value;
  return tg ? formatDuration(Math.max(0, sliderValue.value - tg.start)) : '';
});
const totalLabel = computed(() => {
  const tg = target.value;
  return tg && tg.end !== null ? formatDuration(Math.max(0, tg.end - tg.start)) : '--:--';
});

/// Status shown in place of a name when there is nothing to scrub.
const placeholder = computed(() => {
  let playing = 0;
  let paused = 0;
  for (const s of store.playbackStates.values()) {
    if (s.isPlaying) playing++;
    else if (s.isPaused) paused++;
  }
  if (playing === 0 && paused === 0) return 'Nothing playing';
  if (playing === 0) return paused === 1 ? '1 clip paused' : `${paused} clips paused`;
  return playing === 1 ? '1 clip playing' : `${playing} clips playing`;
});

function openDetails(): void {
  if (target.value) store.editEntryId = target.value.entryId;
}
</script>

<template>
  <div class="scrubber">
    <div class="scrubber-info">
      <button
        v-if="target"
        type="button"
        class="scrubber-name text-primary"
        :title="`${target.name} — open details`"
        @click="openDetails"
      >
        {{ target.name }}
      </button>
      <span v-else class="scrubber-name text-medium-emphasis">{{ placeholder }}</span>
      <span class="scrubber-time" :class="{ 'scrubber-time--idle': !target }">
        {{ target ? elapsedLabel : '--:--' }} / {{ totalLabel }}
      </span>
    </div>
    <div class="scrubber-slider" @pointerdown.capture="onPointerDown">
      <VSlider
        :model-value="target ? sliderValue : 0"
        :min="target?.start ?? 0"
        :max="target ? (target.end ?? target.start + 1) : 1"
        :step="0"
        :disabled="!target || target.end === null"
        hide-details
        density="compact"
        color="primary"
        @update:model-value="onSliderInput"
        @end="onSliderEnd"
      />
    </div>
  </div>
</template>

<style scoped>
/* Fixed footprint: the name column has a fixed width and both text lines
   ellipsize / use tabular digits, so nothing here reflows with playback. */
.scrubber {
  display: grid;
  grid-template-columns: 180px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  width: 100%;
  min-width: 0;
}

.scrubber-info {
  display: flex;
  flex-direction: column;
  min-width: 0;
  line-height: 1.25;
}

.scrubber-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
  background: none;
  border: none;
  padding: 0;
  font: inherit;
  font-size: 0.875rem;
}

button.scrubber-name {
  cursor: pointer;
}

button.scrubber-name:hover {
  text-decoration: underline;
}

.scrubber-time {
  font-size: 0.75rem;
  font-variant-numeric: tabular-nums;
  color: rgba(255, 255, 255, 0.72);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.scrubber-time--idle {
  color: rgba(255, 255, 255, 0.38);
}

.scrubber-slider {
  min-width: 0;
}
</style>
