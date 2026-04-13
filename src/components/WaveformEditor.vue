<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, computed, toRef } from 'vue';
import { useAudio } from '../composables/useAudio';
import { useAppStore } from '../stores/appStore';
import { useWaveformData } from '../composables/useWaveformData';
import { useWaveformZoom } from '../composables/useWaveformZoom';

const props = defineProps<{
  filePath: string;
  loopStart: number;
  loopEnd: number;
  trimStart: number | null;
  trimEnd: number | null;
  trackId?: string;
  loopEnabled?: boolean;
}>();

const emit = defineEmits<{
  'update:loopStart': [value: number];
  'update:loopEnd': [value: number];
  'update:trimStart': [value: number | null];
  'update:trimEnd': [value: number | null];
  'commit': [values: { trimStart: number | null; trimEnd: number | null; loopStart: number; loopEnd: number }];
}>();

const audio = useAudio();
const store = useAppStore();

const canvasRef = ref<HTMLCanvasElement | null>(null);

const CANVAS_HEIGHT = 80;
const MARKER_GRAB_THRESHOLD_PX = 7;

// --- Composables ---

const data = useWaveformData(toRef(props, 'filePath'));
const zoom = useWaveformZoom(data.duration);

// --- Local marker refs (updated during drag; props updated only on mouseup) ---

const localLoopStart = ref(props.loopStart);
const localLoopEnd = ref(props.loopEnd);
const localTrimStart = ref<number | null>(props.trimStart);
const localTrimEnd = ref<number | null>(props.trimEnd);

watch(() => props.loopStart, (v) => { if (!dragging.value) localLoopStart.value = v; });
watch(() => props.loopEnd, (v) => { if (!dragging.value) localLoopEnd.value = v; });
watch(() => props.trimStart, (v) => { if (!dragging.value) localTrimStart.value = v; });
watch(() => props.trimEnd, (v) => { if (!dragging.value) localTrimEnd.value = v; });

// --- Playhead ---

let rafId: number | null = null;

const playbackState = computed(() => {
  if (props.trackId) return store.playbackStates.get(props.trackId);
  return store.playbackStates.get('__preview');
});

function startPlayheadLoop(): void {
  if (rafId !== null) return;
  function tick() {
    drawWaveform();
    if (playbackState.value?.isPlaying) {
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
    drawWaveform();
  }
}, { immediate: true });

// --- Derived state ---

const effectiveLoopEnd = computed(() => {
  return localLoopEnd.value === 0 ? data.duration.value : localLoopEnd.value;
});

const effectiveTrimStart = computed(() => localTrimStart.value ?? 0);
const effectiveTrimEnd = computed(() => localTrimEnd.value ?? data.duration.value);

const loopStartDisplay = computed(() => localLoopStart.value.toFixed(2));
const loopEndDisplay = computed(() => {
  return localLoopEnd.value === 0 ? 'end' : localLoopEnd.value.toFixed(2);
});

const viewRangeDisplay = computed(() => {
  if (!zoom.isZoomed.value) return '';
  return `Viewing: ${zoom.viewStart.value.toFixed(1)}s – ${zoom.viewEnd.value.toFixed(1)}s / ${data.duration.value.toFixed(1)}s`;
});

const canvasCursor = computed(() => {
  if (dragging.value) return 'grabbing';
  if (panning.value) return 'grabbing';
  return 'pointer';
});

// --- Canvas helpers ---

function getCanvasWidth(): number {
  const canvas = canvasRef.value;
  if (!canvas) return 800;
  return canvas.getBoundingClientRect().width;
}

function getCanvasXInCssPixels(event: MouseEvent): number {
  const canvas = canvasRef.value;
  if (!canvas) return 0;
  return event.clientX - canvas.getBoundingClientRect().left;
}

function fireZoomedPeaks(): void {
  const canvasPixelWidth = Math.round(getCanvasWidth() * (window.devicePixelRatio || 1));
  data.requestZoomedPeaks(
    zoom.isZoomed.value,
    zoom.viewStart.value,
    zoom.viewEnd.value,
    canvasPixelWidth,
  );
}

