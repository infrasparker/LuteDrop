<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAudio } from '../composables/useAudio';
import { useAppStore } from '../stores/appStore';

interface WaveformData {
  peaks: number[];
  duration: number;
}

const props = defineProps<{
  filePath: string;
  loopStart: number;
  loopEnd: number;
  trackId: string;
}>();

const emit = defineEmits<{
  'update:loopStart': [value: number];
  'update:loopEnd': [value: number];
}>();

const audio = useAudio();
const store = useAppStore();

const canvasRef = ref<HTMLCanvasElement | null>(null);

const CANVAS_HEIGHT = 80;
const MARKER_GRAB_THRESHOLD_PX = 7; // in actual screen pixels
const ZOOM_MIN = 1.0;
const ZOOM_MAX = 50.0;
const ZOOM_STEP = 1.15; // multiplier per wheel tick

const duration = ref(0);
const loading = ref(false);
const error = ref('');

// Two-tier peaks: basePeaks (full file, always available) + zoomedPeaks (high-res visible range)
const basePeaks = ref<number[]>([]);
const basePeaksRangeStart = ref(0);
const basePeaksRangeEnd = ref(0);

const zoomedPeaks = ref<number[]>([]);
const zoomedPeaksRangeStart = ref(0);
const zoomedPeaksRangeEnd = ref(0);

// Zoom & pan state
const zoomLevel = ref(1.0);
const viewStart = ref(0);
const viewEnd = ref(0);

type MarkerType = 'loopStart' | 'loopEnd' | null;
const dragging = ref<MarkerType>(null);
const panning = ref(false);
const panStartX = ref(0);
const panStartViewStart = ref(0);

// Click-to-preview: brief play cursor line
const playCursorTime = ref<number | null>(null);
let playCursorTimeout: ReturnType<typeof setTimeout> | null = null;

// Playhead animation
let rafId: number | null = null;

const playbackState = computed(() => store.playbackStates.get(props.trackId));
const isTrackActive = computed(() => {
  const s = playbackState.value;
  return s && (s.isPlaying || s.isPaused);
});

function startPlayheadLoop(): void {
  if (rafId !== null) return;
  function tick() {
    drawWaveform();
    const s = playbackState.value;
    if (s?.isPlaying) {
      rafId = requestAnimationFrame(tick);
    } else {
      rafId = null;
    }
  }
  rafId = requestAnimationFrame(tick);
}

function stopPlayheadLoop(): void {
  if (rafId !== null) {
    cancelAnimationFrame(rafId);
    rafId = null;
  }
}

watch(() => playbackState.value?.isPlaying, (playing) => {
  if (playing) {
    startPlayheadLoop();
  } else {
    stopPlayheadLoop();
    drawWaveform(); // one last draw to show paused position or clear
  }
});

function showPlayCursor(seconds: number): void {
  playCursorTime.value = seconds;
  if (playCursorTimeout) clearTimeout(playCursorTimeout);
  playCursorTimeout = setTimeout(() => {
    playCursorTime.value = null;
    drawWaveform();
  }, 600);
  drawWaveform();
}

const effectiveLoopEnd = computed(() => {
  return props.loopEnd === 0 ? duration.value : props.loopEnd;
});

const loopStartDisplay = computed(() => props.loopStart.toFixed(2));
const loopEndDisplay = computed(() => {
  return props.loopEnd === 0 ? 'end' : props.loopEnd.toFixed(2);
});

const isZoomed = computed(() => zoomLevel.value > 1.0);

const viewRangeDisplay = computed(() => {
  if (!isZoomed.value) return '';
  return `Viewing: ${viewStart.value.toFixed(1)}s – ${viewEnd.value.toFixed(1)}s / ${duration.value.toFixed(1)}s`;
});

const canvasCursor = computed(() => {
  if (dragging.value) return 'grabbing';
  if (panning.value) return 'grabbing';
  return 'pointer';
});

// Debounced zoom peak request
let zoomDebounce: ReturnType<typeof setTimeout> | null = null;

