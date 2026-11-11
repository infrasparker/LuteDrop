<script setup lang="ts">
import type { FlagBand, FlagKind } from '@generated/index';
import { FLAG_BANDS } from '../utils/moodFlags';

/// The flag menu's items, shared by the song editor's mood profile and the
/// Moods page: Definitely · Understandable · Stretch · No · Higher · Lower,
/// then (optionally) Open in Moods, then Remove flag when flagged.
defineProps<{ flagged: boolean; openInMoods?: boolean }>();
const emit = defineEmits<{ set: [kind: FlagKind, band: FlagBand | null]; open: []; remove: [] }>();
</script>

<template>
  <VList density="compact">
    <VListItem v-for="b in FLAG_BANDS" :key="b.band" :title="b.menuLabel" @click="emit('set', 'band', b.band)" />
    <VDivider />
    <VListItem title="Higher" @click="emit('set', 'higher', null)" />
    <VListItem title="Lower" @click="emit('set', 'lower', null)" />
    <template v-if="openInMoods || flagged">
      <VDivider />
      <VListItem v-if="openInMoods" title="Open in Moods" prepend-icon="mdi-palette-outline" @click="emit('open')" />
      <VListItem v-if="flagged" title="Remove flag" @click="emit('remove')" />
    </template>
  </VList>
</template>
