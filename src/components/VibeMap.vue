<script setup lang="ts">
// Canvas scatter of every analyzed music track. Points are colored by Role;
// hovering shows the name, a click plays the track, and dragging draws a
// lasso whose enclosed points are emitted for selection.
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useTheme } from 'vuetify';
import type { LibraryEntry, VibePoint } from '@generated/index';

const props = defineProps<{
  points: VibePoint[];
  entriesById: Map<string, LibraryEntry>;
  seedIds: string[];
  resultIds: string[];
  selectedIds: string[];
  playingId: string | null;
}>();

const emit = defineEmits<{
  play: [entryId: string];
  lasso: [entryIds: string[]];
}>();

// Inset so points on the [0, 1] edges are not clipped.
const PAD = 14;
// Nearest point within this many CSS pixels counts as a hit.
const HIT_RADIUS = 6;
// Movement beyond this many CSS pixels turns a press into a lasso drag.
const DRAG_THRESHOLD = 4;
const BASE_RADIUS = 3;

const theme = useTheme();
const wrapper = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const size = ref({ width: 0, height: 0 });

const hover = ref<{ id: string; x: number; y: number } | null>(null);
const lasso = ref<{ x: number; y: number }[]>([]);
let pressStart: { x: number; y: number } | null = null;
let lassoing = false;

const seedSet = computed(() => new Set(props.seedIds));
const resultSet = computed(() => new Set(props.resultIds));
const selectedSet = computed(() => new Set(props.selectedIds));

interface ScreenPoint { id: string; x: number; y: number }

// Points in CSS pixels for the current canvas size.
const screenPoints = computed<ScreenPoint[]>(() => {
  const w = Math.max(0, size.value.width - 2 * PAD);
  const h = Math.max(0, size.value.height - 2 * PAD);
  return props.points.map((p) => ({ id: p.entryId, x: PAD + p.x * w, y: PAD + (1 - p.y) * h }));
});

// --- Colors from the active Vuetify theme ---

type Rgb = [number, number, number];

// Theme colors are usually '#rrggbb' strings, but the theme type also allows
// numbers and RGB objects.
function toRgb(value: unknown, fallback: Rgb): Rgb {
  if (typeof value === 'number') return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
  if (typeof value === 'string') {
    const m = /^#?([0-9a-f]{6})/i.exec(value);
    if (m) return toRgb(parseInt(m[1], 16), fallback);
    const short = /^#?([0-9a-f]{3})$/i.exec(value);
    if (short) return toRgb(parseInt(short[1].replace(/./g, (d) => d + d), 16), fallback);
    return fallback;
  }
  if (value && typeof value === 'object' && 'r' in value && 'g' in value && 'b' in value) {
    const { r, g, b } = value as { r: number; g: number; b: number };
    return [r, g, b];
  }
  return fallback;
}

function rgba(rgb: Rgb, alpha = 1): string {
  return `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${alpha})`;
}

const palette = computed(() => {
  const c = theme.current.value.colors;
  const dark = theme.current.value.dark;
  const ink = toRgb(c['on-surface'], dark ? [232, 228, 220] : [33, 33, 33]);
  return {
    combat: toRgb(c.error, [211, 47, 47]),
    nonCombat: toRgb(c.primary, [240, 168, 48]),
    unset: ink,
    ink,
    accent: toRgb(c.primary, [240, 168, 48]),
  };
});

function roleColor(entry: LibraryEntry | undefined): Rgb {
  const p = palette.value;
  if (entry?.role === 'combat') return p.combat;
  if (entry?.role === 'nonCombat') return p.nonCombat;
  return p.unset;
}

const legend = computed(() => [
  { label: 'Combat', color: rgba(palette.value.combat) },
  { label: 'Non-combat', color: rgba(palette.value.nonCombat) },
  { label: 'No role', color: rgba(palette.value.unset, 0.45) },
]);

// --- Drawing ---

let frame = 0;
function scheduleDraw(): void {
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    draw();
  });
}

