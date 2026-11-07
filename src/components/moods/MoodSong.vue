<script setup lang="ts">
import { computed } from 'vue';
import type { FlagBand, FlagKind } from '@generated/index';
import { useAppStore } from '../../stores/appStore';
import { useLibraryPreview } from '../../composables/useLibraryPreview';
import { albumOf } from '../../utils/moodTune';
import MoodFlagMenu from '../MoodFlagMenu.vue';

/// A song on the Moods page: its name (click opens the song editor; artist
/// and album in the tooltip), with play and flag buttons that show on hover
/// or focus of the row. The flag menu can also be opened from the row (F).
const props = defineProps<{ entryId: string; moodId: string; name: string }>();
const menuOpen = defineModel<boolean>('menu', { default: false });

const store = useAppStore();
const preview = useLibraryPreview();
const entry = computed(() => store.findLibraryEntry(props.entryId));
const album = computed(() => (entry.value ? albumOf(entry.value.filePath) : ''));
const flagged = computed(() => entry.value?.moods.some((m) => m.mood === props.moodId && m.flag !== null) ?? false);
const playing = computed(() => preview.playingId.value === props.entryId);

function setFlag(kind: FlagKind, band: FlagBand | null): void {
  void store.command('set_mood_flag', { entryId: props.entryId, mood: props.moodId, kind, band });
}
function clearFlag(): void {
  void store.command('clear_mood_flag', { entryId: props.entryId, mood: props.moodId });
}
</script>

<template>
  <span class="mood-song">
    <button type="button" class="song-name text-truncate" @click="store.editEntryId = entryId">
      {{ name }}
      <VTooltip v-if="album" activator="parent" location="top" open-delay="400">{{ album }}</VTooltip>
    </button>
    <span class="song-actions" :class="{ 'is-on': playing || menuOpen }">
      <VBtn
        icon
        size="x-small"
        variant="text"
        :aria-label="playing ? `Stop ${name}` : `Play ${name}`"
        @click="preview.toggle(entryId)"
      >
        <VIcon size="small">{{ playing ? 'mdi-stop' : 'mdi-play' }}</VIcon>
      </VBtn>
      <VMenu v-model="menuOpen" location="bottom end">
        <template #activator="{ props: menuProps }">
          <VBtn v-bind="menuProps" icon size="x-small" variant="text" :aria-label="`Flag ${name}`">
            <VIcon size="small">{{ flagged ? 'mdi-flag' : 'mdi-flag-outline' }}</VIcon>
          </VBtn>
        </template>
        <MoodFlagMenu :flagged="flagged" @set="setFlag" @remove="clearFlag" />
      </VMenu>
    </span>
  </span>
</template>

<style scoped>
.mood-song {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: 2px;
}
.song-name {
  appearance: none;
  background: transparent;
  border: 0;
  min-width: 0;
  text-align: left;
  color: inherit;
  font: inherit;
  padding: 4px 0;
  min-height: 28px;
  cursor: pointer;
}
.song-name:hover,
.song-name:focus-visible {
  text-decoration: underline;
}
.song-actions {
  display: inline-flex;
  flex: 0 0 auto;
  opacity: 0;
}
.song-actions.is-on,
:global(.song-row:hover) .song-actions,
:global(.song-row:focus-within) .song-actions {
  opacity: 1;
}
</style>
