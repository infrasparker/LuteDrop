<script setup lang="ts">
import { computed, ref, watch, nextTick, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { SceneReference, Track } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { useAudio } from '../composables/useAudio';
import { useLocalVolume } from '../shared/useLocalVolume';
import { playbackLabel } from '../utils/soundstage';
import ShareTrackDialog from './ShareTrackDialog.vue';
import DuplicateTrackDialog from './DuplicateTrackDialog.vue';
import TrackSettings from './TrackSettings.vue';
import ReorderGrip from './ReorderGrip.vue';
import TrackClipList from './TrackClipList.vue';
import LibraryBrowser from './LibraryBrowser.vue';
import ConfirmDialog from './ConfirmDialog.vue';
import { useToast } from '../composables/useToast';
const props = defineProps<{ track: Track; boardId?: string; sceneId?: string; index?: number; count?: number }>();
const store = useAppStore();
const ui = useSoundstageUiStore();
const audio = useAudio();
const playback = computed(() => store.playbackStates.get(props.track.id));
const active = computed(() => !!(playback.value?.isPlaying || playback.value?.isPaused));
const selected = computed(() => playback.value?.currentClipId ?? (manual.value ? store.trackMix(props.track.id).selectedClipId ?? props.track.clips[0]?.id : null));
const current = computed(() => props.track.clips.find((clip) => clip.id === selected.value));
const manual = computed(() => props.track.mode.type === 'seamless' || props.track.mode.type === 'oneShot');
const { local: volume, set: setVolume } = useLocalVolume(() => store.trackMix(props.track.id).volume, (value) => audio.setTrackVolume(props.track.id, value));
const expanded = computed({ get: () => ui.expandedTrackIds.has(props.track.id), set: (value) => ui.setTrackExpanded(props.track.id, value) });
const references = ref<SceneReference[]>([]);
const referencesLoading = ref(false);
const referencesFailed = ref(false);
const showLibrary = ref(false);
const removeBoard = ref(false);
const draftName = ref(props.track.name);
const boardScenes = computed(() => store.boards.find((board) => board.id === props.boardId)?.scenes.filter((scene) => scene.tracks.some((state) => state.trackId === props.track.id)) ?? []);
const showShare = ref(false);
const showDuplicate = ref(false);
watch(() => props.track.name, (value) => { draftName.value = value; });
watch([expanded, () => store.boards], async ([value]) => {
  if (!value) return;
  referencesLoading.value = true; referencesFailed.value = false;
  try { references.value = await invoke<SceneReference[]>('get_track_references', { trackId: props.track.id }); }
  catch (error) { referencesFailed.value = true; useToast().showError(String(error)); }
  finally { referencesLoading.value = false; }
}, { immediate: true });
async function rename(): Promise<void> {
  const name = draftName.value.trim();
  if (name && name !== props.track.name) await store.command('update_track', { trackId: props.track.id, updates: { name } });
}
async function removeFromBoard(): Promise<void> {
  if (await store.command('detach_track', { boardId: props.boardId, trackId: props.track.id, removeFromScenes: true })) removeBoard.value = false;
}
const extra = ref<string | null>(null);
const wrapper = ref<HTMLElement | null>(null);
const busy = ref(false);
async function focus(): Promise<void> { if (ui.focusTrackId !== props.track.id) return; await nextTick(); ui.focusTrackId = null; wrapper.value?.scrollIntoView({ block: 'nearest' }); }
onMounted(focus); watch(() => ui.focusTrackId, focus);
function toggle(): void { if (active.value) void audio.stopTrack(props.track.id); else void audio.playTrack(props.track.id); }
async function reorder(delta: number): Promise<void> {
  if (!props.boardId || props.index === undefined || busy.value) return;
  busy.value = true;
  try { await store.command(props.sceneId ? 'reorder_scene_track' : 'reorder_track', { boardId: props.boardId, sceneId: props.sceneId, trackId: props.track.id, newIndex: props.index + delta }); }
  finally { busy.value = false; }
}
</script>
<template>
  <article ref="wrapper" :data-reorder-id="track.id" class="track-card mixer-surface" :class="{ 'mixer-active': active, 'track-expanded': expanded }" :aria-label="`${track.name} track`">
    <div class="track-head">
      <ReorderGrip class="track-drag-handle" :label="`Drag track ${track.name} to reorder`" />
      <VBtn :icon="active ? 'mdi-stop' : 'mdi-play'" :aria-label="`${active ? 'Stop' : 'Play'} ${track.name}`" :title="`${active ? 'Stop' : 'Play'} ${track.name}`" variant="tonal" :color="active ? 'primary' : undefined" size="small" rounded="circle" :disabled="!track.clips.length" @click="toggle" />
      <button type="button" class="track-heading" :aria-label="`${expanded ? 'Collapse' : 'Expand'} ${track.name} track`" :aria-expanded="expanded" @click="expanded = !expanded"><span class="track-title" :title="track.name">{{ track.name }}</span><span class="track-mode">{{ playbackLabel(track.mode) }}<span v-if="playback?.isPaused"> · Paused</span></span></button>
      <div class="track-volume"><VSlider :model-value="volume" :aria-label="`${track.name} volume`" @update:model-value="setVolume" :min="0" :max="1" :step="0.01" hide-details density="compact" /><span>{{ Math.round(volume * 100) }}%</span></div>
      <VBtn :icon="expanded ? 'mdi-chevron-up' : 'mdi-chevron-down'" :aria-label="`${expanded ? 'Close editor for' : 'Edit'} ${track.name}`" :aria-expanded="expanded" title="Edit shared track settings" variant="text" size="x-small" @click="expanded = !expanded" />
      <VMenu location="bottom end">
        <template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" icon="mdi-dots-vertical" variant="text" size="x-small" :aria-label="`Actions for ${track.name}`" /></template>
        <VList density="compact" min-width="220">
          <VListItem prepend-icon="mdi-playlist-plus" title="Add to…" @click="showShare = true" />
          <VListItem prepend-icon="mdi-content-duplicate" title="Duplicate…" @click="showDuplicate = true" />
          <template v-if="boardId">
            <VDivider class="my-1" />
            <VListItem prepend-icon="mdi-arrow-up" title="Move up" :disabled="busy || !index" @click="reorder(-1)" />
            <VListItem prepend-icon="mdi-arrow-down" title="Move down" :disabled="busy || index === (count ?? 0) - 1" @click="reorder(1)" />
            <VDivider class="my-1" />
            <VListItem v-if="sceneId" prepend-icon="mdi-minus-circle-outline" title="Remove from this scene" @click="store.command('detach_scene_track', { sceneId, trackId: track.id })" />
            <VListItem prepend-icon="mdi-tab-remove" v-if="!sceneId" title="Remove from board…" @click="removeBoard = true" />
          </template>
        </VList>
      </VMenu>
    </div>
    <div v-if="!track.clips.length" class="track-empty"><VBtn text="Add clips from Library" prepend-icon="mdi-plus" variant="text" size="small" @click="expanded = true" /></div>
    <div v-else class="track-body">
      <div v-if="!manual" class="now-playing"><VIcon size="16" :color="active ? 'primary' : undefined">mdi-music-note</VIcon><span>{{ active ? current?.name ?? 'Waiting for next clip' : track.mode.type === 'shuffle' ? 'Random starting clip' : 'Ready to play' }}</span><span v-if="active && playback && !playback.hasSound" class="text-caption text-medium-emphasis">Waiting for next clip</span></div>
      <div class="clip-choices" :aria-label="`${track.name} clip choices`">
        <VBtn v-for="clip in track.clips.slice(0, 5)" :key="clip.id" size="small" variant="outlined" class="mixer-choice" :class="{ 'mixer-choice-active': active && selected === clip.id }" :color="active && selected === clip.id ? 'primary' : undefined" :aria-label="`Play ${clip.name} on ${track.name}`" :aria-pressed="active && selected === clip.id" :title="clip.name" :disabled="store.isEntryMissing(clip.libraryEntryId)" @click="audio.playTrackClip(track.id, clip.id)"><VIcon v-if="active && selected === clip.id" start size="14">mdi-waveform</VIcon>{{ clip.name }}</VBtn>
        <VMenu v-if="track.clips.length > 5" location="bottom">
          <template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" :text="`+${track.clips.length - 5} clips`" size="small" variant="text" /></template>
          <VCard width="360" class="pa-3"><VAutocomplete v-model="extra" :items="track.clips" item-title="name" item-value="id" label="Choose a clip" variant="outlined" hide-details autofocus @update:model-value="(id) => { if (id) audio.playTrackClip(track.id, id); }" /></VCard>
        </VMenu>
      </div>
    </div>
    <section v-if="expanded" class="inline-editor" :aria-label="`Shared settings for ${track.name}`">
      <div class="shared-scope mixer-context"><div><div class="shared-heading"><VIcon size="16">mdi-link-variant</VIcon>Shared track settings</div><p v-if="referencesLoading">Loading scene usage…</p><p v-else-if="referencesFailed" class="text-error">Could not load scene usage.</p><p v-else>Used in {{ references.length }} scene{{ references.length === 1 ? '' : 's' }}. Changes here apply everywhere this track is used.</p></div><VBtn v-if="sceneId" text="Make independent copy for this scene" variant="outlined" size="small" :loading="busy" @click="showDuplicate = true" /></div>
      <div v-if="references.length" class="scene-references"><VChip v-for="reference in references" :key="reference.sceneId" size="x-small" :title="reference.configurationName">{{ reference.boardName }} / {{ reference.sceneName }}</VChip></div>
      <div class="inline-name"><VTextField v-model="draftName" label="Track name (shared)" variant="outlined" density="compact" hide-details @keydown.enter="rename" /><VBtn text="Rename" size="small" variant="tonal" :disabled="!draftName.trim() || draftName.trim() === track.name" @click="rename" /></div>
      <TrackSettings :track="track" />
      <TrackClipList :track="track" :is-seamless="track.mode.type === 'seamless'" @open-library-browser="showLibrary = true" />
      <LibraryBrowser v-model="showLibrary" :track-id="track.id" />
    </section>
    <ConfirmDialog v-model="removeBoard" title="Remove track from board" :message="`Remove “${track.name}” from this board${boardScenes.length ? ` and its scene layouts: ${boardScenes.map((scene) => scene.name).join(', ')}` : ''}? The shared track stays in the inventory and on other boards.`" confirm-label="Remove from board" @confirm="removeFromBoard" />
    <ShareTrackDialog v-model="showShare" :track-id="track.id" :board-id="boardId" />
    <DuplicateTrackDialog v-model="showDuplicate" :track="track" :scene-id="sceneId" />
  </article>
</template>
<style scoped>
.track-card { min-width: 0; padding: 12px 14px; }
.track-heading:focus-visible { outline: 2px solid rgb(var(--v-theme-primary)); border-radius: 3px; }
.inline-editor { border-top: 1px solid rgba(var(--v-theme-on-surface), .12); margin-top: 20px; padding-top: 20px; }
.shared-scope { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 12px; padding: 14px; margin-bottom: 12px; }
.shared-heading { display: flex; align-items: center; gap: 7px; font-size: 13px; font-weight: 500; }
.shared-scope p { font-size: 12px; margin-top: 6px; opacity: .7; }
.scene-references { display: flex; flex-wrap: wrap; gap: 5px; margin-bottom: 18px; }
.inline-name { display: flex; align-items: center; gap: 12px; max-width: 600px; margin: 20px 0; }
.track-head { display: flex; align-items: center; gap: 10px; }
.track-heading { display: block; min-width: 0; flex: 1; text-align: left; color: inherit; background: transparent; border: 0; padding: 4px 0; cursor: pointer; }
.track-title { display: block; font-weight: 500; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.track-mode { display: block; font-size: 12px; color: rgba(var(--v-theme-on-surface), .6); margin-top: 3px; }
.track-volume { display: flex; align-items: center; gap: 8px; flex: 0 0 280px; }
.track-volume :deep(.v-slider) { min-width: 0; flex: 1; }
.track-volume span { flex: 0 0 32px; font-size: 11px; opacity: .6; width: 32px; text-align: right; font-variant-numeric: tabular-nums; }
.track-body { padding: 8px 0 0 68px; }
.clip-choices { display: flex; gap: 6px; flex-wrap: wrap; }
.clip-choices :deep(.v-btn) { text-transform: none; letter-spacing: 0; max-width: 100%; }
.clip-choices :deep(.v-btn__content) { display: block; overflow: hidden; text-overflow: ellipsis; }
.now-playing { display: flex; gap: 6px; align-items: center; font-size: 12px; margin-bottom: 10px; }
.now-playing span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.track-empty { padding-top: 12px; }
@media(max-width: 900px) { .track-head { flex-wrap: wrap; } .track-volume { flex: 1 0 calc(100% - 68px); margin-left: 68px; order: 4; } .track-body { padding-left: 68px; } }
@media(max-width: 600px) { .track-card { padding: 10px; } .track-body { padding-left: 0; } }
</style>
