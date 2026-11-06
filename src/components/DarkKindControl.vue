<script setup lang="ts">
import { computed, ref } from 'vue';
import type { DarkKind } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { DARK_KIND_LABELS, DARK_KINDS } from '../utils/mood';
import FacetStatus from './FacetStatus.vue';

/// Sad | Scary toggle for a Dark track's kind, plus its status column. The
/// shown kind is yours if set, else the analyzer's (the higher of its Sad and
/// Scary z-scores). Clicking a kind makes it yours; Confirm stores the
/// analyzer's kind as yours; Use auto clears yours. The editor shows this row
/// only while the shown Bright ↔ Dark step is Dark.
///
/// Like MoodScaleControl, the root uses `display: contents`, so inside a grid
/// the options and the status are two cells that line up with the other rows.
const props = defineProps<{
  entryIds: string[];
  auto: DarkKind | null;
  user: DarkKind | null;
  sadZ: number | null;
  scaryZ: number | null;
}>();

const store = useAppStore();
const busy = ref(false);

const shown = computed(() => props.user ?? props.auto);
const labelOf = (kind: DarkKind | null): string | null => (kind === null ? null : DARK_KIND_LABELS[kind]);
const signed = (z: number): string => `${z >= 0 ? '+' : ''}${z.toFixed(2)}`;
const autoHint = computed(() =>
  props.sadZ === null || props.scaryZ === null
    ? null
    : `Sad z = ${signed(props.sadZ)} · Scary z = ${signed(props.scaryZ)}`,
);

async function setKind(kind: DarkKind | null): Promise<void> {
  if (busy.value || props.entryIds.length === 0) return;
  busy.value = true;
  try {
    await store.command('set_entry_dark_kind', { entryIds: props.entryIds, kind });
  } finally {
    busy.value = false;
  }
}

function choose(kind: DarkKind): void {
  if (props.user !== kind) void setKind(kind);
}

function confirm(): void {
  if (props.auto !== null) void setKind(props.auto);
}
</script>

<template>
  <div class="facet-control">
    <VBtnToggle
      :model-value="shown"
      density="compact"
      variant="outlined"
      divided
      color="primary"
      :disabled="busy"
      @update:model-value="() => {}"
    >
      <VBtn
        v-for="kind in DARK_KINDS"
        :key="kind"
        :value="kind"
        size="small"
        @click.prevent="choose(kind)"
      >
        {{ DARK_KIND_LABELS[kind] }}
      </VBtn>
    </VBtnToggle>
    <FacetStatus
      :auto-label="labelOf(auto)"
      :user-label="labelOf(user)"
      :auto-hint="autoHint"
      :disabled="busy"
      @confirm="confirm"
      @use-auto="setKind(null)"
    />
  </div>
</template>

<style scoped>
.facet-control {
  display: contents;
}
</style>
