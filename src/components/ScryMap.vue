<script lang="ts">
import { ref as moduleRef } from 'vue';

// The Seed view / Map choice, kept for the session (module scope outlives the
// component when the user leaves Scry and comes back). Seed view by default.
const preferSeedView = moduleRef(true);
</script>

<script setup lang="ts">
// Canvas scatter of every analyzed music track. Points are colored by Role;
// hovering shows the name, a click on a point plays the track and pins a small
// card with a Seed button (shift-click seeds straight away), a shift-click on
// empty space asks for the tracks nearest that spot (shift-right-click clears
// the spot and any seeds), a right-click on empty space clears the selection, a left-drag lasso adds the
// enclosed points to the selection, and a right-click or right-drag lasso
// removes them from it.
//
// With seeds set, the map can switch to a seed view (src/utils/seedView.ts):
// the seeds sit at the centre and each track's distance from them is its exact
// similarity rank, the same order as the results list, while its direction
// still comes from the map. Every interaction works there too, except the
// spot search: a spot's neighbours on the radial layout mean nothing new (they
// are just similar to the seeds), so shift-clicking empty space there only
// shows a note pointing to the Map view.
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useTheme } from 'vuetify';
import type { LibraryEntry, ScryPoint } from '@generated/index';
import { seedViewLayout } from '../utils/seedView';

const props = defineProps<{
  points: ScryPoint[];
  entriesById: Map<string, LibraryEntry>;
  seedIds: string[];
  resultIds: string[];
  selectedIds: string[];
  playingId: string | null;
  // Point to emphasize, e.g. the results row under the mouse.
  highlightId?: string | null;
  // A clicked empty spot, in the same [0, 1] coordinates as the points.
  spot?: { x: number; y: number } | null;
  // Every scored track's similarity to the seeds, and the seeds it is for;
  // when set, the seed view is available.
  seedScores?: { seedIds: string[]; scores: Map<string, number> } | null;
}>();

const emit = defineEmits<{
  play: [entryId: string];
  lasso: [entryIds: string[]];
  deselect: [entryIds: string[]];
  // Empty space was clicked; coordinates are in [0, 1] like the points.
  spot: [pos: { x: number; y: number }];
  // Toggle a track in or out of the seeds.
  seed: [entryId: string];
  // Empty space was shift-right-clicked: drop the spot and the seeds.
  clearSearch: [];
  // Empty space was right-clicked: clear the selection.
  clearSelection: [];
}>();

// Inset so points on the [0, 1] edges are not clipped.
const PAD = 14;
// Nearest point within this many CSS pixels counts as a hit.
const HIT_RADIUS = 6;
// Movement beyond this many CSS pixels turns a press into a lasso drag.
const DRAG_THRESHOLD = 4;
const BASE_RADIUS = 3;
const SEED_RADIUS = 9;
const HIGHLIGHT_RADIUS = 12;
const SPOT_RADIUS = 7;
// Seed view <-> map transition length.
const TWEEN_MS = 350;

const theme = useTheme();
const wrapper = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const size = ref({ width: 0, height: 0 });

const hover = ref<{ id: string; x: number; y: number } | null>(null);
// The point last clicked: it gets a small card with its actions.
const pinnedId = ref<string | null>(null);
const lasso = ref<{ x: number; y: number }[]>([]);
// The mouse button that started the current press: 0 adds, 2 removes.
let pressButton = -1;
let pressStart: { x: number; y: number } | null = null;
let lassoing = false;

const seedSet = computed(() => new Set(props.seedIds));
const resultSet = computed(() => new Set(props.resultIds));
const selectedSet = computed(() => new Set(props.selectedIds));

interface ScreenPoint { id: string; x: number; y: number }

// --- Seed view ---

const layout = computed(() =>
  props.seedScores ? seedViewLayout(props.points, props.seedScores.seedIds, props.seedScores.scores) : null,
);
const seedViewOn = computed(() => layout.value !== null && preferSeedView.value);

const view = computed({
  get: () => (seedViewOn.value ? 'seed' : 'map'),
  set: (v: 'seed' | 'map') => {
    preferSeedView.value = v === 'seed';
  },
});

// Shown briefly over the map, e.g. when a spot search is not available.
const notice = ref('');
let noticeTimer = 0;
function showNotice(text: string): void {
  notice.value = text;
  clearTimeout(noticeTimer);
  noticeTimer = window.setTimeout(() => (notice.value = ''), 3000);
}

