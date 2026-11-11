<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { SceneReference } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { useToast } from '../composables/useToast';
import TrackSettings from './TrackSettings.vue';
import TrackClipList from './TrackClipList.vue';
import LibraryBrowser from './LibraryBrowser.vue';
import ShareTrackDialog from './ShareTrackDialog.vue';
import DuplicateTrackDialog from './DuplicateTrackDialog.vue';
import ConfirmDialog from './ConfirmDialog.vue';
const store = useAppStore();
const ui = useSoundstageUiStore();
const track = computed(() => ui.editTrackId ? store.findTrack(ui.editTrackId) : undefined);
const open = computed({ get: () => !!track.value, set: (value) => { if (!value) ui.editTrackId = null; } });
const name = ref('');
const references = ref<SceneReference[]>([]);
const referencesFailed = ref(false);
const referencesLoading = ref(false);
const showShare = ref(false);
const showDuplicate = ref(false);
let referenceRequest = 0;
const showLibrary = ref(false);
const deleteConfirm = ref(false);
const busy = ref(false);
watch(() => ui.editTrackId, () => { name.value = track.value?.name ?? ''; deleteConfirm.value = false; showShare.value = false; showDuplicate.value = false; });
watch([() => ui.editTrackId, () => store.boards], async ([id]) => {
  const request = ++referenceRequest;
  references.value = []; referencesFailed.value = false; referencesLoading.value = !!id;
  if (!id) return;
  try { const result = await invoke<SceneReference[]>('get_track_references', { trackId: id }); if (request === referenceRequest) references.value = result; }
  catch (error) { if (request === referenceRequest) { referencesFailed.value = true; useToast().showError(String(error)); } }
  finally { if (request === referenceRequest) referencesLoading.value = false; }
}, { immediate: true });
async function rename(): Promise<void> {
  if (track.value && name.value.trim() && name.value.trim() !== track.value.name) await store.command('update_track', { trackId: track.value.id, updates: { name: name.value.trim() } });
}
async function remove(): Promise<void> {
  if (!track.value || busy.value || referencesFailed.value || referencesLoading.value) return;
  busy.value = true;
  try {
    if (await store.command('delete_track', { trackId: track.value.id, removeFromScenes: true })) ui.editTrackId = null;
  } finally { busy.value = false; }
}
</script>
<template>
  <VDialog v-model="open" max-width="900" scrollable>
    <VCard v-if="track" rounded="lg">
      <VCardTitle class="d-flex align-center px-6 pt-5"><span>Edit shared track</span><VSpacer /><VMenu location="bottom end"><template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" icon="mdi-dots-vertical" aria-label="Track actions" variant="text" size="small" /></template><VList density="compact"><VListItem title="Add to…" prepend-icon="mdi-playlist-plus" @click="showShare = true" /><VListItem title="Duplicate…" prepend-icon="mdi-content-duplicate" @click="showDuplicate = true" /><VDivider class="my-1" /><VListItem title="Delete shared track…" prepend-icon="mdi-delete-outline" class="text-error" :disabled="referencesLoading || referencesFailed || busy" @click="deleteConfirm = true" /></VList></VMenu><VBtn icon="mdi-close" aria-label="Close track editor" variant="text" size="small" @click="open = false" /></VCardTitle>
      <VCardSubtitle class="px-6">Settings and clips are shared wherever this track is used. Scene selections and volumes are saved separately.</VCardSubtitle>
      <VCardText class="px-6 pt-5">
        <div class="d-flex align-center ga-3 mb-5"><VTextField v-model="name" label="Track name" variant="outlined" density="comfortable" hide-details @keydown.enter="rename" /><VBtn text="Rename" variant="tonal" :disabled="!name.trim() || name.trim() === track.name" @click="rename" /></div>
        <h3 class="text-subtitle-2 mb-3">Playback</h3>
        <TrackSettings :track="track" />
        <VDivider class="my-5" />
        <TrackClipList :key="track.id" :track="track" :is-seamless="track.mode.type === 'seamless'" @open-library-browser="showLibrary = true" />
        <VDivider class="my-5" />
        <p v-if="referencesLoading" class="text-body-2">Loading scene usage…</p>
        <div v-else class="text-subtitle-2 mb-2">Used by {{ references.length }} scene{{ references.length === 1 ? '' : 's' }}</div>
        <p v-if="referencesFailed" class="text-error text-body-2">Could not load scene references.</p>
        <div v-else-if="!referencesLoading && !references.length" class="text-body-2 text-medium-emphasis">Available in the track inventory, even without a scene.</div>
        <VChip v-for="reference in references" :key="reference.sceneId" class="mr-2 mb-2" size="small" :title="reference.configurationName">{{ reference.boardName }} / {{ reference.sceneName }}</VChip>
      </VCardText>
      <VDivider />
      <VCardActions class="px-5 py-3"><VSpacer /><span class="text-caption text-medium-emphasis mr-3">Track settings save automatically</span><VBtn text="Done" variant="tonal" color="primary" @click="open = false" /></VCardActions>
      <LibraryBrowser v-model="showLibrary" :track-id="track.id" />
      <ShareTrackDialog v-model="showShare" :track-id="track.id" />
      <DuplicateTrackDialog v-model="showDuplicate" :track="track" />
    </VCard>
  </VDialog>
  <ConfirmDialog v-model="deleteConfirm" title="Delete shared track" :message="`Delete “${track?.name ?? ''}” from the inventory and all boards? ${references.length} scene reference(s) will be removed. Audio files stay in the Library.`" confirm-label="Delete track and references" confirm-color="error" @confirm="remove" />
</template>