function requestZoomedPeaks(): void {
  if (zoomDebounce) clearTimeout(zoomDebounce);
  zoomDebounce = setTimeout(async () => {
    if (duration.value <= 0 || !props.filePath) return;

    if (!isZoomed.value) {
      // Zoomed all the way out -- clear zoomed peaks, basePeaks are enough
      zoomedPeaks.value = [];
      drawWaveform();
      return;
    }

    // Request high-res peaks for only the visible range
    const canvasPixelWidth = Math.round(getCanvasWidth() * (window.devicePixelRatio || 1));
    try {
      const reqStart = viewStart.value;
      const reqEnd = viewEnd.value;
      const data = await invoke<WaveformData>('get_waveform_data', {
        filePath: props.filePath,
        width: canvasPixelWidth,
        start: reqStart,
        end: reqEnd,
      });
      zoomedPeaks.value = data.peaks;
      zoomedPeaksRangeStart.value = reqStart;
      zoomedPeaksRangeEnd.value = reqEnd;
      drawWaveform();
    } catch (_) {
      // keep existing peaks on error
    }
  }, 150);
}

// Get the actual pixel width of the canvas element
function getCanvasWidth(): number {
  const canvas = canvasRef.value;
  if (!canvas) return 800;
  return canvas.getBoundingClientRect().width;
}

// Sync viewStart/viewEnd from zoomLevel, keeping them consistent
function resetView(): void {
  zoomLevel.value = 1.0;
  viewStart.value = 0;
  viewEnd.value = duration.value;
}

function clampView(): void {
  const dur = duration.value;
  if (dur <= 0) return;

  const visibleDuration = dur / zoomLevel.value;

  // Clamp viewStart so the window stays within [0, dur]
  if (viewStart.value < 0) viewStart.value = 0;
  if (viewStart.value + visibleDuration > dur) {
    viewStart.value = dur - visibleDuration;
  }
  if (viewStart.value < 0) viewStart.value = 0;

  viewEnd.value = viewStart.value + visibleDuration;
}

async function loadWaveform(): Promise<void> {
  if (!props.filePath) return;

  loading.value = true;
  error.value = '';

  try {
    // Request peaks at a reasonable resolution for full file
    const data = await invoke<WaveformData>('get_waveform_data', {
      filePath: props.filePath,
      width: 1200,
    });
    basePeaks.value = data.peaks;
    basePeaksRangeStart.value = 0;
    basePeaksRangeEnd.value = data.duration;
    zoomedPeaks.value = [];
    duration.value = data.duration;
    resetView();
  } catch (e: unknown) {
    error.value = String(e);
    basePeaks.value = [];
    zoomedPeaks.value = [];
    duration.value = 0;
  } finally {
    loading.value = false;
  }
}

function secondsToX(seconds: number, w: number): number {
  const visibleDuration = viewEnd.value - viewStart.value;
  if (visibleDuration <= 0) return 0;
  return ((seconds - viewStart.value) / visibleDuration) * w;
}

function xToSeconds(x: number, w: number): number {
  const visibleDuration = viewEnd.value - viewStart.value;
  if (visibleDuration <= 0 || w <= 0) return 0;
  const seconds = viewStart.value + (x / w) * visibleDuration;
  return Math.max(0, Math.min(duration.value, seconds));
}

function drawPeaksPath(
  ctx: CanvasRenderingContext2D,
  peaksArr: number[],
  rangeStart: number,
  rangeEnd: number,
  cssW: number,
  cssH: number,
  color: string,
): void {
  const peakCount = peaksArr.length;
  if (peakCount === 0) return;

  const pDuration = rangeEnd - rangeStart;
  if (pDuration <= 0) return;

  const midY = cssH / 2;
  const maxBarHeight = midY - 2;

  // Only draw peaks that fall within the visible range
  const startIdx = Math.max(0, Math.floor(((viewStart.value - rangeStart) / pDuration) * peakCount));
  const endIdx = Math.min(peakCount, Math.ceil(((viewEnd.value - rangeStart) / pDuration) * peakCount));

  if (startIdx >= endIdx) return;

  ctx.fillStyle = color;

  // Top half
  ctx.beginPath();
  const x0 = secondsToX(rangeStart + (startIdx / peakCount) * pDuration, cssW);
  ctx.moveTo(x0, midY);
  for (let i = startIdx; i < endIdx; i++) {
    const peakTime = rangeStart + (i / peakCount) * pDuration;
    const x = secondsToX(peakTime, cssW);
    ctx.lineTo(x, midY - peaksArr[i] * maxBarHeight);
  }
  const xLast = secondsToX(rangeStart + ((endIdx - 1) / peakCount) * pDuration, cssW);
  ctx.lineTo(xLast, midY);
  ctx.closePath();
  ctx.fill();

  // Bottom half (mirror)
  ctx.beginPath();
  ctx.moveTo(x0, midY);
  for (let i = startIdx; i < endIdx; i++) {
    const peakTime = rangeStart + (i / peakCount) * pDuration;
    const x = secondsToX(peakTime, cssW);
    ctx.lineTo(x, midY + peaksArr[i] * maxBarHeight);
  }
  ctx.lineTo(xLast, midY);
  ctx.closePath();
  ctx.fill();
}