// Drawable area in CSS pixels, inside the padding.
const plotSize = computed(() => ({
  w: Math.max(0, size.value.width - 2 * PAD),
  h: Math.max(0, size.value.height - 2 * PAD),
}));

function toScreen(x: number, y: number): { x: number; y: number } {
  return { x: PAD + x * plotSize.value.w, y: PAD + (1 - y) * plotSize.value.h };
}

// Inverse of toScreen, clamped to the map.
function toData(sx: number, sy: number): { x: number; y: number } {
  const { w, h } = plotSize.value;
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return { x: clamp(w > 0 ? (sx - PAD) / w : 0), y: clamp(h > 0 ? 1 - (sy - PAD) / h : 0) };
}

// The seed view's unit disc on screen: centred, scaled the same on both axes
// so its rings stay circles.
const disc = computed(() => {
  const { w, h } = plotSize.value;
  return { cx: PAD + w / 2, cy: PAD + h / 2, scale: Math.min(w, h) / 2 };
});

// Where each point belongs in the current view, in CSS pixels.
const targetPoints = computed<ScreenPoint[]>(() => {
  const positions = seedViewOn.value ? layout.value?.positions : null;
  const { cx, cy, scale } = disc.value;
  return props.points.map((p) => {
    const q = positions?.get(p.entryId);
    return q ? { id: p.entryId, x: cx + q.x * scale, y: cy - q.y * scale } : { id: p.entryId, ...toScreen(p.x, p.y) };
  });
});

// A short tween when the view or the seed layout changes; resizes and the
// first render jump straight to place.
const tween = ref(1);
let tweenFrom: Map<string, { x: number; y: number }> | null = null;
let tweenFrame = 0;
let animateNext = false;

const ease = (t: number) => 1 - (1 - t) ** 3;

// Points in CSS pixels as drawn right now.
const screenPoints = computed<ScreenPoint[]>(() => {
  const target = targetPoints.value;
  const from = tweenFrom;
  if (tween.value >= 1 || !from) return target;
  const e = ease(tween.value);
  return target.map((p) => {
    const f = from.get(p.id);
    return f ? { id: p.id, x: f.x + (p.x - f.x) * e, y: f.y + (p.y - f.y) * e } : p;
  });
});

// Animate switching views, and new seed scores while in the seed view.
watch(
  [seedViewOn, layout],
  ([on], [wasOn]) => {
    animateNext = on || wasOn;
  },
  { flush: 'sync' },
);
watch(targetPoints, (_now, before) => {
  const animate = animateNext;
  animateNext = false;
  if (!animate || before.length === 0 || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) {
    cancelAnimationFrame(tweenFrame);
    tweenFrom = null;
    tween.value = 1;
    return;
  }
  // Start from where the points are drawn now (mid-tween included).
  const shown = new Map<string, { x: number; y: number }>();
  const e = ease(tween.value);
  for (const p of before) {
    const f = tween.value < 1 ? tweenFrom?.get(p.id) : undefined;
    shown.set(p.id, f ? { x: f.x + (p.x - f.x) * e, y: f.y + (p.y - f.y) * e } : { x: p.x, y: p.y });
  }
  tweenFrom = shown;
  tween.value = 0;
  const start = performance.now();
  cancelAnimationFrame(tweenFrame);
  const step = (now: number) => {
    tween.value = Math.min(1, (now - start) / TWEEN_MS);
    if (tween.value < 1) tweenFrame = requestAnimationFrame(step);
    else tweenFrom = null;
  };
  tweenFrame = requestAnimationFrame(step);
});

const pointById = computed(() => {
  const map = new Map<string, ScreenPoint>();
  for (const pt of screenPoints.value) map.set(pt.id, pt);
  return map;
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
    nonCombat: toRgb(c.secondary, [255, 109, 0]),
    unset: ink,
    ink,
    // Theme background, used as an outline behind marks so they stand apart from neighbouring points.
    shade: toRgb(c.background, dark ? [10, 10, 12] : [255, 255, 255]),
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
]);

// --- Drawing ---
// Points, rings, and seeds change rarely and are cached in an offscreen
// canvas; hover, highlight, playing, and lasso marks are drawn over a copy of
// it, so moving the mouse never repaints every point.