// --- Canvas drawing ---

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

  const startIdx = Math.max(0, Math.floor(((zoom.viewStart.value - rangeStart) / pDuration) * peakCount));
  const endIdx = Math.min(peakCount, Math.ceil(((zoom.viewEnd.value - rangeStart) / pDuration) * peakCount));

  if (startIdx >= endIdx) return;

  ctx.fillStyle = color;

  // Top half
  ctx.beginPath();
  const x0 = zoom.secondsToX(rangeStart + (startIdx / peakCount) * pDuration, cssW);
  ctx.moveTo(x0, midY);
  for (let i = startIdx; i < endIdx; i++) {
    const peakTime = rangeStart + (i / peakCount) * pDuration;
    const x = zoom.secondsToX(peakTime, cssW);
    ctx.lineTo(x, midY - peaksArr[i] * maxBarHeight);
  }
  const xLast = zoom.secondsToX(rangeStart + ((endIdx - 1) / peakCount) * pDuration, cssW);
  ctx.lineTo(xLast, midY);
  ctx.closePath();
  ctx.fill();

  // Bottom half (mirror)
  ctx.beginPath();
  ctx.moveTo(x0, midY);
  for (let i = startIdx; i < endIdx; i++) {
    const peakTime = rangeStart + (i / peakCount) * pDuration;
    const x = zoom.secondsToX(peakTime, cssW);
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

  const displayWidth = canvas.getBoundingClientRect().width;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(displayWidth * dpr);
  canvas.height = Math.round(CANVAS_HEIGHT * dpr);
  ctx.scale(dpr, dpr);

  const cssW = displayWidth;
  const cssH = CANVAS_HEIGHT;
  const midY = cssH / 2;

  // Background
  ctx.fillStyle = '#141418';
  ctx.fillRect(0, 0, cssW, cssH);

  if (data.basePeaks.value.length === 0) return;

  // Waveform peaks
  if (zoom.isZoomed.value) {
    if (data.zoomedPeaks.value.length > 0) {
      const zoomStartX = zoom.secondsToX(data.zoomedPeaksRangeStart.value, cssW);
      const zoomEndX = zoom.secondsToX(data.zoomedPeaksRangeEnd.value, cssW);

      if (zoomStartX > 0) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(0, 0, zoomStartX, cssH);
        ctx.clip();
        drawPeaksPath(ctx, data.basePeaks.value, data.basePeaksRangeStart.value, data.basePeaksRangeEnd.value, cssW, cssH, '#7A5418');
        ctx.restore();
      }

      if (zoomEndX < cssW) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(zoomEndX, 0, cssW - zoomEndX, cssH);
        ctx.clip();
        drawPeaksPath(ctx, data.basePeaks.value, data.basePeaksRangeStart.value, data.basePeaksRangeEnd.value, cssW, cssH, '#7A5418');
        ctx.restore();
      }

      drawPeaksPath(ctx, data.zoomedPeaks.value, data.zoomedPeaksRangeStart.value, data.zoomedPeaksRangeEnd.value, cssW, cssH, '#F0A830');
    } else {
      drawPeaksPath(ctx, data.basePeaks.value, data.basePeaksRangeStart.value, data.basePeaksRangeEnd.value, cssW, cssH, '#7A5418');
    }
  } else {
    drawPeaksPath(ctx, data.basePeaks.value, data.basePeaksRangeStart.value, data.basePeaksRangeEnd.value, cssW, cssH, '#F0A830');
  }

  // Trim dimmed overlay and markers — always draw using effective (defaulted) values
  {
    const TRIM_COLOR = '#FF6D00';
    const ts = effectiveTrimStart.value;
    const te = effectiveTrimEnd.value;

    // Dimmed region before trim start
    const tsX = zoom.secondsToX(ts, cssW);
    const leftX = zoom.secondsToX(0, cssW);
    if (tsX > leftX) {
      ctx.fillStyle = 'rgba(0, 0, 0, 0.45)';
      ctx.fillRect(leftX, 0, tsX - leftX, cssH);
    }

    // Dimmed region after trim end
    const teX = zoom.secondsToX(te, cssW);
    const rightX = zoom.secondsToX(data.duration.value, cssW);
    if (rightX > teX) {
      ctx.fillStyle = 'rgba(0, 0, 0, 0.45)';
      ctx.fillRect(teX, 0, rightX - teX, cssH);
    }

    // Trim start marker
    if (tsX >= -2 && tsX <= cssW + 2) {
      ctx.strokeStyle = TRIM_COLOR;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(tsX, 0);
      ctx.lineTo(tsX, cssH);
      ctx.stroke();
      ctx.fillStyle = TRIM_COLOR;
      ctx.fillRect(tsX - 5, cssH - 8, 5, 8);
    }

    // Trim end marker
    if (teX >= -2 && teX <= cssW + 2) {
      ctx.strokeStyle = TRIM_COLOR;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(teX, 0);
      ctx.lineTo(teX, cssH);
      ctx.stroke();
      ctx.fillStyle = TRIM_COLOR;
      ctx.fillRect(teX, cssH - 8, 5, 8);
    }
  }

  // Loop markers — only draw if loop is enabled
  if (props.loopEnabled !== false) {
    const loopStartX = zoom.secondsToX(localLoopStart.value, cssW);
    const drawLoopEnd = localLoopEnd.value === 0 ? data.duration.value : localLoopEnd.value;
    const loopEndX = zoom.secondsToX(drawLoopEnd, cssW);

    ctx.strokeStyle = '#4CAF50';
    ctx.lineWidth = 2;

    if (loopStartX >= -2 && loopStartX <= cssW + 2) {
      ctx.beginPath();
      ctx.moveTo(loopStartX, 0);
      ctx.lineTo(loopStartX, cssH);
      ctx.stroke();
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
      ctx.fillStyle = '#4CAF50';
      ctx.fillRect(loopEndX, 0, 5, 8);
    }
  }

  // Center line
  ctx.strokeStyle = 'rgba(232, 228, 220, 0.15)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(cssW, midY);
  ctx.stroke();

  // Playhead
  const ps = playbackState.value;
  if (ps && (ps.isPlaying || ps.isPaused)) {
    const phX = zoom.secondsToX(ps.currentTime, cssW);
    if (phX >= 0 && phX <= cssW) {
      ctx.strokeStyle = '#E8E4DC';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(phX, 0);
      ctx.lineTo(phX, cssH);
      ctx.stroke();

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

// --- Marker interaction ---

type MarkerType = 'loopStart' | 'loopEnd' | 'trimStart' | 'trimEnd' | null;
const dragging = ref<MarkerType>(null);
const panning = ref(false);
const panStartX = ref(0);
const panStartViewStart = ref(0);
const mouseDownX = ref(0);
const mouseDownWasMarker = ref(false);
const PAN_DRAG_THRESHOLD = 4;

function findNearestMarker(x: number): MarkerType {
  const cssW = getCanvasWidth();

  type Candidate = { type: MarkerType; dist: number };
  const candidates: Candidate[] = [];

  const trimStartDist = Math.abs(x - zoom.secondsToX(effectiveTrimStart.value, cssW));
  if (trimStartDist < MARKER_GRAB_THRESHOLD_PX) {
    candidates.push({ type: 'trimStart', dist: trimStartDist });
  }

  const trimEndDist = Math.abs(x - zoom.secondsToX(effectiveTrimEnd.value, cssW));
  if (trimEndDist < MARKER_GRAB_THRESHOLD_PX) {
    candidates.push({ type: 'trimEnd', dist: trimEndDist });
  }

  if (props.loopEnabled !== false) {
    const loopStartDist = Math.abs(x - zoom.secondsToX(localLoopStart.value, cssW));
    if (loopStartDist < MARKER_GRAB_THRESHOLD_PX) {
      candidates.push({ type: 'loopStart', dist: loopStartDist });
    }

    const loopEndDist = Math.abs(x - zoom.secondsToX(effectiveLoopEnd.value, cssW));
    if (loopEndDist < MARKER_GRAB_THRESHOLD_PX) {
      candidates.push({ type: 'loopEnd', dist: loopEndDist });
    }
  }

  if (candidates.length === 0) return null;
  candidates.sort((a, b) => a.dist - b.dist);
  return candidates[0].type;
}

/** Update local refs during drag — no emit, no DB write. */
function clampLocal(marker: MarkerType, seconds: number): void {
  const dur = data.duration.value;
  const ts = effectiveTrimStart.value;
  const te = effectiveTrimEnd.value;

  let clamped = Math.max(0, Math.min(dur, seconds));
  const round = (v: number) => Math.round(v * 100) / 100;
  const snapThreshold = dur * 0.01;

  switch (marker) {
    case 'trimStart': {
      clamped = Math.max(0, clamped);
      clamped = Math.min(clamped, te - 0.01);
      const rounded = round(clamped);

      // Apply all at once
      localTrimStart.value = rounded;

      if (props.loopEnabled !== false) {
        // Calculate all new values first
        let newLoopStart = localLoopStart.value;
        let newLoopEnd = localLoopEnd.value;

        if (rounded > newLoopStart) {
          newLoopStart = rounded;
        }
        // Push loop end if loop start catches up
        const curLoopEnd = newLoopEnd === 0 ? dur : newLoopEnd;
        if (newLoopStart >= curLoopEnd - 0.01) {
          newLoopEnd = round(Math.min(newLoopStart + 0.01, te));
        }

        localLoopStart.value = newLoopStart;
        localLoopEnd.value = newLoopEnd;
      }
      break;
    }
    case 'trimEnd': {
      clamped = Math.max(ts + 0.01, clamped);
      clamped = Math.min(clamped, dur);
      const rounded = round(clamped);

      localTrimEnd.value = rounded;

      if (props.loopEnabled !== false) {
        let newLoopStart = localLoopStart.value;
        let newLoopEnd = localLoopEnd.value;

        const curLoopEnd = newLoopEnd === 0 ? dur : newLoopEnd;
        if (rounded < curLoopEnd) {
          newLoopEnd = round(rounded);
        }
        if (newLoopEnd !== 0 && newLoopEnd <= newLoopStart + 0.01) {
          newLoopStart = round(Math.max(newLoopEnd - 0.01, ts));
        }

        localLoopStart.value = newLoopStart;
        localLoopEnd.value = newLoopEnd;
      }
      break;
    }
    case 'loopStart': {
      const curLoopEnd2 = localLoopEnd.value === 0 ? dur : localLoopEnd.value;
      clamped = Math.max(ts, clamped);
      clamped = Math.min(clamped, curLoopEnd2 - 0.01);
      localLoopStart.value = round(clamped);
      break;
    }
    case 'loopEnd': {
      if (clamped > dur - snapThreshold) {
        localLoopEnd.value = 0;
        return;
      }
      clamped = Math.max(clamped, localLoopStart.value + 0.01);
      clamped = Math.min(clamped, te);
      localLoopEnd.value = round(clamped);
      break;
    }
  }
}

/** Emit final values on mouseup (persistence) as a single batch. */
function emitFinalValues(): void {
  const dur = data.duration.value;

  // Trim: emit null if value equals the default bound (0 for start, duration for end)
  const tsVal = localTrimStart.value;
  const teVal = localTrimEnd.value;
  const trimStart = (tsVal === null || tsVal === 0) ? null : tsVal;
  const trimEnd = (teVal !== null && dur > 0 && teVal >= dur) ? null : teVal;

  // If both loop values are 0, emit them as 0 (backend treats as "no loop")
  const loopStart = localLoopStart.value;
  const loopEnd = localLoopEnd.value;

  emit('commit', {
    trimStart,
    trimEnd,
    loopStart,
    loopEnd,
  });
}

// --- Mouse events ---

function onMouseDown(event: MouseEvent): void {
  if (data.duration.value <= 0) return;
  const x = getCanvasXInCssPixels(event);
  const marker = findNearestMarker(x);

  mouseDownX.value = event.clientX;

  if (marker) {
    mouseDownWasMarker.value = true;
    dragging.value = marker;
    clampLocal(marker, zoom.xToSeconds(x, getCanvasWidth()));
    drawWaveform();
  } else {
    mouseDownWasMarker.value = false;
    if (zoom.isZoomed.value) {
      panning.value = true;
      panStartX.value = event.clientX;
      panStartViewStart.value = zoom.viewStart.value;
    }
  }
}

function onMouseMove(event: MouseEvent): void {
  if (data.duration.value <= 0) return;

  if (dragging.value) {
    const x = getCanvasXInCssPixels(event);
    clampLocal(dragging.value, zoom.xToSeconds(x, getCanvasWidth()));
    drawWaveform();
    return;
  }

  if (panning.value) {
    const cssW = getCanvasWidth();
    const deltaX = event.clientX - panStartX.value;
    const visibleDuration = zoom.viewEnd.value - zoom.viewStart.value;
    const deltaSec = -(deltaX / cssW) * visibleDuration;
    zoom.viewStart.value = panStartViewStart.value + deltaSec;
    zoom.clampView();
    drawWaveform();
  }
}

function onMouseUp(event: MouseEvent): void {
  const wasPanning = panning.value;
  const wasMarker = mouseDownWasMarker.value;
  const dragDistance = Math.abs(event.clientX - mouseDownX.value);

  dragging.value = null;
  panning.value = false;
  mouseDownWasMarker.value = false;

  if (wasMarker) {
    emitFinalValues();
    return;
  }

  if (wasPanning && zoom.isZoomed.value) {
    if (dragDistance <= PAN_DRAG_THRESHOLD) {
      zoom.viewStart.value = panStartViewStart.value;
      zoom.clampView();
      const clickTime = zoom.xToSeconds(getCanvasXInCssPixels(event), getCanvasWidth());
      if (props.trackId) {
        audio.playTrackFrom(props.trackId, clickTime);
      } else if (props.filePath) {
        audio.playPreview(
          props.filePath, clickTime, 1.0,
          localTrimStart.value, localTrimEnd.value,
          props.loopEnabled !== false ? (localLoopStart.value === 0 ? null : localLoopStart.value) : null,
          props.loopEnabled !== false ? (localLoopEnd.value === 0 ? null : localLoopEnd.value) : null,
        );
      }
    } else {
      fireZoomedPeaks();
    }
    return;
  }

  if (!wasMarker && dragDistance <= PAN_DRAG_THRESHOLD && data.duration.value > 0) {
    const clickTime = zoom.xToSeconds(getCanvasXInCssPixels(event), getCanvasWidth());
    if (props.trackId) {
      audio.playTrackFrom(props.trackId, clickTime);
    } else if (props.filePath) {
      audio.playPreview(
        props.filePath, clickTime, 1.0,
        localTrimStart.value, localTrimEnd.value,
        props.loopEnabled !== false ? (localLoopStart.value === 0 ? null : localLoopStart.value) : null,
        props.loopEnabled !== false ? (localLoopEnd.value === 0 ? null : localLoopEnd.value) : null,
      );
    }
  }
}

function onWheel(event: WheelEvent): void {
  if (data.duration.value <= 0) return;
  event.preventDefault();

  const x = getCanvasXInCssPixels(event);
  zoom.applyZoom(getCanvasWidth(), x, event.deltaY < 0);
  drawWaveform();
  fireZoomedPeaks();
}

function onDblClick(): void {
  if (data.duration.value <= 0) return;
  zoom.resetView();
  drawWaveform();
  fireZoomedPeaks();
}

// --- Lifecycle ---

watch(canvasRef, (canvas) => {
  if (canvas && data.basePeaks.value.length > 0) drawWaveform();
});
watch(data.basePeaks, () => {
  if (data.basePeaks.value.length > 0) zoom.resetView();
  drawWaveform();
});
watch(data.zoomedPeaks, () => drawWaveform());
watch(() => [props.loopStart, props.loopEnd, props.trimStart, props.trimEnd], () => drawWaveform());

watch(() => props.filePath, () => {
  localLoopStart.value = props.loopStart;
  localLoopEnd.value = props.loopEnd;
  localTrimStart.value = props.trimStart;
  localTrimEnd.value = props.trimEnd;
  zoom.resetView();
  data.loadBasePeaks();
});

onMounted(() => {
  data.loadBasePeaks();
  window.addEventListener('mousemove', onMouseMove);
  window.addEventListener('mouseup', onMouseUp);
  window.addEventListener('resize', drawWaveform);
});

onUnmounted(() => {
  data.cleanup();
  stopPlayheadLoop();
  window.removeEventListener('mousemove', onMouseMove);
  window.removeEventListener('mouseup', onMouseUp);
  window.removeEventListener('resize', drawWaveform);
});
</script>

<template>
  <div class="waveform-editor">
    <div class="waveform-container">
      <div v-if="data.loading.value" class="waveform-overlay">
        <div style="width: 60%; text-align: center;">
          <VProgressLinear indeterminate color="primary" rounded height="4" />
          <div class="text-caption text-medium-emphasis mt-2">Loading waveform...</div>
        </div>
      </div>
      <div v-else-if="data.error.value" class="waveform-overlay text-caption text-error">
        {{ data.error.value }}
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
      <span v-if="localTrimStart !== null || localTrimEnd !== null" class="text-caption" style="color: #FF6D00;">
        Trim: {{ localTrimStart !== null ? localTrimStart.toFixed(2) + 's' : 'start' }} - {{ localTrimEnd !== null ? localTrimEnd.toFixed(2) + 's' : 'end' }}
      </span>
      <span class="text-caption text-medium-emphasis">
        Loop: {{ loopStartDisplay }}s - {{ loopEndDisplay }}
      </span>
      <span class="text-caption text-medium-emphasis ml-auto">
        <template v-if="zoom.isZoomed.value">{{ viewRangeDisplay }}</template>
        <template v-else>{{ data.duration.value.toFixed(1) }}s</template>
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
