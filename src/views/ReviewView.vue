<script setup lang="ts">
// The Review tab: one music track at a time, with its Role and mood profile.
// Approve confirms the role shown, marks the track reviewed and moves on
// (mood bars the user did not touch stay the analyzer's suggestions); Skip
// moves on without saving anything (the track goes to the end of the queue
// for this session). Changing Role or a mood level on the card saves at once
// but does not move on by itself.
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { ScryHit } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useReviewQueue } from '../composables/useReviewQueue';
import { useLibraryPreview } from '../composables/useLibraryPreview';
import RoleControl from '../components/RoleControl.vue';
import MoodProfile from '../components/MoodProfile.vue';
import { topMoodText } from '../utils/moods';

const store = useAppStore();
const { queue, skippedWaiting, approvedCount, skip, noteApproved } = useReviewQueue();
const { playingId, toggle: togglePreview, stop: stopPreview } = useLibraryPreview();

// The card stays on its entry until Approve or Skip, even when a Role or mood
// change has already marked it reviewed (and so taken it out of the queue).
const currentId = ref<string | null>(null);
const current = computed(() => (currentId.value === null ? undefined : store.findLibraryEntry(currentId.value)));

watch(
  [queue, current],
  () => {
    // Pick the first card, and move on if the shown entry was deleted.
    if (current.value === undefined) currentId.value = queue.value[0]?.id ?? null;
  },
  { immediate: true },
);

const inQueue = computed(() => queue.value.some((e) => e.id === currentId.value));
const total = computed(() => approvedCount.value + queue.value.length + (current.value && !inQueue.value ? 1 : 0));
const position = computed(() => Math.min(approvedCount.value + skippedWaiting.value + 1, total.value));

/// "Artist / Album" from the file's two parent folders.
const folder = computed(() => {
  if (!current.value) return '';
  const parts = current.value.filePath.split(/[\\/]/).filter((p) => p.length > 0 && !/^[A-Za-z]:$/.test(p));
  return [parts[parts.length - 3], parts[parts.length - 2]].filter(Boolean).join(' / ');
});

/// A readout only: the two highest moods shown, e.g. "Mysterious 0.87 · Grim 0.74".
const readout = computed(() => (current.value ? topMoodText(current.value, 2) : ''));
const isPlaying = computed(() => current.value !== undefined && playingId.value === current.value.id);

// --- Sounds like: the two nearest reviewed tracks by sound ---

const SIMILAR_LIMIT = 200;
const SIMILAR_TIMEOUT_MS = 5000;
const neighbourIds = ref<string[]>([]);
let neighbourRequest = 0;

const neighbours = computed(() =>
  neighbourIds.value.map((id) => store.findLibraryEntry(id)).filter((e) => e !== undefined),
);

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('timed out')), ms);
    promise.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      },
    );
  });
}

watch(
  currentId,
  async (id) => {
    const request = ++neighbourRequest;
    neighbourIds.value = [];
    if (id === null) return;
    try {
      const hits = await withTimeout(
        invoke<ScryHit[]>('similar_entries', { entryIds: [id], limit: SIMILAR_LIMIT }),
        SIMILAR_TIMEOUT_MS,
      );
      if (request !== neighbourRequest) return;
      neighbourIds.value = hits
        .map((h) => h.entryId)
        .filter((hitId) => hitId !== id && (store.findLibraryEntry(hitId)?.moodReviewedAt ?? null) !== null)
        .slice(0, 2);
    } catch {
      // Not analyzed, analyzer off or slow: the line is left out.
    }
  },
  { immediate: true },
);

// --- Actions ---

const busy = ref(false);

function advance(from: string): void {
  if (playingId.value === from) stopPreview();
  currentId.value = queue.value.find((e) => e.id !== from)?.id ?? null;
}

async function approve(): Promise<void> {
  const entry = current.value;
  if (!entry || busy.value) return;
  busy.value = true;
  try {
    if (await store.command('approve_entry_review', { entryId: entry.id })) {
      noteApproved();
      advance(entry.id);
    }
  } finally {
    busy.value = false;
  }
}

function skipCurrent(): void {
  const entry = current.value;
  if (!entry || busy.value) return;
  skip(entry.id);
  advance(entry.id);
}

function play(): void {
  if (current.value) togglePreview(current.value.id);
}

function edit(): void {
  if (current.value) store.editEntryId = current.value.id;
}