let frame = 0;
let baseDirty = true;
const baseCanvas = document.createElement('canvas');

function scheduleDraw(): void {
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    draw();
  });
}

function invalidateBase(): void {
  baseDirty = true;
  scheduleDraw();
}

function diamond(ctx: CanvasRenderingContext2D, x: number, y: number, r: number): void {
  ctx.beginPath();
  ctx.moveTo(x, y - r);
  ctx.lineTo(x + r, y);
  ctx.lineTo(x, y + r);
  ctx.lineTo(x - r, y);
  ctx.closePath();
}

// Seed: a large light diamond with a dark outline and a role-colored center,
// unlike any role color or ring.
function drawSeed(ctx: CanvasRenderingContext2D, pt: ScreenPoint): void {
  const p = palette.value;
  diamond(ctx, pt.x, pt.y, SEED_RADIUS);
  ctx.lineJoin = 'round';
  ctx.lineWidth = 4;
  ctx.strokeStyle = rgba(p.shade, 0.95);
  ctx.stroke();
  ctx.fillStyle = rgba(p.ink);
  ctx.fill();
  ctx.beginPath();
  ctx.arc(pt.x, pt.y, 2.5, 0, Math.PI * 2);
  ctx.fillStyle = rgba(roleColor(props.entriesById.get(pt.id)));
  ctx.fill();
}

// Seed view guides: faint rings around the seeds, each labelled with the
// similarity a track on it has, plus a dashed rim for unscored tracks.
function drawRings(ctx: CanvasRenderingContext2D): void {
  const l = layout.value;
  if (!seedViewOn.value || !l) return;
  const p = palette.value;
  const { cx, cy, scale } = disc.value;
  ctx.font = '10px sans-serif';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'bottom';
  const rings: { radius: number; label: string; dashed: boolean }[] = l.rings.map((r) => ({
    radius: r.radius,
    label: r.score.toFixed(2),
    dashed: false,
  }));
  if (l.unrankedRadius !== null) rings.push({ radius: l.unrankedRadius, label: 'unranked', dashed: true });
  for (const ring of rings) {
    const r = ring.radius * scale;
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.setLineDash(ring.dashed ? [3, 4] : []);
    ctx.lineWidth = 1;
    ctx.strokeStyle = rgba(p.ink, 0.13);
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.lineWidth = 3;
    ctx.strokeStyle = rgba(p.shade, 0.9);
    ctx.strokeText(ring.label, cx, cy - r - 1);
    ctx.fillStyle = rgba(p.ink, 0.5);
    ctx.fillText(ring.label, cx, cy - r - 1);
  }
}

function drawBase(ctx: CanvasRenderingContext2D): void {
  drawRings(ctx);
  const p = palette.value;
  const focused = seedSet.value.size > 0 || resultSet.value.size > 0;
  const pts = screenPoints.value;

  // Every point, dimmed while results or seeds are shown.
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

  // Seeds on top of everything else in the base layer.
  for (const pt of pts) {
    if (seedSet.value.has(pt.id)) drawSeed(ctx, pt);
  }
}

// Highlighted point: soft light halo, the point enlarged in its role color,
// and a light ring with a dark outline so it reads against orange and red.
function drawHighlight(ctx: CanvasRenderingContext2D, pt: ScreenPoint): void {
  const p = palette.value;
  ctx.beginPath();
  ctx.arc(pt.x, pt.y, HIGHLIGHT_RADIUS + 6, 0, Math.PI * 2);
  ctx.fillStyle = rgba(p.ink, 0.16);
  ctx.fill();

  if (seedSet.value.has(pt.id)) {
    drawSeed(ctx, pt);
  } else {
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, BASE_RADIUS + 2.5, 0, Math.PI * 2);
    ctx.fillStyle = rgba(roleColor(props.entriesById.get(pt.id)));
    ctx.fill();
  }

  ctx.beginPath();
  ctx.arc(pt.x, pt.y, HIGHLIGHT_RADIUS, 0, Math.PI * 2);
  ctx.lineWidth = 5;
  ctx.strokeStyle = rgba(p.shade, 0.9);
  ctx.stroke();
  ctx.lineWidth = 2.5;
  ctx.strokeStyle = rgba(p.ink);
  ctx.stroke();
}