function draw(): void {
  const el = canvas.value;
  if (!el) return;
  const ctx = el.getContext('2d');
  if (!ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const { width, height } = size.value;
  if (el.width !== Math.round(width * dpr) || el.height !== Math.round(height * dpr)) {
    el.width = Math.round(width * dpr);
    el.height = Math.round(height * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);

  const p = palette.value;
  const focused = seedSet.value.size > 0 || resultSet.value.size > 0;
  const pts = screenPoints.value;

  // Base layer: every point, dimmed while results or seeds are shown.
  for (const pt of pts) {
    const entry = props.entriesById.get(pt.id);
    const alpha = entry?.role ? 0.85 : 0.4;
    const dim = focused && !resultSet.value.has(pt.id) && !seedSet.value.has(pt.id);
    ctx.fillStyle = rgba(roleColor(entry), dim ? alpha * 0.35 : alpha);
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, BASE_RADIUS, 0, Math.PI * 2);
    ctx.fill();
  }

  // Current results: thin ring.
  ctx.lineWidth = 1.5;
  ctx.strokeStyle = rgba(p.ink, 0.9);
  for (const pt of pts) {
    if (!resultSet.value.has(pt.id)) continue;
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, BASE_RADIUS + 2.5, 0, Math.PI * 2);
    ctx.stroke();
  }

  // Selection: thick ring.
  ctx.lineWidth = 2.5;
  ctx.strokeStyle = rgba(p.accent);
  for (const pt of pts) {
    if (!selectedSet.value.has(pt.id)) continue;
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, BASE_RADIUS + 4, 0, Math.PI * 2);
    ctx.stroke();
  }

  // Seeds: filled diamond with an outline.
  for (const pt of pts) {
    if (!seedSet.value.has(pt.id)) continue;
    const r = 7;
    ctx.beginPath();
    ctx.moveTo(pt.x, pt.y - r);
    ctx.lineTo(pt.x + r, pt.y);
    ctx.lineTo(pt.x, pt.y + r);
    ctx.lineTo(pt.x - r, pt.y);
    ctx.closePath();
    ctx.fillStyle = rgba(roleColor(props.entriesById.get(pt.id)));
    ctx.fill();
    ctx.lineWidth = 2;
    ctx.strokeStyle = rgba(p.ink);
    ctx.stroke();
  }

  // Playing and hovered points.
  for (const id of [props.playingId, hover.value?.id]) {
    if (!id) continue;
    const pt = pts.find((q) => q.id === id);
    if (!pt) continue;
    ctx.lineWidth = 2;
    ctx.strokeStyle = rgba(p.ink);
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, BASE_RADIUS + 6, 0, Math.PI * 2);
    ctx.stroke();
  }

  // Lasso polygon while dragging.
  if (lasso.value.length > 1) {
    ctx.beginPath();
    ctx.moveTo(lasso.value[0].x, lasso.value[0].y);
    for (const q of lasso.value.slice(1)) ctx.lineTo(q.x, q.y);
    ctx.closePath();
    ctx.fillStyle = rgba(p.accent, 0.12);
    ctx.fill();
    ctx.setLineDash([5, 4]);
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = rgba(p.accent);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}

watch(
  [screenPoints, () => props.entriesById, seedSet, resultSet, selectedSet, () => props.playingId, hover, lasso, palette],
  scheduleDraw,
  { deep: false },
);

// --- Hit testing ---

function nearest(x: number, y: number): ScreenPoint | null {
  let best: ScreenPoint | null = null;
  let bestDist = HIT_RADIUS * HIT_RADIUS;
  for (const pt of screenPoints.value) {
    const d = (pt.x - x) ** 2 + (pt.y - y) ** 2;
    if (d <= bestDist) {
      bestDist = d;
      best = pt;
    }
  }
  return best;
}

// Ray casting: count edge crossings of a horizontal ray from (x, y).
function insidePolygon(x: number, y: number, poly: { x: number; y: number }[]): boolean {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const a = poly[i];
    const b = poly[j];
    if ((a.y > y) !== (b.y > y) && x < ((b.x - a.x) * (y - a.y)) / (b.y - a.y) + a.x) {
      inside = !inside;
    }
  }
  return inside;
}

// --- Pointer handling ---

function localPos(e: PointerEvent): { x: number; y: number } {
  const rect = canvas.value!.getBoundingClientRect();
  return { x: e.clientX - rect.left, y: e.clientY - rect.top };
}

function onPointerDown(e: PointerEvent): void {
  if (e.button !== 0) return;
  canvas.value?.setPointerCapture(e.pointerId);
  pressStart = localPos(e);
  lassoing = false;
  lasso.value = [];
}

function onPointerMove(e: PointerEvent): void {
  const pos = localPos(e);
  if (pressStart) {
    if (!lassoing && Math.hypot(pos.x - pressStart.x, pos.y - pressStart.y) > DRAG_THRESHOLD) {
      lassoing = true;
      hover.value = null;
      lasso.value = [pressStart];
    }
    if (lassoing) {
      const last = lasso.value[lasso.value.length - 1];
      if (Math.hypot(pos.x - last.x, pos.y - last.y) >= 3) lasso.value = [...lasso.value, pos];
      return;
    }
  }
  const hit = nearest(pos.x, pos.y);
  hover.value = hit ? { id: hit.id, x: hit.x, y: hit.y } : null;
}

