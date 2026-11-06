<script setup lang="ts">
import { computed } from 'vue';
import type { LibraryEntry } from '@generated/index';
import {
  countByPool,
  POOLS,
  SORT_HINTS,
  SORT_LABELS,
  type PoolChoice,
  type PoolFilters,
  type SortKey,
} from '../utils/pools';

/// The filter bar shared by the Library and Scry: pool chips with counts (in
/// vocabulary order, any number at once, plus Unsorted for music with no pool
/// shown), the Role toggle, and a Sort menu. Music only, so callers hide it
/// for ambiance. `entries` are the entries the chip counts are taken from;
/// `sortKeys` are the sort options offered, in menu order.
const filters = defineModel<PoolFilters>({ required: true });
const sort = defineModel<SortKey>('sort', { required: true });
const props = defineProps<{
  size: 'small' | 'x-small';
  entries: LibraryEntry[];
  sortKeys: SortKey[];
}>();

const counts = computed(() => countByPool(props.entries));

const chips = computed<{ value: PoolChoice; label: string; count: number }[]>(() => [
  ...POOLS.map((p) => ({ value: p.id as PoolChoice, label: p.name, count: counts.value.get(p.id) ?? 0 })),
  { value: 'unsorted', label: 'Unsorted', count: counts.value.get('unsorted') ?? 0 },
]);

function setPools(pools: PoolChoice[]): void {
  filters.value = { ...filters.value, pools };
}

function setRole(role: PoolFilters['role']): void {
  filters.value = { ...filters.value, role };
}
</script>

<template>
  <div class="d-flex flex-wrap align-center ga-2">
    <VChipGroup
      :model-value="filters.pools"
      multiple
      column
      selected-class="text-primary"
      class="pool-chips py-0"
      @update:model-value="setPools"
    >
      <VChip
        v-for="chip in chips"
        :key="chip.value"
        :value="chip.value"
        :size="size === 'x-small' ? 'x-small' : 'small'"
        variant="outlined"
        filter
      >
        {{ chip.label }}<span class="text-medium-emphasis ml-1">{{ chip.count }}</span>
      </VChip>
    </VChipGroup>
    <VBtnToggle
      :model-value="filters.role"
      mandatory
      density="compact"
      variant="outlined"
      divided
      @update:model-value="setRole"
    >
      <VBtn value="all" :size="size">Any role</VBtn>
      <VBtn value="combat" :size="size" prepend-icon="mdi-sword-cross">Combat</VBtn>
      <VBtn value="nonCombat" :size="size" prepend-icon="mdi-music-note">Non-combat</VBtn>
      <VBtn value="unset" :size="size">Unset</VBtn>
    </VBtnToggle>
    <VMenu location="bottom start">
      <template #activator="{ props: activator }">
        <VBtn v-bind="activator" :size="size" variant="outlined" prepend-icon="mdi-sort" append-icon="mdi-menu-down">
          Sort: {{ SORT_LABELS[sort] }}
        </VBtn>
      </template>
      <VList density="compact">
        <VListItem
          v-for="key in sortKeys"
          :key="key"
          :title="SORT_LABELS[key]"
          :subtitle="SORT_HINTS[key]"
          :active="key === sort"
          color="primary"
          @click="sort = key"
        />
      </VList>
    </VMenu>
  </div>
</template>

<style scoped>
.pool-chips {
  max-width: 100%;
}
</style>