function draw(): void {
  const el = canvas.value;
  if (!el) return;
  const ctx = el.getContext('2d');
  if (!ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const { width, height } = size.value;
  const pxWidth = Math.round(width * dpr);
  const pxHeight = Math.round(height * dpr);
  if (el.width !== pxWidth || el.height !== pxHeight) {
    el.width = pxWidth;
    el.height = pxHeight;
  }
  if (baseCanvas.width !== pxWidth || baseCanvas.height !== pxHeight) {
    baseCanvas.width = pxWidth;
    baseCanvas.height = pxHeight;
    baseDirty = true;
  }
  if (pxWidth === 0 || pxHeight === 0) return;

  if (baseDirty) {
    const bctx = baseCanvas.getContext('2d');
    if (!bctx) return;
    bctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    bctx.clearRect(0, 0, width, height);
    drawBase(bctx);
    baseDirty = false;
  }

  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, pxWidth, pxHeight);
  ctx.drawImage(baseCanvas, 0, 0);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  const p = palette.value;

  // Playing and hovered points.
  for (const id of [props.playingId, hover.value?.id]) {
    if (!id) continue;
    const pt = pointById.value.get(id);
    if (!pt) continue;
    ctx.lineWidth = 2;
    ctx.strokeStyle = rgba(p.ink);
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, BASE_RADIUS + 6, 0, Math.PI * 2);
    ctx.stroke();
  }

  // Clicked spot: an accent crosshair ring (map coordinates, so map view only).
  if (props.spot && !seedViewOn.value) {
    const s = toScreen(props.spot.x, props.spot.y);
    ctx.lineWidth = 4;
    ctx.strokeStyle = rgba(p.shade, 0.9);
    ctx.beginPath();
    ctx.arc(s.x, s.y, SPOT_RADIUS, 0, Math.PI * 2);
    ctx.stroke();
    ctx.lineWidth = 2;
    ctx.strokeStyle = rgba(p.accent);
    ctx.stroke();
    ctx.beginPath();
    for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
      ctx.moveTo(s.x + dx * (SPOT_RADIUS + 2), s.y + dy * (SPOT_RADIUS + 2));
      ctx.lineTo(s.x + dx * (SPOT_RADIUS + 7), s.y + dy * (SPOT_RADIUS + 7));
    }
    ctx.stroke();
  }

  if (props.highlightId) {
    const pt = pointById.value.get(props.highlightId);
    if (pt) drawHighlight(ctx, pt);
  }

  // Lasso polygon while dragging: accent to add, light to remove.
  if (lasso.value.length > 1) {
    const color = pressButton === 2 ? p.ink : p.accent;
    ctx.beginPath();
    ctx.moveTo(lasso.value[0].x, lasso.value[0].y);
    for (const q of lasso.value.slice(1)) ctx.lineTo(q.x, q.y);
    ctx.closePath();
    ctx.fillStyle = rgba(color, pressButton === 2 ? 0.08 : 0.12);
    ctx.fill();
    ctx.setLineDash([5, 4]);
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = rgba(color);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}

watch([screenPoints, () => props.entriesById, seedSet, resultSet, selectedSet, palette, seedViewOn], invalidateBase);
watch([() => props.playingId, () => props.highlightId, () => props.spot, hover, lasso], scheduleDraw);

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
  if (pressStart || (e.button !== 0 && e.button !== 2)) return;
  canvas.value?.setPointerCapture(e.pointerId);
  pressButton = e.button;
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
      pinnedId.value = null;
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
  const removing = pressButton === 2;
  // Chorded buttons (hold left, add right, release left) fire no pointerdown or
  // pointerup of their own, so a release of a different button cancels the press.
  const cancelled = e.button !== pressButton;
  const wasLassoing = lassoing;
  const poly = lasso.value;
  pressStart = null;
  lassoing = false;
  lasso.value = [];
  if (canvas.value?.hasPointerCapture(e.pointerId)) canvas.value.releasePointerCapture(e.pointerId);
  if (cancelled) return;
  if (wasLassoing) {
    if (poly.length >= 3) {
      const ids = screenPoints.value.filter((pt) => insidePolygon(pt.x, pt.y, poly)).map((pt) => pt.id);
      if (ids.length === 0) return;
      if (removing) emit('deselect', ids);
      else emit('lasso', ids);
    }
    return;
  }
  const hit = nearest(start.x, start.y);
  if (!hit) {
    pinnedId.value = null;
    if (!e.shiftKey) {
      if (removing) emit('clearSelection');
      return;
    }
    if (removing) emit('clearSearch');
    else if (seedViewOn.value) showNotice('Spot search works on the Map view');
    else emit('spot', toData(start.x, start.y));
    return;
  }
  if (removing) emit('deselect', [hit.id]);
  else if (e.shiftKey) emit('seed', hit.id);
  else {
    emit('play', hit.id);
    pinnedId.value = hit.id;
  }
}