function onPointerUp(e: PointerEvent): void {
  if (!pressStart) return;
  const start = pressStart;
  pressStart = null;
  canvas.value?.releasePointerCapture(e.pointerId);
  if (lassoing) {
    const poly = lasso.value;
    lassoing = false;
    lasso.value = [];
    if (poly.length >= 3) {
      const ids = screenPoints.value.filter((pt) => insidePolygon(pt.x, pt.y, poly)).map((pt) => pt.id);
      if (ids.length > 0) emit('lasso', ids);
    }
    return;
  }
  const hit = nearest(start.x, start.y);
  if (hit) emit('play', hit.id);
}

function onPointerCancel(): void {
  pressStart = null;
  lassoing = false;
  lasso.value = [];
}

function onPointerLeave(): void {
  if (!pressStart) hover.value = null;
}

const hoverName = computed(() => {
  if (!hover.value) return '';
  return props.entriesById.get(hover.value.id)?.name ?? '';
});

// Keep the tooltip inside the canvas horizontally.
const tooltipStyle = computed(() => {
  if (!hover.value) return {};
  const left = Math.min(Math.max(hover.value.x, 80), Math.max(80, size.value.width - 80));
  const above = hover.value.y > 36;
  return {
    left: `${left}px`,
    top: `${above ? hover.value.y - 12 : hover.value.y + 14}px`,
    transform: above ? 'translate(-50%, -100%)' : 'translate(-50%, 0)',
  };
});

// --- Size tracking ---

let observer: ResizeObserver | null = null;
let dprQuery: MediaQueryList | null = null;

function measure(): void {
  const el = wrapper.value;
  if (!el) return;
  size.value = { width: el.clientWidth, height: el.clientHeight };
  scheduleDraw();
}

// Redraw at the new resolution when the window moves to a display with a
// different pixel ratio.
function watchPixelRatio(): void {
  dprQuery?.removeEventListener('change', watchPixelRatio);
  dprQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
  dprQuery.addEventListener('change', watchPixelRatio);
  scheduleDraw();
}

onMounted(() => {
  observer = new ResizeObserver(measure);
  if (wrapper.value) observer.observe(wrapper.value);
  measure();
  watchPixelRatio();
});

onBeforeUnmount(() => {
  observer?.disconnect();
  dprQuery?.removeEventListener('change', watchPixelRatio);
  if (frame) cancelAnimationFrame(frame);
});
</script>

<template>
  <div class="vibe-map d-flex flex-column">
    <div ref="wrapper" class="vibe-map-canvas-wrap flex-grow-1">
      <canvas
        ref="canvas"
        class="vibe-map-canvas"
        :class="{ 'is-hovering': !!hover }"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="onPointerCancel"
        @pointerleave="onPointerLeave"
      />
      <div v-if="hover && hoverName" class="vibe-map-tooltip text-caption" :style="tooltipStyle">
        {{ hoverName }}
      </div>
    </div>
    <div class="d-flex flex-wrap align-center ga-3 px-2 pt-2 text-caption text-medium-emphasis">
      <span v-for="item in legend" :key="item.label" class="d-inline-flex align-center ga-1">
        <span class="swatch" :style="{ background: item.color }" />{{ item.label }}
      </span>
      <span class="d-inline-flex align-center ga-1"><span class="swatch-diamond" />Seed</span>
      <span class="d-inline-flex align-center ga-1"><span class="swatch-ring" />Result</span>
      <span class="d-inline-flex align-center ga-1"><span class="swatch-ring selected" />Selected</span>
      <VSpacer />
      <span>Click to play · drag to lasso-select</span>
    </div>
  </div>
</template>

<style scoped>
.vibe-map {
  min-height: 0;
  height: 100%;
}

.vibe-map-canvas-wrap {
  position: relative;
  min-height: 0;
  border: 1px solid rgba(var(--v-theme-on-surface), 0.12);
  border-radius: 8px;
  background: rgb(var(--v-theme-surface));
  overflow: hidden;
}

.vibe-map-canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  cursor: crosshair;
  touch-action: none;
}

.vibe-map-canvas.is-hovering {
  cursor: pointer;
}

.vibe-map-tooltip {
  position: absolute;
  pointer-events: none;
  max-width: 260px;
  padding: 2px 8px;
  border-radius: 4px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  background: rgb(var(--v-theme-surface-variant));
  color: rgb(var(--v-theme-on-surface));
  border: 1px solid rgba(var(--v-theme-on-surface), 0.15);
}

.swatch {
  display: inline-block;
  width: 9px;
  height: 9px;
  border-radius: 50%;
}

.swatch-diamond {
  display: inline-block;
  width: 9px;
  height: 9px;
  transform: rotate(45deg);
  border: 2px solid rgb(var(--v-theme-on-surface));
}

.swatch-ring {
  display: inline-block;
  width: 11px;
  height: 11px;
  border-radius: 50%;
  border: 1.5px solid rgb(var(--v-theme-on-surface));
}

.swatch-ring.selected {
  border: 2.5px solid rgb(var(--v-theme-primary));
}
</style>