function drawWaveform(): void {
  const canvas = canvasRef.value;
  if (!canvas) return;

  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  // Match canvas resolution to display size (avoids stretching)
  const displayWidth = canvas.getBoundingClientRect().width;
  const dpr = window.devicePixelRatio || 1;
  const w = Math.round(displayWidth * dpr);
  const h = Math.round(CANVAS_HEIGHT * dpr);

  canvas.width = w;
  canvas.height = h;
  ctx.scale(dpr, dpr);

  // Draw in CSS pixels from here
  const cssW = displayWidth;
  const cssH = CANVAS_HEIGHT;
  const midY = cssH / 2;

  // 1. Background
  ctx.fillStyle = '#141418';
  ctx.fillRect(0, 0, cssW, cssH);

  if (basePeaks.value.length === 0) return;

  if (isZoomed.value) {
    if (zoomedPeaks.value.length > 0) {
      // Draw base layer only for areas NOT covered by zoomed peaks
      const zoomStartX = secondsToX(zoomedPeaksRangeStart.value, cssW);
      const zoomEndX = secondsToX(zoomedPeaksRangeEnd.value, cssW);

      // Left uncovered area
      if (zoomStartX > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, 0, zoomStartX, cssH);
        ctx.clip();
        drawPeaksPath(ctx, basePeaks.value, basePeaksRangeStart.value, basePeaksRangeEnd.value, cssW, cssH, '#7A5418');
        ctx.restore();
      }

      // Right uncovered area
      if (zoomEndX < cssW) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(zoomEndX, 0, cssW - zoomEndX, cssH);
        ctx.clip();
        drawPeaksPath(ctx, basePeaks.value, basePeaksRangeStart.value, basePeaksRangeEnd.value, cssW, cssH, '#7A5418');
        ctx.restore();
      }

      // Draw zoomed detail in full gold
      drawPeaksPath(ctx, zoomedPeaks.value, zoomedPeaksRangeStart.value, zoomedPeaksRangeEnd.value, cssW, cssH, '#F0A830');
    } else {
      // No zoomed peaks yet — show base layer everywhere
      drawPeaksPath(ctx, basePeaks.value, basePeaksRangeStart.value, basePeaksRangeEnd.value, cssW, cssH, '#7A5418');
    }
  } else {
    drawPeaksPath(ctx, basePeaks.value, basePeaksRangeStart.value, basePeaksRangeEnd.value, cssW, cssH, '#F0A830');
  }

  // 4. Draw loop markers (solid green lines) — only if visible
  const loopStartX = secondsToX(props.loopStart, cssW);
  const loopEndX = secondsToX(effectiveLoopEnd.value, cssW);

  ctx.strokeStyle = '#4CAF50';
  ctx.lineWidth = 2;

  if (loopStartX >= -2 && loopStartX <= cssW + 2) {
    ctx.beginPath();
    ctx.moveTo(loopStartX, 0);
    ctx.lineTo(loopStartX, cssH);
    ctx.stroke();

    // Half-box handle at top, pointing outward (left of line)
    ctx.fillStyle = '#4CAF50';
    ctx.fillRect(loopStartX - 5, 0, 5, 8);
  }

  if (loopEndX >= -2 && loopEndX <= cssW + 2) {
    ctx.strokeStyle = '#4CAF50';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(loopEndX, 0);
    ctx.lineTo(loopEndX, cssH);
    ctx.stroke();

    // Half-box handle at top, pointing outward (right of line)
    ctx.fillStyle = '#4CAF50';
    ctx.fillRect(loopEndX, 0, 5, 8);
  }

  // 5. Center line
  ctx.strokeStyle = 'rgba(232, 228, 220, 0.15)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(cssW, midY);
  ctx.stroke();

  // 6. Play cursor (brief visual feedback on click-to-preview)
  if (playCursorTime.value !== null) {
    const cursorX = secondsToX(playCursorTime.value, cssW);
    if (cursorX >= 0 && cursorX <= cssW) {
      ctx.strokeStyle = '#E8E4DC';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(cursorX, 0);
      ctx.lineTo(cursorX, cssH);
      ctx.stroke();
    }
  }

  // 7. Playhead (live playback position)
  const ps = playbackState.value;
  if (ps && (ps.isPlaying || ps.isPaused)) {
    const phX = secondsToX(ps.currentTime, cssW);
    if (phX >= 0 && phX <= cssW) {
      // Vertical line
      ctx.strokeStyle = '#E8E4DC';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(phX, 0);
      ctx.lineTo(phX, cssH);
      ctx.stroke();

      // Small triangle at top
      ctx.fillStyle = '#E8E4DC';
      ctx.beginPath();
      ctx.moveTo(phX - 4, 0);
      ctx.lineTo(phX + 4, 0);
      ctx.lineTo(phX, 6);
      ctx.closePath();
      ctx.fill();
    }
  }
}

