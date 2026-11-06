<script setup lang="ts">
import { computed, ref } from 'vue';
import type { LibraryEntry, Pool } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { effectivePool, poolName, POOLS, suggestionText } from '../utils/pools';
import FacetStatus from './FacetStatus.vue';

/// The eight pool buttons for one music entry, plus its status column. The
/// shown pool is the confirmed one if set, else the analyzer's suggestion
/// (marked with a sparkle). Clicking a pool confirms it; Confirm keeps the
/// suggestion; Use suggestion clears the confirmed pool. Every one of these
/// also marks the entry as reviewed (the backend sets `pool_reviewed_at`).
///
/// Like the other facet controls, the root uses `display: contents`, so
/// inside a grid the buttons and the status are two cells.
const props = defineProps<{ entry: LibraryEntry }>();

const store = useAppStore();
const busy = ref(false);

const shown = computed(() => effectivePool(props.entry));
const autoHint = computed(() => {
  const text = suggestionText(props.entry);
  return text === null ? null : `Suggested: ${text}`;
});

async function setPool(pool: Pool | null): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  try {
    await store.command('set_entry_pool', { entryIds: [props.entry.id], pool });
  } finally {
    busy.value = false;
  }
}

function choose(pool: Pool): void {
  if (props.entry.poolUser !== pool) void setPool(pool);
}

function confirm(): void {
  if (props.entry.poolAuto !== null) void setPool(props.entry.poolAuto);
}
</script>

<template>
  <div class="facet-control">
    <div class="pool-buttons d-flex flex-wrap ga-1">
      <VBtn
        v-for="p in POOLS"
        :key="p.id"
        size="small"
        :variant="shown === p.id ? 'flat' : 'outlined'"
        :color="shown === p.id ? 'primary' : undefined"
        :disabled="busy"
        :append-icon="entry.poolAuto === p.id && shown !== p.id ? 'mdi-creation' : undefined"
        @click="choose(p.id)"
      >
        {{ p.name }}
      </VBtn>
    </div>
    <FacetStatus
      :auto-label="poolName(entry.poolAuto)"
      :user-label="poolName(entry.poolUser)"
      :auto-hint="autoHint"
      :disabled="busy"
      @confirm="confirm"
      @use-suggestion="setPool(null)"
    />
  </div>
</template>

<style scoped>
.facet-control {
  display: contents;
}
.pool-buttons {
  max-width: 600px;
}
</style>
