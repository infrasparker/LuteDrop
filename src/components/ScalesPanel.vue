<script setup lang="ts">
import type { LibraryEntry } from '@generated/index';
import { MOOD_SCALES, SCALE_NAMES, scaleValues } from '../utils/mood';
import MoodScaleControl from './MoodScaleControl.vue';

/// The three mood scales in a collapsed "Scales" expander. They are sort keys
/// and shade the Review readout word; most tracks never need them touched.
defineProps<{ entry: LibraryEntry }>();
</script>

<template>
  <VExpansionPanels variant="accordion">
    <VExpansionPanel value="scales">
      <VExpansionPanelTitle class="text-body-2">Scales</VExpansionPanelTitle>
      <VExpansionPanelText>
        <div class="scales-grid">
          <template v-for="scale in MOOD_SCALES" :key="scale">
            <span class="text-caption text-medium-emphasis">{{ SCALE_NAMES[scale] }}</span>
            <MoodScaleControl
              :entry-ids="[entry.id]"
              :scale="scale"
              :z="scaleValues(entry, scale).z"
              :user="scaleValues(entry, scale).user"
            />
          </template>
        </div>
      </VExpansionPanelText>
    </VExpansionPanel>
  </VExpansionPanels>
</template>

<style scoped>
/* Label | options | status, as in the editor's facet grid. */
.scales-grid {
  display: grid;
  grid-template-columns: max-content max-content 1fr;
  align-items: center;
  column-gap: 12px;
  row-gap: 12px;
}
</style>