function getCanvasXInCssPixels(event: MouseEvent): number {
  const canvas = canvasRef.value;
  if (!canvas) return 0;
  const rect = canvas.getBoundingClientRect();
  return event.clientX - rect.left;
}

function findNearestMarker(x: number): MarkerType {
  const cssW = getCanvasWidth();

  type Candidate = { type: MarkerType; dist: number };
  const candidates: Candidate[] = [];

  const loopStartDist = Math.abs(x - secondsToX(props.loopStart, cssW));
  if (loopStartDist < MARKER_GRAB_THRESHOLD_PX) {
    candidates.push({ type: 'loopStart', dist: loopStartDist });
  }

  const loopEndDist = Math.abs(x - secondsToX(effectiveLoopEnd.value, cssW));
  if (loopEndDist < MARKER_GRAB_THRESHOLD_PX) {
    candidates.push({ type: 'loopEnd', dist: loopEndDist });
  }

  if (candidates.length === 0) {
    return null;
  }

  candidates.sort((a, b) => a.dist - b.dist);
  return candidates[0].type;
}

function clampAndEmit(marker: MarkerType, seconds: number): void {
  const dur = duration.value;
  const ls = props.loopStart;
  const le = effectiveLoopEnd.value;

  let clamped = Math.max(0, Math.min(dur, seconds));
  const round = (v: number) => Math.round(v * 100) / 100;

  const snapThreshold = dur * 0.01;

  switch (marker) {
    case 'loopStart': {
      clamped = Math.max(0, clamped);
      clamped = Math.min(clamped, le - 0.01);
      emit('update:loopStart', round(clamped));
      break;
    }
    case 'loopEnd': {
      if (clamped > dur - snapThreshold) {
        emit('update:loopEnd', 0);
        return;
      }
      clamped = Math.max(clamped, ls + 0.01);
      clamped = Math.min(clamped, dur);
      emit('update:loopEnd', round(clamped));
      break;
    }
  }
}

// Track click vs drag: if mouse doesn't move significantly, treat as click-to-play
const mouseDownX = ref(0);
const mouseDownWasMarker = ref(false);
const PAN_DRAG_THRESHOLD = 4; // px movement before we consider it a drag

function onMouseDown(event: MouseEvent): void {
  if (duration.value <= 0) return;
  const x = getCanvasXInCssPixels(event);
  const marker = findNearestMarker(x);

  mouseDownX.value = event.clientX;

  if (marker) {
    // Drag a marker
    mouseDownWasMarker.value = true;
    dragging.value = marker;
    const cssW = getCanvasWidth();
    clampAndEmit(marker, xToSeconds(x, cssW));
  } else {
    mouseDownWasMarker.value = false;
    if (isZoomed.value) {
      // Start panning (will check in mouseup if it was actually a click)
      panning.value = true;
      panStartX.value = event.clientX;
      panStartViewStart.value = viewStart.value;
    }
  }
}

function onMouseMove(event: MouseEvent): void {
  if (duration.value <= 0) return;

  if (dragging.value) {
    const x = getCanvasXInCssPixels(event);
    const cssW = getCanvasWidth();
    clampAndEmit(dragging.value, xToSeconds(x, cssW));
    return;
  }

  if (panning.value) {
    const cssW = getCanvasWidth();
    const deltaX = event.clientX - panStartX.value;
    const visibleDuration = viewEnd.value - viewStart.value;
    const deltaSec = -(deltaX / cssW) * visibleDuration;
    viewStart.value = panStartViewStart.value + deltaSec;
    clampView();
    drawWaveform();
    return;
  }
}