// Also runs on lost capture (e.g. alt-tab mid-drag), where no pointerup arrives.
function onPointerCancel(): void {
  pressStart = null;
  lassoing = false;
  lasso.value = [];
}

function onPointerLeave(): void {
  if (!pressStart) hover.value = null;
}

const hoverName = computed(() => {
  // The pinned card already names its point.
  if (!hover.value || hover.value.id === pinnedId.value) return '';
  return props.entriesById.get(hover.value.id)?.name ?? '';
});

// --- Pinned card ---

const pinned = computed(() => (pinnedId.value ? pointById.value.get(pinnedId.value) ?? null : null));
const pinnedName = computed(() => (pinnedId.value ? props.entriesById.get(pinnedId.value)?.name ?? '' : ''));
const pinnedIsSeed = computed(() => !!pinnedId.value && seedSet.value.has(pinnedId.value));

// A point that leaves the map (deleted entry) drops its card.
watch(pointById, (map) => {
  if (pinnedId.value && !map.has(pinnedId.value)) pinnedId.value = null;
});

// Beside the point, flipped below it near the top edge and kept inside the canvas.
const cardStyle = computed(() => {
  const pt = pinned.value;
  if (!pt) return {};
  const left = Math.min(Math.max(pt.x, 120), Math.max(120, size.value.width - 120));
  const above = pt.y > 56;
  return {
    left: `${left}px`,
    top: `${above ? pt.y - 14 : pt.y + 14}px`,
    transform: above ? 'translate(-50%, -100%)' : 'translate(-50%, 0)',
  };
});

function onKeydown(e: KeyboardEvent): void {
  if (e.key === 'Escape') pinnedId.value = null;
}

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

const ariaLabel = computed(() => {
  const parts = [
    seedViewOn.value
      ? `Seed view of ${props.points.length} analyzed tracks: distance from the centre is similarity to the seeds`
      : `Map of ${props.points.length} analyzed tracks by sound`,
  ];
  if (props.seedIds.length > 0) parts.push(`${props.seedIds.length} seeds`);
  if (props.resultIds.length > 0) parts.push(`${props.resultIds.length} results`);
  if (props.selectedIds.length > 0) parts.push(`${props.selectedIds.length} selected`);
  return parts.join(', ');
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
  invalidateBase();
}

onMounted(() => {
  window.addEventListener('keydown', onKeydown);
  observer = new ResizeObserver(measure);
  if (wrapper.value) observer.observe(wrapper.value);
  measure();
  watchPixelRatio();
});

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeydown);
  observer?.disconnect();
  clearTimeout(noticeTimer);
  cancelAnimationFrame(tweenFrame);
  dprQuery?.removeEventListener('change', watchPixelRatio);
  if (frame) cancelAnimationFrame(frame);
});
</script>

