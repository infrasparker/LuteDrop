<script setup lang="ts">
import { computed, ref, watch, nextTick, onMounted, onUnmounted } from 'vue';
import type { Track } from '@generated/index';
import { useRouter } from 'vue-router';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import Sortable from 'sortablejs';
import ClipRow from './ClipRow.vue';
import FindSimilarButton from './FindSimilarButton.vue';

const props = defineProps<{
  track: Track;
  isSeamless: boolean;
}>();

const emit = defineEmits<{
  'open-library-browser': [trackId: string];
}>();

const store = useAppStore();


function addClips(): void {
  emit('open-library-browser', props.track.id);
}

// --- Find similar ---

// Distinct music entries behind the track's clips.
const similarSeeds = computed(() => {
  const ids = new Set<string>();
  for (const clip of props.track.clips) {
    const entry = store.findLibraryEntry(clip.libraryEntryId);
    if (entry?.entryType === 'music') ids.add(entry.id);
  }
  return [...ids];
});

// Opens Scry seeded with the track's music, targeting this track.
// The Soundstage scrolls back to this track on return.
const router = useRouter();
const ui = useSoundstageUiStore();
function findSimilar(): void {
  ui.focusTrackId = props.track.id;
  ui.editTrackId = null;
  router.push({ name: 'scry', query: { seed: similarSeeds.value, target: props.track.id } });
}

// --- Clip reordering ---

const clipsContainer = ref<HTMLElement | null>(null);
let sortableInstance: Sortable | null = null;

function initSortable(): void {
  if (sortableInstance) {
    sortableInstance.destroy();
    sortableInstance = null;
  }
  if (!clipsContainer.value) return;
  sortableInstance = Sortable.create(clipsContainer.value, {
    handle: '.drag-handle',
    animation: 200,
    forceFallback: true,
    fallbackClass: 'stage-drag-preview',
    ghostClass: 'stage-drag-placeholder',
    fallbackOnBody: true,
    onStart: () => {
      document.body.classList.add('sortable-dragging');
    },
    onEnd: async (event) => {
      document.body.classList.remove('sortable-dragging');
      if (event.oldIndex === undefined || event.newIndex === undefined) return;
      // Restore Sortable's DOM move before Vue applies the authoritative order.
      const siblings = [...event.from.children].filter((el) => el !== event.item);
      event.from.insertBefore(event.item, siblings[event.oldIndex] ?? null);
      const clips = [...props.track.clips];
      const [moved] = clips.splice(event.oldIndex, 1);
      clips.splice(event.newIndex, 0, moved);
      await store.command('update_track', {
        trackId: props.track.id,
        updates: { clips },
      });
    },
  });
}

onMounted(() => {
  nextTick(() => initSortable());
});

watch(clipsContainer, (el) => {
  if (el) nextTick(() => initSortable());
});

onUnmounted(() => {
  if (sortableInstance) sortableInstance.destroy();
});
</script>

<template>
  <div>
    <p v-if="isSeamless" class="text-caption text-medium-emphasis mb-3">Every clip is an alternative. Choose one from the track card to loop it.</p>
    <div class="d-flex align-center mb-2">
      <span class="text-subtitle-2">Clips</span>
      <VSpacer />
      <FindSimilarButton :disabled="similarSeeds.length === 0" @click="findSimilar" />
      <VBtn prepend-icon="mdi-plus" variant="tonal" rounded="xl" @click="addClips">
        Add Clips
      </VBtn>
    </div>

    <div ref="clipsContainer" class="clip-list">
      <ClipRow
        v-for="clip in track.clips"
        :key="clip.id"
        :clip="clip"
        :track-id="track.id"
      />
    </div>

    <div v-if="track.clips.length === 0" class="text-caption text-medium-emphasis">
      No clips yet.
    </div>
  </div>
</template>
<style scoped>
.clip-list { display: flex; flex-direction: column; gap: 6px; }
</style>
