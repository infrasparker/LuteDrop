<script setup lang="ts">
import { computed, ref, watch, nextTick, onMounted, onUnmounted } from 'vue';
import type { Track } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { open } from '@tauri-apps/plugin-dialog';
import Sortable from 'sortablejs';
import ClipRow from './ClipRow.vue';

const props = defineProps<{
  track: Track;
  compId: string;
  isSeamless: boolean;
}>();

const store = useAppStore();

const hasClips = computed(() => props.track.clips.length > 0);
const seamlessClip = computed(() => props.track.clips[0] ?? null);
const canAddClips = computed(() => !props.isSeamless || !hasClips.value);

async function addClips(): Promise<void> {
  const paths = await open({
    multiple: !props.isSeamless,
    filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'] }],
  });
  if (!paths) return;
  const pathList = Array.isArray(paths) ? paths : [paths];
  for (const filePath of pathList) {
    const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';
    await store.command('add_clip', { trackId: props.track.id, name: fileName, filePath });
  }
}

// --- SortableJS for sequence/shuffle clip reordering ---

const clipsContainer = ref<HTMLElement | null>(null);
let sortableInstance: Sortable | null = null;

function initSortable(): void {
  if (sortableInstance) {
    sortableInstance.destroy();
    sortableInstance = null;
  }
  if (!clipsContainer.value || props.isSeamless) return;
  sortableInstance = Sortable.create(clipsContainer.value, {
    handle: '.drag-handle',
    animation: 200,
    forceFallback: true,
    fallbackClass: 'sortable-fallback',
    ghostClass: 'sortable-ghost',
    fallbackOnBody: true,
    onStart: () => {
      document.body.classList.add('sortable-dragging');
    },
    onEnd: async (event) => {
      document.body.classList.remove('sortable-dragging');
      if (event.oldIndex === undefined || event.newIndex === undefined) return;
      const clips = [...props.track.clips];
      const [moved] = clips.splice(event.oldIndex, 1);
      clips.splice(event.newIndex, 0, moved);
      await store.command('update_track', {
        compId: props.compId,
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
  <!-- Seamless mode: single clip -->
  <div v-if="isSeamless">
    <div class="d-flex align-center mb-2">
      <span class="text-subtitle-2">Clip</span>
      <VSpacer />
      <VBtn
        v-if="!hasClips"
        prepend-icon="mdi-plus"
        variant="tonal"
        rounded="xl"
        @click="addClips"
      >
        Add Clip
      </VBtn>
    </div>
    <div v-if="hasClips">
      <ClipRow :clip="seamlessClip!" :track-id="track.id" :comp-id="compId" />
    </div>
    <div v-else class="text-caption text-medium-emphasis">
      No clip yet. Seamless mode uses a single clip.
    </div>
  </div>

  <!-- Sequence/Shuffle mode: clips list with drag reorder -->
  <div v-else>
    <div class="d-flex align-center mb-2">
      <span class="text-subtitle-2">Clips</span>
      <VSpacer />
      <VBtn prepend-icon="mdi-plus" variant="tonal" rounded="xl" @click="addClips">
        Add Clips
      </VBtn>
    </div>

    <div ref="clipsContainer">
      <ClipRow
        v-for="clip in track.clips"
        :key="clip.id"
        :clip="clip"
        :track-id="track.id"
        :comp-id="compId"
      />
    </div>

    <div v-if="track.clips.length === 0" class="text-caption text-medium-emphasis">
      No clips yet.
    </div>
  </div>
</template>
