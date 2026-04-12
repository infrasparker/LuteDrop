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

const peaks = ref<number[]>([]);
const duration = ref(0);
const loading = ref(false);
const error = ref('');

type MarkerType = 'loopStart' | 'loopEnd' | null;
const dragging = ref<MarkerType>(null);

const effectiveLoopEnd = computed(() => {
  return props.loopEnd === 0 ? duration.value : props.loopEnd;
});

const loopStartDisplay = computed(() => props.loopStart.toFixed(2));
const loopEndDisplay = computed(() => {
  return props.loopEnd === 0 ? 'end' : props.loopEnd.toFixed(2);
});

// Get the actual pixel width of the canvas element
function getCanvasWidth(): number {
  const canvas = canvasRef.value;
  if (!canvas) return 800;
  return canvas.getBoundingClientRect().width;
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
  } catch (e: unknown) {
    error.value = String(e);
    peaks.value = [];
    duration.value = 0;
  } finally {
    loading.value = false;
  }
}

function secondsToX(seconds: number, w: number): number {
  if (duration.value <= 0) return 0;
  return (seconds / duration.value) * w;
}

function xToSeconds(x: number, w: number): number {
  if (duration.value <= 0 || w <= 0) return 0;
  const seconds = (x / w) * duration.value;
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

  // Draw waveform bars
  const barWidth = cssW / peaks.value.length;
  const maxBarHeight = midY - 2;

  for (let i = 0; i < peaks.value.length; i++) {
    const peak = peaks.value[i];
    const barHeight = peak * maxBarHeight;
    const x = i * barWidth;

    ctx.fillStyle = '#F0A830';
    ctx.fillRect(x, midY - barHeight, Math.max(barWidth - 0.5, 0.5), barHeight * 2);
  }

  // Draw loop markers (solid green lines)
  const loopStartX = secondsToX(props.loopStart, cssW);
  const loopEndX = secondsToX(effectiveLoopEnd.value, cssW);

  ctx.strokeStyle = '#4CAF50';
  ctx.lineWidth = 2;

  ctx.beginPath();
  ctx.moveTo(loopStartX, 0);
  ctx.lineTo(loopStartX, cssH);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(loopEndX, 0);
  ctx.lineTo(loopEndX, cssH);
  ctx.stroke();

  // Half-box handles pointing outward
  ctx.fillStyle = '#4CAF50';
  ctx.fillRect(loopStartX - 5, cssH - 8, 5, 8);
  ctx.fillRect(loopEndX, cssH - 8, 5, 8);

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
  dragging.value = findNearestMarker(x);

  if (dragging.value) {
    const cssW = getCanvasWidth();
    clampAndEmit(dragging.value, xToSeconds(x, cssW));
  }
}

function onMouseMove(event: MouseEvent): void {
  if (!dragging.value || duration.value <= 0) return;
  const x = getCanvasXInCssPixels(event);
  const cssW = getCanvasWidth();
  clampAndEmit(dragging.value, xToSeconds(x, cssW));
}

function onMouseUp(): void {
  dragging.value = null;
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

watch(() => props.filePath, () => loadWaveform());

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
        :style="{ cursor: dragging ? 'grabbing' : 'pointer' }"
        @mousedown="onMouseDown"
      />
    </div>
    <div class="waveform-labels d-flex ga-4 mt-1">
      <span class="text-caption text-medium-emphasis">
        Loop: {{ loopStartDisplay }}s - {{ loopEndDisplay }}
      </span>
      <span class="text-caption text-medium-emphasis ml-auto">
        {{ duration.toFixed(1) }}s
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