function onMouseUp(event: MouseEvent): void {
  const wasPanning = panning.value;
  const wasMarker = mouseDownWasMarker.value;
  const dragDistance = Math.abs(event.clientX - mouseDownX.value);

  dragging.value = null;
  panning.value = false;
  mouseDownWasMarker.value = false;

  if (wasPanning && isZoomed.value) {
    if (dragDistance <= PAN_DRAG_THRESHOLD) {
      // It was a click, not a pan drag -- restore view and play from position
      viewStart.value = panStartViewStart.value;
      clampView();
      const x = getCanvasXInCssPixels(event);
      const cssW = getCanvasWidth();
      const clickTime = xToSeconds(x, cssW);
      audio.playTrackFrom(props.trackId, clickTime);
      showPlayCursor(clickTime);
    } else {
      requestZoomedPeaks();
    }
    return;
  }

  // Not zoomed, not a marker drag -- click-to-play
  if (!wasMarker && dragDistance <= PAN_DRAG_THRESHOLD && duration.value > 0) {
    const x = getCanvasXInCssPixels(event);
    const cssW = getCanvasWidth();
    const clickTime = xToSeconds(x, cssW);
    audio.playTrackFrom(props.trackId, clickTime);
    showPlayCursor(clickTime);
  }
}

function onWheel(event: WheelEvent): void {
  if (duration.value <= 0) return;
  event.preventDefault();

  const cssW = getCanvasWidth();
  const x = getCanvasXInCssPixels(event);
  const timeAtCursor = xToSeconds(x, cssW);

  // Determine zoom direction
  const factor = event.deltaY > 0 ? 1 / ZOOM_STEP : ZOOM_STEP;
  let newZoom = zoomLevel.value * factor;
  newZoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, newZoom));

  // If effectively at 1.0, snap to exact 1.0
  if (newZoom < 1.005) newZoom = 1.0;

  const newVisibleDuration = duration.value / newZoom;

  // Keep timeAtCursor at the same fractional position on screen
  const cursorFraction = x / cssW;
  viewStart.value = timeAtCursor - cursorFraction * newVisibleDuration;
  zoomLevel.value = newZoom;
  clampView();
  drawWaveform();
  requestZoomedPeaks();
}

function onDblClick(): void {
  if (duration.value <= 0) return;
  resetView();
  drawWaveform();
  requestZoomedPeaks();
}

function onResize(): void {
  drawWaveform();
}

// Draw when canvas becomes available
watch(canvasRef, (canvas) => {
  if (canvas && basePeaks.value.length > 0) {
    drawWaveform();
  }
});

watch(basePeaks, () => drawWaveform());
watch(zoomedPeaks, () => drawWaveform());
watch(
  () => [props.loopStart, props.loopEnd],
  () => drawWaveform()
);

watch(() => props.filePath, () => {
  resetView();
  loadWaveform();
});

onMounted(() => {
  loadWaveform();
  window.addEventListener('mousemove', onMouseMove);
  window.addEventListener('mouseup', onMouseUp);
  window.addEventListener('resize', onResize);
});

onUnmounted(() => {
  if (zoomDebounce) clearTimeout(zoomDebounce);
  if (playCursorTimeout) clearTimeout(playCursorTimeout);
  stopPlayheadLoop();
  window.removeEventListener('mousemove', onMouseMove);
  window.removeEventListener('mouseup', onMouseUp);
  window.removeEventListener('resize', onResize);
});
</script>

<template>
  <div class="waveform-editor">
    <div class="waveform-container">
      <div v-if="loading" class="waveform-overlay text-caption text-medium-emphasis">
        Loading waveform...
      </div>
      <div v-else-if="error" class="waveform-overlay text-caption text-error">
        {{ error }}
      </div>
      <canvas
        ref="canvasRef"
        class="waveform-canvas"
        :style="{ cursor: canvasCursor }"
        @mousedown="onMouseDown"
        @wheel="onWheel"
        @dblclick="onDblClick"
      />
    </div>
    <div class="waveform-labels d-flex ga-4 mt-1">
      <span class="text-caption text-medium-emphasis">
        Loop: {{ loopStartDisplay }}s - {{ loopEndDisplay }}
      </span>
      <span class="text-caption text-medium-emphasis ml-auto">
        <template v-if="isZoomed">{{ viewRangeDisplay }}</template>
        <template v-else>{{ duration.toFixed(1) }}s</template>
      </span>
    </div>
  </div>
</template>

<style scoped>
.waveform-editor {
  width: 100%;
}

.waveform-container {
  position: relative;
  height: 80px;
}

.waveform-canvas {
  width: 100%;
  height: 80px;
  display: block;
  border-radius: 8px;
  border: 1px solid rgba(232, 228, 220, 0.1);
}

.waveform-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #141418;
  border-radius: 8px;
  border: 1px solid rgba(232, 228, 220, 0.1);
  z-index: 1;
}
</style>