<template>
  <div class="scry-map d-flex flex-column">
    <div ref="wrapper" class="scry-map-canvas-wrap flex-grow-1">
      <canvas
        ref="canvas"
        class="scry-map-canvas"
        :class="{ 'is-hovering': !!hover }"
        role="img"
        :aria-label="ariaLabel"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="onPointerCancel"
        @lostpointercapture="onPointerCancel"
        @pointerleave="onPointerLeave"
      />
      <div v-if="hover && hoverName" class="scry-map-tooltip text-caption" :style="tooltipStyle">
        {{ hoverName }}
      </div>
      <VBtnToggle
        v-if="layout"
        v-model="view"
        class="scry-map-toggle"
        density="compact"
        mandatory
        divided
        variant="outlined"
        rounded="lg"
      >
        <VBtn value="seed" size="x-small" prepend-icon="mdi-target" title="Distance from the centre is similarity to the seeds">
          Seed view
        </VBtn>
        <VBtn value="map" size="x-small" prepend-icon="mdi-scatter-plot-outline" title="Every track by sound (approximate neighbours)">
          Map
        </VBtn>
      </VBtnToggle>
      <div v-if="notice" class="scry-map-notice text-caption">{{ notice }}</div>
      <div v-if="pinned" class="scry-map-card d-flex align-center ga-1" :style="cardStyle">
        <span class="text-caption text-truncate px-1" :title="pinnedName">{{ pinnedName }}</span>
        <VBtn
          :icon="pinnedIsSeed ? 'mdi-seed-off-outline' : 'mdi-seed-plus-outline'"
          size="x-small"
          variant="text"
          :title="pinnedIsSeed ? 'Remove seed' : 'Add as seed'"
          :aria-label="pinnedIsSeed ? 'Remove seed' : 'Add as seed'"
          @click="emit('seed', pinned.id)"
        />
        <VBtn icon="mdi-close" size="x-small" variant="text" aria-label="Close" @click="pinnedId = null" />
      </div>
    </div>
    <div class="d-flex flex-wrap align-center ga-3 px-2 pt-2 text-caption text-medium-emphasis">
      <span v-for="item in legend" :key="item.label" class="d-inline-flex align-center ga-1">
        <span class="swatch" :style="{ background: item.color }" />{{ item.label }}
      </span>
      <span class="d-inline-flex align-center ga-1"><span class="swatch-diamond" />Seed</span>
      <span class="d-inline-flex align-center ga-1"><span class="swatch-ring" />Result</span>
      <span class="d-inline-flex align-center ga-1"><span class="swatch-ring selected" />Selected</span>
      <span v-if="seedViewOn" class="d-inline-flex align-center ga-1">
        <span class="swatch-ring guide" />Similarity to the seeds (closer is more similar)
      </span>
      <VSpacer />
      <span v-if="seedViewOn">Click a dot to play, shift-click to seed · drag to select, right-drag to deselect, right-click empty space to clear · shift-right-click to clear seeds · switch to Map for spot search</span>
      <span v-else>Click a dot to play, shift-click to seed · drag to select, right-drag to deselect, right-click empty space to clear · shift-click empty space for nearby tracks, shift-right-click to clear spot and seeds</span>
    </div>
  </div>
</template>

<style scoped>
.scry-map {
  min-height: 0;
  height: 100%;
}

.scry-map-canvas-wrap {
  position: relative;
  min-height: 0;
  border: 1px solid rgba(var(--v-theme-on-surface), 0.12);
  border-radius: 8px;
  background: rgb(var(--v-theme-surface));
  overflow: hidden;
}

.scry-map-canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  cursor: crosshair;
  touch-action: none;
}

.scry-map-canvas.is-hovering {
  cursor: pointer;
}

.scry-map-tooltip {
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

.scry-map-card {
  position: absolute;
  max-width: 300px;
  padding: 2px 2px 2px 6px;
  border-radius: 6px;
  white-space: nowrap;
  background: rgb(var(--v-theme-surface-variant));
  color: rgb(var(--v-theme-on-surface));
  border: 1px solid rgba(var(--v-theme-on-surface), 0.15);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
  z-index: 1;
}

.swatch {
  display: inline-block;
  width: 9px;
  height: 9px;
  border-radius: 50%;
}

.swatch-diamond {
  display: inline-block;
  width: 10px;
  height: 10px;
  margin: 0 2px;
  transform: rotate(45deg);
  background: rgb(var(--v-theme-on-surface));
  box-shadow: 0 0 0 1.5px rgb(var(--v-theme-background));
}

.swatch-ring {
  display: inline-block;
  width: 11px;
  height: 11px;
  border-radius: 50%;
  border: 1.5px solid rgb(var(--v-theme-on-surface));
}

.swatch-ring.guide {
  border: 1px solid rgba(var(--v-theme-on-surface), 0.35);
}

.scry-map-toggle {
  position: absolute;
  top: 8px;
  right: 8px;
  z-index: 1;
  background: rgb(var(--v-theme-surface));
}

.scry-map-notice {
  position: absolute;
  top: 8px;
  left: 50%;
  transform: translateX(-50%);
  padding: 2px 10px;
  border-radius: 4px;
  pointer-events: none;
  background: rgb(var(--v-theme-surface-variant));
  color: rgb(var(--v-theme-on-surface));
  border: 1px solid rgba(var(--v-theme-on-surface), 0.15);
  z-index: 1;
}

.swatch-ring.selected {
  border: 2.5px solid rgb(var(--v-theme-primary));
}
</style>
