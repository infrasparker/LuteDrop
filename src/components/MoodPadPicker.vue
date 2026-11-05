<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import type { LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { entryAutoCell, entryCell, MOOD_SCALES, scaleValues, type MoodCell } from '../utils/mood';
import MoodPad, { type PadSize } from './MoodPad.vue';

/// The mood pad in pick mode, in a menu. Used by the track editor's Mood row
/// (one entry) and the Library's bulk "Set mood…" (several entries).
///
/// The pad opens on the size the track sits on, with the track's cell pinned
/// and the analyzer's cell sparkled when it differs. With several entries
/// whose moods differ, it opens on Moderate with no pin. Clicking a cell sets
/// all three scales to that cell as yours, skipping entries whose step on a
/// scale is already yours, then closes.
///
/// The `activator` slot replaces the default button (the track's mood name).
const props = defineProps<{ entries: LibraryEntry[] }>();

const store = useAppStore();
const open = ref(false);
const busy = ref(false);
const size = ref<PadSize>('mid');

/// The one cell every entry shares, or null when they differ or any is not placed.
function shared(cells: (MoodCell | null)[]): MoodCell | null {
  const first = cells[0] ?? null;
  return first !== null && cells.every((c) => c?.key === first.key) ? first : null;
}

const current = computed(() => shared(props.entries.map(entryCell)));
const auto = computed(() => shared(props.entries.map(entryAutoCell)));
const isBulk = computed(() => props.entries.length > 1);

watch(open, (isOpen) => {
  if (isOpen) size.value = current.value?.grand ?? 'mid';
});

async function pick(cell: MoodCell): Promise<void> {
  if (busy.value || props.entries.length === 0) return;
  busy.value = true;
  try {
    // Every scale becomes yours, even one that already matched the analyzer:
    // picking a cell is a decision about the whole mood.
    for (const scale of MOOD_SCALES) {
      const entryIds = props.entries.filter((e) => scaleValues(e, scale).user !== cell[scale]).map((e) => e.id);
      if (entryIds.length > 0) {
        // A failed command has already shown its error; stay open so the user sees where it stopped.
        if (!(await store.command('set_entry_mood_step', { entryIds, scale, step: cell[scale] }))) return;
      }
    }
    open.value = false;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <VMenu v-model="open" :close-on-content-click="false" location="bottom start">
    <template #activator="{ props: activator }">
      <slot name="activator" :props="activator" :current="current" :busy="busy">
        <VBtn
          v-bind="activator"
          size="small"
          variant="outlined"
          prepend-icon="mdi-grid"
          append-icon="mdi-menu-down"
          :disabled="busy || entries.length === 0"
        >
          {{ current ? current.name : 'Not placed' }}
        </VBtn>
      </slot>
    </template>
    <VCard class="pa-3" max-width="560">
      <div class="text-subtitle-2">
        <template v-if="isBulk">
          Mood · {{ entries.length }} tracks<template v-if="current">: <strong>{{ current.name }}</strong></template>
          <template v-else>, mixed or not placed</template>
        </template>
        <template v-else>
          Mood<template v-if="current"> · this track: <strong>{{ current.name }}</strong></template>
        </template>
      </div>
      <div class="text-caption text-medium-emphasis mb-2">
        Click a mood to set all three scales as yours.
        <template v-if="auto && auto.key !== current?.key">
          The analyzer would say {{ auto.name }} (✨).
        </template>
      </div>
      <MoodPad
        v-model:size="size"
        mode="pick"
        :current="current?.key ?? null"
        :auto="auto?.key ?? null"
        @pick="pick"
      />
    </VCard>
  </VMenu>
</template>
