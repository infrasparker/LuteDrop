import { ref, computed, type Ref } from 'vue';

const ZOOM_MIN = 1.0;
const ZOOM_MAX = 50.0;
const ZOOM_STEP = 1.15;

export function useWaveformZoom(duration: Ref<number>) {
  const zoomLevel = ref(1.0);
  const viewStart = ref(0);
  const viewEnd = ref(0);

  const isZoomed = computed(() => zoomLevel.value > 1.0);

  function resetView(): void {
    zoomLevel.value = 1.0;
    viewStart.value = 0;
    viewEnd.value = duration.value;
  }

  function clampView(): void {
    const dur = duration.value;
    if (dur <= 0) return;

    const visibleDuration = dur / zoomLevel.value;

    if (viewStart.value < 0) viewStart.value = 0;
    if (viewStart.value + visibleDuration > dur) {
      viewStart.value = dur - visibleDuration;
    }
    if (viewStart.value < 0) viewStart.value = 0;

    viewEnd.value = viewStart.value + visibleDuration;
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

  /** Apply a zoom step centered on a screen position. */
  function applyZoom(cssW: number, cursorX: number, zoomIn: boolean): void {
    const timeAtCursor = xToSeconds(cursorX, cssW);

    const factor = zoomIn ? ZOOM_STEP : 1 / ZOOM_STEP;
    let newZoom = zoomLevel.value * factor;
    newZoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, newZoom));
    if (newZoom < 1.005) newZoom = 1.0;

    const newVisibleDuration = duration.value / newZoom;
    const cursorFraction = cursorX / cssW;
    viewStart.value = timeAtCursor - cursorFraction * newVisibleDuration;
    zoomLevel.value = newZoom;
    clampView();
  }

  return {
    zoomLevel,
    viewStart,
    viewEnd,
    isZoomed,
    resetView,
    clampView,
    secondsToX,
    xToSeconds,
    applyZoom,
  };
}
