<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Track } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useTrackUpdates } from '../composables/useTrackUpdates';
import { useContextMenuState } from '../composables/useContextMenu';
import TrackSettings from './TrackSettings.vue';
import TrackClipList from './TrackClipList.vue';

const props = defineProps<{
  track: Track;
  compId: string;
}>();

const emit = defineEmits<{
  'open-library-browser': [trackId: string];
}>();

const store = useAppStore();
const audio = useAudio();
const { updateField } = useTrackUpdates(() => props.track.id, () => props.compId);
const { menuVisible, menuX, menuY, openMenu } = useContextMenuState();

const playbackState = computed(() => store.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);
const isActive = computed(() => isPlaying.value || (playbackState.value?.isPaused ?? false));
const isSeamless = computed(() => props.track.mode.type === 'seamless');

const currentClipName = computed(() => {
  const clipId = playbackState.value?.currentClipId;
  if (!clipId) return null;
  return props.track.clips.find((c) => c.id === clipId)?.name ?? null;
});

// Rename state
const showRenameDialog = ref(false);
const renameTrackName = ref('');

// Delete state
const showDeleteConfirm = ref(false);

function openContextMenu(event: MouseEvent): void {
  openMenu(event);
}

function openRenameDialog(): void {
  renameTrackName.value = props.track.name;
  showRenameDialog.value = true;
}

async function renameTrack(): Promise<void> {
  if (!renameTrackName.value.trim()) return;
  await updateField({ name: renameTrackName.value.trim() });
  showRenameDialog.value = false;
}

function confirmOrDeleteTrack(): void {
  if (props.track.clips.length === 0) {
    deleteTrack();
  } else {
    showDeleteConfirm.value = true;
  }
}

async function deleteTrack(): Promise<void> {
  showDeleteConfirm.value = false;
  await audio.stopTrack(props.track.id);
  await store.command('delete_track', { compId: props.compId, trackId: props.track.id });
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

async function setVolume(v: number): Promise<void> {
  audio.setTrackVolume(props.track.id, v);
  await updateField({ volume: v });
}
</script>

<template>
  <div>
    <VExpansionPanels>
      <VExpansionPanel
        color="surface-light"
        rounded="0"
        elevation="0"
        :style="isActive ? { borderLeft: '3px solid #FF6D00' } : { borderLeft: '3px solid transparent' }"
        @contextmenu.prevent="openContextMenu"
      >
        <template #title>
          <div
            class="d-flex align-center ga-2"
            style="width: 100%;"
          >
            <VBtn
              :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
              :color="isPlaying ? 'secondary' : 'primary'"
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

            <span class="text-subtitle-2 font-weight-bold">{{ track.name }}</span>

            <VSpacer />

            <span v-if="currentClipName" class="text-caption text-primary" style="white-space: nowrap;">
              {{ currentClipName }}
            </span>
            <span class="text-caption text-medium-emphasis" style="white-space: nowrap;">
              {{ track.clips.length }} clip{{ track.clips.length !== 1 ? 's' : '' }}
              &middot; {{ track.mode.type }}
            </span>

            <VSlider
              :model-value="track.volume"
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
            />
          </div>
        </template>

        <template #text>
          <div class="pt-2">
            <TrackSettings :track="track" :comp-id="compId" />
            <TrackClipList :track="track" :comp-id="compId" :is-seamless="isSeamless" @open-library-browser="(id) => emit('open-library-browser', id)" />
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
    <VDialog v-model="showRenameDialog" max-width="400">
      <VCard title="Rename Track">
        <VCardText>
          <VTextField
            v-model="renameTrackName"
            label="Track Name"
            variant="outlined"
            autofocus
            @keydown.enter="renameTrack"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showRenameDialog = false" />
          <VBtn text="Rename" color="primary" variant="tonal" @click="renameTrack" />
        </VCardActions>
      </VCard>
    </VDialog>

    <!-- Delete confirmation -->
    <VDialog v-model="showDeleteConfirm" max-width="450">
      <VCard title="Delete Track">
        <VCardText>
          Are you sure you want to delete "{{ track.name }}"?
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showDeleteConfirm = false" />
          <VBtn text="Delete" color="error" variant="tonal" @click="deleteTrack" />
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>

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
