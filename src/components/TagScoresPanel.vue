<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useToast } from '../composables/useToast';
import { useAutoTagEvents } from '../composables/useAutoTagEvents';
import { useAppStore } from '../stores/appStore';
import type { EntryTagScores, TagScore } from '@generated/index';
import { poolName } from '../utils/pools';

// Expansion panel for the entry editor: how strongly the analyzer thinks each
// vocabulary tag fits this entry. It starts open and loads on mount, and
// the panel is hidden for ambiance entries and for entries with no scores.
const props = defineProps<{
  entryId: string;
  // Ambiance entries are never scored.
  isAmbiance: boolean;
}>();

const toast = useToast();
const store = useAppStore();
const { completedRuns } = useAutoTagEvents();

const scores = ref<EntryTagScores | null>(null);
const loading = ref(false);
// True once a load has finished for the current entry.
const loaded = ref(false);
// v-model of the VExpansionPanels: the panel value while open, else undefined.
const open = ref<string | undefined>('scores');
// Bumped on every request so a slow answer for a previous entry is dropped.
let requestId = 0;

async function load(): Promise<void> {
  if (props.isAmbiance) return; // ambiance entries are never scored
  const id = ++requestId;
  loading.value = true;
  try {
    const result = await invoke<EntryTagScores | null>('get_entry_tag_scores', { entryId: props.entryId });
    if (id !== requestId) return;
    scores.value = result;
  } catch (e) {
    if (id !== requestId) return;
    scores.value = null;
    toast.showError(`Could not load tag scores: ${e}`);
  } finally {
    if (id === requestId) {
      loading.value = false;
      loaded.value = true;
    }
  }
}

// Load on expand, and again when a run finishes while it is open.
watch(open, (value) => {
  if (value !== undefined) void load();
});
// Open from the start, so the first load is not triggered by a change.
onMounted(() => {
  if (open.value !== undefined) void load();
});
watch(completedRuns, () => {
  if (open.value !== undefined) void load();
  else loaded.value = false; // a hidden panel may have scores now
});
watch(
  () => props.entryId,
  () => {
    requestId++;
    scores.value = null;
    loaded.value = false;
    loading.value = false;
    // A collapsed panel reopens, and its open watcher does the load.
    const wasOpen = open.value !== undefined;
    open.value = 'scores';
    if (wasOpen) void load();
  },
);

const entry = computed(() => store.findLibraryEntry(props.entryId));

// Tags you voted no on; ↺ clears the vote so the analyzer's opinion shows again.
const rejected = computed(() => entry.value?.rejectedTags ?? []);
const resetting = ref(false);

async function resetVote(tag: string): Promise<void> {
  if (resetting.value) return;
  resetting.value = true;
  try {
    await store.command('reset_entry_tag_votes', { entryIds: [props.entryId], tags: [tag] });
  } finally {
    resetting.value = false;
  }
}

// The panel has to exist before the first load so it can be expanded; once a
// load comes back empty it hides, unless there are "you said no" tags to list.
const visible = computed(
  () => !props.isAmbiance && !(loaded.value && scores.value === null && rejected.value.length === 0),
);

const trusted = computed<TagScore[]>(() => (scores.value?.tags ?? []).filter((t) => t.trusted));
const untrusted = computed<TagScore[]>(() => (scores.value?.tags ?? []).filter((t) => !t.trusted));
const topRows = computed<TagScore[]>(() => (scores.value ? [scores.value.gate, ...trusted.value] : []));

// The gate row is always the Combat probability, whatever tag name an old
// saved score carries. Its `applied` is set for either role, so the chip shows
// the entry's actual role.

function rowName(row: TagScore): string {
  return row === scores.value?.gate ? 'Combat' : row.tag;
}

function gateLabel(): string | null {
  const e = entry.value;
  if (!e?.role) return null;
  const name = e.role === 'combat' ? 'Combat' : 'Non-combat';
  return `${name} · ${e.roleUser !== null ? 'confirmed' : 'suggested'}`;
}

const soundRows = computed<{ label: string; value: string }[]>(() => {
  const e = entry.value;
  if (!e) return [];
  const rows: { label: string; value: string }[] = [];
  // The pool suggestion with its probability and the runner-up.
  if (e.poolAuto !== null) {
    const p = e.poolAutoP === null ? '' : ` · p = ${e.poolAutoP.toFixed(2)}`;
    const next = e.poolAutoRunner === null ? '' : ` · next: ${poolName(e.poolAutoRunner)}`;
    rows.push({ label: 'Pool suggestion', value: `${poolName(e.poolAuto)}${p}${next}` });
  }
  return rows;
});

