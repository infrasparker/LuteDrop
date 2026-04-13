<script setup lang="ts">
import { computed, watch } from 'vue';
import type { LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';

const props = defineProps<{
  entry: LibraryEntry;
  selectable?: boolean;
  selected?: boolean;
}>();

const emit = defineEmits<{
  select: [entryId: string];
  click: [];
}>();

const store = useAppStore();
const audio = useAudio();

const previewState = computed(() => store.playbackStates.get('__library'));
const isPreviewPlaying = computed(() => store.libraryPlayingEntryId === props.entry.id && previewState.value?.isPlaying);

watch(previewState, (state) => {
  if (!state?.isPlaying && store.libraryPlayingEntryId === props.entry.id) {
    store.libraryPlayingEntryId = null;
  }
});

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

function formatDuration(seconds: number | null): string {
  if (seconds === null) return '--:--';
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

async function removeTag(tag: string): Promise<void> {
  const newTags = props.entry.tags.filter((t) => t !== tag);
  await store.command('update_library_entry_tags', {
    entryId: props.entry.id,
    tags: newTags,
  });
}

function handleClick(): void {
  if (props.selectable) {
    emit('select', props.entry.id);
  } else {
    emit('click');
  }
}
</script>

<template>
  <VListItem
    :class="[
      selected ? 'bg-primary-lighten-4' : '',
      store.isEntryMissing(entry.id) ? 'opacity-50' : '',
      'cursor-pointer',
      isPreviewPlaying ? 'preview-active' : '',
    ]"
    rounded
    @click="handleClick"
  >
    <template #prepend>
      <VIcon v-if="store.isEntryMissing(entry.id)" color="warning" class="mr-2">mdi-alert</VIcon>
      <VIcon v-else class="mr-2">mdi-music-note</VIcon>
    </template>

    <VListItemTitle class="font-weight-medium">{{ entry.name }}</VListItemTitle>

    <VListItemSubtitle>
      <span class="mr-3">{{ formatDuration(entry.duration) }}</span>
      <VChip
        v-for="tag in entry.tags"
        :key="tag"
        size="small"
        class="mr-1"
        closable
        @click:close.stop="removeTag(tag)"
        @click.stop
      >
        {{ tag }}
      </VChip>
    </VListItemSubtitle>

    <template #append>
      <VBtn
        :icon="isPreviewPlaying ? 'mdi-stop' : 'mdi-play'"
        variant="text"
        size="small"
        @click.stop="togglePreview"
      />
      <slot name="actions" />
    </template>
  </VListItem>
</template>

<style scoped>
.preview-active {
  background: rgba(240, 168, 48, 0.08) !important;
  border-left: 3px solid #F0A830;
}
</style>
