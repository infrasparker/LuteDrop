<script setup lang="ts">
import { computed } from 'vue';
import type { AutoTag, LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import TagChip from './TagChip.vue';
import { useAudio } from '../composables/useAudio';
import { formatDuration } from '../shared/format';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';

const props = defineProps<{
  entry: LibraryEntry;
  selected?: boolean;
}>();

const emit = defineEmits<{
  select: [entryId: string];
}>();

const store = useAppStore();
const audio = useAudio();

const previewState = computed(() => store.playbackStates.get(LIBRARY_PLAYBACK_ID));
const isPreviewPlaying = computed(() => store.libraryPlayingEntryId === props.entry.id && previewState.value?.isPlaying);

function togglePreview(): void {
  if (isPreviewPlaying.value) {
    audio.stopLibrarySound();
  } else {
    audio.playLibrarySound(
      props.entry.id,
      props.entry.filePath,
      props.entry.trimStart ?? 0,
      props.entry.volume,
      props.entry.trimStart, props.entry.trimEnd,
      props.entry.loopStart, props.entry.loopEnd,
    );
  }
}

async function removeTag(tag: string): Promise<void> {
  const newTags = props.entry.tags.filter((t) => t !== tag);
  await store.command('update_library_entry_tags', {
    entryId: props.entry.id,
    tags: newTags,
    seenAutoTags: props.entry.autoTags.map((a) => a.tag),
  });
}

function autoFor(tag: string): AutoTag | null {
  return props.entry.autoTags.find((a) => a.tag === tag) ?? null;
}

function handleClick(): void {
  emit('select', props.entry.id);
}

/// Opens the full waveform editor on top of the picker so the file can be
/// scrubbed and auditioned before deciding to add it.
function openEditor(): void {
  store.editEntryId = props.entry.id;
}
</script>

<template>
  <VListItem
    :class="[
      store.isEntryMissing(entry.id) ? 'opacity-50' : '',
      'cursor-pointer',
      isPreviewPlaying ? 'preview-active' : '',
      selected ? 'entry-selected' : '',
    ]"
    :active="selected"
    color="primary"
    rounded
    @click="handleClick"
  >
    <template #prepend>
      <VCheckboxBtn :model-value="selected" density="compact" class="mr-1" />
      <VIcon v-if="store.isEntryMissing(entry.id)" color="warning" class="mr-2">mdi-alert</VIcon>
      <VIcon v-else class="mr-2">mdi-music-note</VIcon>
    </template>

    <VListItemTitle class="font-weight-medium">
      <span class="entry-name" title="Open details" @click.stop="openEditor">{{ entry.name }}</span>
    </VListItemTitle>

    <VListItemSubtitle>
      <span class="mr-3">{{ formatDuration(entry.duration) }}</span>
      <TagChip
        v-for="tag in entry.tags"
        :key="tag"
        :tag="tag"
        :auto="autoFor(tag)"
        :entry-id="entry.id"
        class="mr-1"
        closable
        @remove="removeTag(tag)"
        @click.stop
      />
    </VListItemSubtitle>

    <template #append>
      <VBtn
        :icon="isPreviewPlaying ? 'mdi-stop' : 'mdi-play'"
        variant="text"
        size="small"
        @click.stop="togglePreview"
      />
      <VBtn
        icon="mdi-waveform"
        variant="text"
        size="small"
        title="Open in editor (scrub, trim, loop)"
        @click.stop="openEditor"
      />
    </template>
  </VListItem>
</template>

<style scoped>
.preview-active {
  background: rgba(240, 168, 48, 0.08) !important;
  border-left: 3px solid #F0A830;
}
.entry-name {
  cursor: pointer;
}
.entry-name:hover {
  text-decoration: underline;
}
.entry-selected {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
}
</style>