function percent(score: number): number {
  return Math.max(0, Math.min(1, score)) * 100;
}

function appliedLabel(s: TagScore): string | null {
  if (s.applied === 'auto') return 'suggested';
  if (s.applied === 'user') return 'confirmed';
  return null;
}
</script>

<template>
  <VExpansionPanels v-if="visible" v-model="open" variant="accordion" class="mt-4">
    <VExpansionPanel value="scores" title="Tag scores">
      <VExpansionPanelText>
        <div v-if="loading && !scores" class="d-flex justify-center pa-3">
          <VProgressCircular indeterminate size="24" />
        </div>
        <div v-else-if="!scores" class="text-caption text-medium-emphasis">No scores yet.</div>
        <template v-else>
          <div v-for="row in topRows" :key="row.tag" class="score-row">
            <span class="score-name text-truncate" :title="rowName(row)">{{ rowName(row) }}</span>
            <div class="score-bar">
              <VProgressLinear :model-value="percent(row.score)" height="8" rounded color="primary" />
              <span
                v-if="row.threshold !== null"
                class="threshold-tick"
                :style="{ left: `${percent(row.threshold)}%` }"
                :title="`Threshold ${row.threshold.toFixed(2)}`"
              />
            </div>
            <span class="score-value text-caption">{{ row.score.toFixed(2) }}</span>
            <VChip
              v-if="row === scores.gate ? gateLabel() : appliedLabel(row)"
              size="x-small"
              label
              :class="row === scores.gate ? 'score-chip-wide' : 'score-chip'"
            >{{ row === scores.gate ? gateLabel() : appliedLabel(row) }}</VChip>
            <span v-else class="score-chip" />
          </div>

          <template v-if="untrusted.length > 0">
            <VDivider class="my-2" />
            <div class="text-caption text-medium-emphasis mb-1">Not applied yet</div>
            <div v-for="row in untrusted" :key="row.tag" class="score-row text-medium-emphasis">
              <span class="score-name text-truncate" :title="row.tag">{{ row.tag }}</span>
              <div class="score-bar">
                <VProgressLinear :model-value="percent(row.score)" height="8" rounded color="grey" />
                <span
                  v-if="row.threshold !== null"
                  class="threshold-tick"
                  :style="{ left: `${percent(row.threshold)}%` }"
                  :title="`Threshold ${row.threshold.toFixed(2)}`"
                />
              </div>
              <span class="score-value text-caption">{{ row.score.toFixed(2) }}</span>
              <VChip v-if="appliedLabel(row)" size="x-small" label class="score-chip">{{ appliedLabel(row) }}</VChip>
              <span v-else class="score-chip" />
            </div>
          </template>
        </template>
        <template v-if="rejected.length > 0">
          <VDivider class="my-2" />
          <div class="text-caption text-medium-emphasis mb-1">You said no</div>
          <div class="d-flex flex-wrap ga-1">
            <VChip
              v-for="tag in rejected"
              :key="tag"
              size="small"
              variant="outlined"
              class="text-medium-emphasis"
              append-icon="mdi-restore"
              :disabled="resetting"
              @click="resetVote(tag)"
            >
              {{ tag }}
              <VTooltip activator="parent" location="top" open-delay="300">
                Clear your no; the analyzer decides this tag again
              </VTooltip>
            </VChip>
          </div>
        </template>
        <template v-if="scores && soundRows.length > 0">
          <VDivider class="my-2" />
          <div class="text-caption text-medium-emphasis mb-1">Sound</div>
          <div v-for="row in soundRows" :key="row.label" class="score-row">
            <span class="score-name text-truncate">{{ row.label }}</span>
            <span class="text-caption">{{ row.value }}</span>
          </div>
        </template>
      </VExpansionPanelText>
    </VExpansionPanel>
  </VExpansionPanels>
</template>

<style scoped>
.score-row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 28px;
}
.score-name {
  flex: 0 0 140px;
  font-size: 0.875rem;
}
.score-bar {
  position: relative;
  flex: 1 1 auto;
}
.threshold-tick {
  position: absolute;
  top: -3px;
  bottom: -3px;
  width: 2px;
  margin-left: -1px;
  background: rgb(var(--v-theme-warning));
}
.score-value {
  flex: 0 0 36px;
  text-align: right;
}
.score-chip {
  flex: 0 0 76px;
  justify-content: center;
}
.score-chip-wide {
  flex: 0 0 auto;
  justify-content: center;
}
</style>
