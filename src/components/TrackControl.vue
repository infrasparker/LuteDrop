<script setup lang="ts">
import { ref, computed, nextTick, onMounted } from 'vue';
import type { Track } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { useAudio } from '../composables/useAudio';
import { useTrackUpdates } from '../composables/useTrackUpdates';
import { useContextMenuState } from '../composables/useContextMenu';
import TrackSettings from './TrackSettings.vue';
import TrackClipList from './TrackClipList.vue';
import ConfirmDialog from './ConfirmDialog.vue';
import PromptDialog from './PromptDialog.vue';
import { useLocalVolume } from '../shared/useLocalVolume';

const props = defineProps<{
  track: Track;
}>();

const emit = defineEmits<{
  'open-library-browser': [trackId: string];
}>();

const store = useAppStore();
const audio = useAudio();
const { updateField } = useTrackUpdates(() => props.track.id);
const { menuVisible, menuX, menuY, openMenu } = useContextMenuState();

const playbackState = computed(() => store.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);
const isActive = computed(() => isPlaying.value || (playbackState.value?.isPaused ?? false));
const isSeamless = computed(() => props.track.mode.type === 'seamless');

// Expansion lives in a store so returning to the Soundstage (e.g. Back from
// Find similar) reopens the same tracks.
const ui = useSoundstageUiStore();
const expanded = computed({
  get: () => (ui.expandedTrackIds.has(props.track.id) ? 'open' : null),
  set: (v: string | null) => ui.setTrackExpanded(props.track.id, v === 'open'),
});

const wrapper = ref<HTMLElement | null>(null);
onMounted(async () => {
  if (ui.focusTrackId !== props.track.id) return;
  ui.focusTrackId = null;
  await nextTick();
  wrapper.value?.scrollIntoView({ block: 'start' });
});

const currentClipName = computed(() => {
  const clipId = playbackState.value?.currentClipId;
  if (!clipId) return null;
  return props.track.clips.find((c) => c.id === clipId)?.name ?? null;
});

// Rename state
const showRenameDialog = ref(false);

// Delete state
const showDeleteConfirm = ref(false);

function openContextMenu(event: MouseEvent): void {
  openMenu(event);
}

function openRenameDialog(): void {
  showRenameDialog.value = true;
}

async function renameTrack(name: string): Promise<void> {
  await updateField({ name });
}

function confirmOrDeleteTrack(): void {
  if (props.track.clips.length === 0) {
    deleteTrack();
  } else {
    showDeleteConfirm.value = true;
  }
}

async function deleteTrack(): Promise<void> {
  await audio.stopTrack(props.track.id);
  await store.command('delete_track', { trackId: props.track.id });
}

function playTrack(): void {
  audio.playTrack(props.track.id);
}

function pauseTrack(): void {
  audio.pauseTrack(props.track.id);
}

function stopTrack(): void {
  audio.stopTrack(props.track.id);
}

// Local ref for smooth slider dragging — audio engine persists to DB directly
const { local: localVolume, set: setVolume } = useLocalVolume(() => props.track.volume, (v) => audio.setTrackVolume(props.track.id, v));

const isMuted = computed(() => store.isTrackMuted(props.track.id));

function toggleMute(): void {
  audio.toggleTrackMute(props.track.id);
}
</script>

<template>
  <div ref="wrapper" :class="{ 'track-wrapper': true, 'track-active': isActive }">
    <VExpansionPanels v-model="expanded">
      <VExpansionPanel
        value="open"
        color="surface-light"
        rounded="0"
        elevation="0"
        @contextmenu.prevent="openContextMenu"
      >
        <template #title>
          <div
            class="d-flex align-center ga-2"
            style="width: 100%;"
          >
            <VBtn
              :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
              variant="tonal"
              size="small"
              rounded="circle"
              @click.stop="isPlaying ? pauseTrack() : playTrack()"
            />
            <VBtn
              icon="mdi-stop"
              variant="tonal"
              size="small"
              rounded="circle"
              @click.stop="stopTrack"
            />

            <span class="text-subtitle-2 font-weight-bold" :style="{ opacity: isMuted ? 0.5 : 1 }">{{ track.name }}</span>

            <VSpacer />

            <span v-if="currentClipName" class="text-caption text-primary" style="white-space: nowrap;">
              {{ currentClipName }}
            </span>
            <span class="text-caption text-medium-emphasis" style="white-space: nowrap;">
              {{ track.clips.length }} clip{{ track.clips.length !== 1 ? 's' : '' }}
              &middot; {{ track.mode.type }}
            </span>

            <VBtn
              :icon="isMuted ? 'mdi-volume-off' : 'mdi-volume-high'"
              :color="isMuted ? 'error' : undefined"
              variant="text"
              size="x-small"
              :title="isMuted ? 'Unmute track' : 'Mute track'"
              @click.stop="toggleMute"
            />
            <VSlider
              :model-value="localVolume"
              @update:model-value="setVolume"
              @click.stop
              @mousedown.stop
              :min="0"
              :max="1"
              :step="0.01"
              hide-details
              density="compact"
              class="mr-4"
              style="flex: 0 0 120px;"
              :style="{ opacity: isMuted ? 0.4 : 1 }"
            />
          </div>
        </template>

        <template #text>
          <div class="pt-2">
            <TrackSettings :track="track" />
            <TrackClipList :track="track" :is-seamless="isSeamless" @open-library-browser="(id) => emit('open-library-browser', id)" />
          </div>
        </template>
      </VExpansionPanel>
    </VExpansionPanels>

    <!-- Context menu -->
    <VMenu v-model="menuVisible" :target="[menuX, menuY]" close-on-content-click>
      <VList density="compact">
        <VListItem @click="openRenameDialog">
          <template #prepend><VIcon>mdi-pencil</VIcon></template>
          <VListItemTitle>Rename</VListItemTitle>
        </VListItem>
        <VListItem @click="confirmOrDeleteTrack" class="text-error">
          <template #prepend><VIcon color="error">mdi-delete</VIcon></template>
          <VListItemTitle>Delete</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>

    <!-- Rename dialog -->
    <PromptDialog
      v-model="showRenameDialog"
      title="Rename Track"
      label="Track Name"
      :initial="track.name"
      confirm-label="Rename"
      @submit="renameTrack"
    />

    <!-- Delete confirmation -->
    <ConfirmDialog
      v-model="showDeleteConfirm"
      title="Delete Track"
      :message="`Are you sure you want to delete &quot;${track.name}&quot;?`"
      confirm-label="Delete"
      confirm-color="error"
      @confirm="deleteTrack"
    />
  </div>
</template>

<style scoped>
.track-wrapper {
  border-left: 3px solid transparent;
}
.track-active {
  border-left: 3px solid #F0A830;
}
.track-active :deep(.v-expansion-panel-title) {
  background: rgba(240, 168, 48, 0.08);
}
</style>

<style>
/* Global styles for SortableJS (not scoped — SortableJS adds classes directly to DOM) */
.sortable-ghost {
  visibility: hidden;
}

.sortable-fallback {
  opacity: 0.9;
  background: #1E1E24;
  border-radius: 4px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
}

/* Force grabbing cursor on everything while dragging */
body.sortable-dragging,
body.sortable-dragging * {
  cursor: grabbing !important;
}
</style>