// Keys: A approve, S skip, Space play/stop. Ignored while typing or while the
// editor dialog is open.
function onKey(e: KeyboardEvent): void {
  if (e.ctrlKey || e.metaKey || e.altKey || store.editEntryId !== null) return;
  const target = e.target as HTMLElement | null;
  if (target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))) return;
  const key = e.key.toLowerCase();
  if (key === 'a') {
    e.preventDefault();
    void approve();
  } else if (key === 's') {
    e.preventDefault();
    skipCurrent();
  } else if (key === ' ') {
    // Stop a focused button from also being pressed by the space bar.
    e.preventDefault();
    (document.activeElement as HTMLElement | null)?.blur();
    play();
  }
}

onMounted(() => window.addEventListener('keydown', onKey));
onBeforeUnmount(() => window.removeEventListener('keydown', onKey));
</script>

<template>
  <div class="review-view pa-6 d-flex flex-column align-center">
    <div
      v-if="!current"
      class="d-flex flex-column align-center justify-center text-medium-emphasis pa-8 flex-grow-1"
    >
      <VIcon size="64" class="mb-4">mdi-cards-outline</VIcon>
      <span class="text-title-large">Nothing to review</span>
      <span class="text-body-2 mt-1">New imports show up here.</span>
    </div>

    <VCard v-else class="review-card pa-8" variant="flat" border>
      <div class="d-flex align-start ga-3">
        <VBtn
          :icon="isPlaying ? 'mdi-stop' : 'mdi-play'"
          color="primary"
          variant="tonal"
          :title="isPlaying ? 'Stop (Space)' : 'Play (Space)'"
          :disabled="store.isEntryMissing(current.id)"
          @click="play"
        />
        <div class="flex-grow-1" style="min-width: 0;">
          <div class="text-title-large text-truncate" :title="current.name">{{ current.name }}</div>
          <div class="text-caption text-medium-emphasis text-truncate" :title="folder">{{ folder }}</div>
        </div>
        <VBtn icon="mdi-pencil" variant="text" size="small" title="Open in the editor" @click="edit" />
        <span class="text-caption text-medium-emphasis progress">{{ position }} of {{ total }}</span>
      </div>

      <div class="readout text-body-1 text-medium-emphasis my-4">
        {{ readout || 'Not analyzed yet' }}
      </div>

      <div class="facet-grid mb-4">
        <span class="text-caption text-medium-emphasis">Role</span>
        <RoleControl
          :entry-ids="[current.id]"
          :role-auto="current.roleAuto"
          :role-auto-score="current.roleAutoScore"
          :role-user="current.roleUser"
        />
      </div>

      <MoodProfile :entries="[current]" class="mb-4" />


      <div v-if="neighbours.length > 0" class="d-flex flex-wrap align-center ga-2 mb-3">
        <span class="text-caption text-medium-emphasis">Sounds like</span>
        <VChip
          v-for="n in neighbours"
          :key="n.id"
          size="small"
          variant="tonal"
          :prepend-icon="playingId === n.id ? 'mdi-stop' : 'mdi-play'"
          @click="togglePreview(n.id)"
        >
          {{ n.name }}
        </VChip>
      </div>

      <div class="action-row d-flex align-center ga-2 mt-6">
        <VBtn color="primary" prepend-icon="mdi-check" :loading="busy" @click="approve">
          Approve
          <span class="key-hint ml-2">A</span>
        </VBtn>
        <VBtn variant="tonal" prepend-icon="mdi-skip-next" :disabled="busy" @click="skipCurrent">
          Skip
          <span class="key-hint ml-2">S</span>
        </VBtn>
        <VSpacer />
        <span class="text-caption text-medium-emphasis">Space plays or stops</span>
      </div>
    </VCard>
  </div>
</template>

<style scoped>
.review-view {
  height: 100%;
  overflow-y: auto;
}
/* Up to 1100px wide, filling the view below that. min-width lets the card
   grow past that rather than clip a row that cannot fit (the view scrolls). */
.review-card {
  width: 100%;
  max-width: 1100px;
  min-width: min-content;
  overflow: visible;
}
.progress {
  white-space: nowrap;
  padding-top: 10px;
}
/* Label | options | status. The status column keeps its text on one line,
   right-aligned, and is never squeezed. */
.facet-grid {
  display: grid;
  grid-template-columns: max-content minmax(min-content, 1fr) max-content;
  align-items: center;
  column-gap: 16px;
  row-gap: 16px;
}
.facet-grid :deep(.facet-status) {
  justify-self: end;
}
.action-row {
  width: 100%;
}
.key-hint {
  font-size: 0.7rem;
  opacity: 0.7;
  border: 1px solid currentColor;
  border-radius: 3px;
  padding: 0 4px;
  line-height: 1.3;
}
</style>
