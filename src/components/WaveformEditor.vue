<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';

interface WaveformData {
  peaks: number[];
  duration: number;
}

const props = defineProps<{
  filePath: string;
  loopStart: number;
  loopEnd: number;
}>();

const emit = defineEmits<{
  'update:loopStart': [value: number];
  'update:loopEnd': [value: number];
}>();

const canvasRef = ref<HTMLCanvasElement | null>(null);

const CANVAS_HEIGHT = 80;
const MARKER_GRAB_THRESHOLD_PX = 12; // in actual screen pixels
const ZOOM_MIN = 1.0;
const ZOOM_MAX = 50.0;
const ZOOM_STEP = 1.15; // multiplier per wheel tick

const peaks = ref<number[]>([]);
const duration = ref(0);
const loading = ref(false);
const error = ref('');

// Zoom & pan state
const zoomLevel = ref(1.0);
const viewStart = ref(0);
const viewEnd = ref(0);

type MarkerType = 'loopStart' | 'loopEnd' | null;
const dragging = ref<MarkerType>(null);
const panning = ref(false);
const panStartX = ref(0);
const panStartViewStart = ref(0);

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
  if (isZoomed.value) return 'grab';
  return 'pointer';
});

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
    // Request peaks at a reasonable resolution
    const data = await invoke<WaveformData>('get_waveform_data', {
      filePath: props.filePath,
      width: 1200,
    });
    peaks.value = data.peaks;
    duration.value = data.duration;
    resetView();
  } catch (e: unknown) {
    error.value = String(e);
    peaks.value = [];
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

  // Background
  ctx.fillStyle = '#141418';
  ctx.fillRect(0, 0, cssW, cssH);

  if (peaks.value.length === 0) return;

  const dur = duration.value;
  const peakCount = peaks.value.length;

  // Only draw peaks within the visible range
  const startPeakIndex = Math.max(0, Math.floor((viewStart.value / dur) * peakCount));
  const endPeakIndex = Math.min(peakCount, Math.ceil((viewEnd.value / dur) * peakCount));

  // Each peak maps to a time; compute its X position in the zoomed view
  const maxBarHeight = midY - 2;

  for (let i = startPeakIndex; i < endPeakIndex; i++) {
    const peak = peaks.value[i];
    const peakTime = (i / peakCount) * dur;
    const nextPeakTime = ((i + 1) / peakCount) * dur;
    const x = secondsToX(peakTime, cssW);
    const xNext = secondsToX(nextPeakTime, cssW);
    const barWidth = xNext - x;
    const barHeight = peak * maxBarHeight;

    ctx.fillStyle = '#F0A830';
    ctx.fillRect(x, midY - barHeight, Math.max(barWidth - 0.5, 0.5), barHeight * 2);
  }

  // Draw loop markers (solid green lines) — only if visible
  const loopStartX = secondsToX(props.loopStart, cssW);
  const loopEndX = secondsToX(effectiveLoopEnd.value, cssW);

  ctx.strokeStyle = '#4CAF50';
  ctx.lineWidth = 2;

  if (loopStartX >= -2 && loopStartX <= cssW + 2) {
    ctx.beginPath();
    ctx.moveTo(loopStartX, 0);
    ctx.lineTo(loopStartX, cssH);
    ctx.stroke();

    // Half-box handle pointing outward
    ctx.fillStyle = '#4CAF50';
    ctx.fillRect(loopStartX - 5, cssH - 8, 5, 8);
  }

  if (loopEndX >= -2 && loopEndX <= cssW + 2) {
    ctx.strokeStyle = '#4CAF50';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(loopEndX, 0);
    ctx.lineTo(loopEndX, cssH);
    ctx.stroke();

    // Half-box handle pointing outward
    ctx.fillStyle = '#4CAF50';
    ctx.fillRect(loopEndX, cssH - 8, 5, 8);
  }

  // Center line
  ctx.strokeStyle = 'rgba(232, 228, 220, 0.15)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(cssW, midY);
  ctx.stroke();
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

function onMouseDown(event: MouseEvent): void {
  if (duration.value <= 0) return;
  const x = getCanvasXInCssPixels(event);
  const marker = findNearestMarker(x);

  if (marker) {
    // Drag a marker
    dragging.value = marker;
    const cssW = getCanvasWidth();
    clampAndEmit(marker, xToSeconds(x, cssW));
  } else if (isZoomed.value) {
    // Start panning
    panning.value = true;
    panStartX.value = event.clientX;
    panStartViewStart.value = viewStart.value;
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

function onMouseUp(): void {
  dragging.value = null;
  panning.value = false;
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
}

function onDblClick(): void {
  if (duration.value <= 0) return;
  resetView();
  drawWaveform();
}

function onResize(): void {
  drawWaveform();
}

// Draw when canvas becomes available
watch(canvasRef, (canvas) => {
  if (canvas && peaks.value.length > 0) {
    drawWaveform();
  }
});

watch(peaks, () => drawWaveform());
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
