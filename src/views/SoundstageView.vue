<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { useContextMenuState } from '../composables/useContextMenu';
import { useAudio } from '../composables/useAudio';
import { useToast } from '../composables/useToast';
import { useListReorder } from '../composables/useListReorder';
import { isBoardActive } from '../shared/playback';
import { sceneMatches, viewTrackIds } from '../utils/soundstage';
import ConfigurationPicker from '../components/ConfigurationPicker.vue';
import SceneCard from '../components/SceneCard.vue';
import TrackCard from '../components/TrackCard.vue';
import TrackPickerDialog from '../components/TrackPickerDialog.vue';
import PromptDialog from '../components/PromptDialog.vue';
import ConfirmDialog from '../components/ConfirmDialog.vue';
const store = useAppStore();
const ui = useSoundstageUiStore();
const audio = useAudio();
const board = computed(() => store.boards.find((item) => item.id === ui.boardId) ?? store.boards[0]);
// null selects the board's freeform All tracks workspace; scene IDs select layouts.
const selectedSceneId = computed(() => {
  if (!board.value) return null;
  const remembered = ui.sceneByBoard[board.value.id];
  if (remembered === null || board.value.scenes.some((scene) => scene.id === remembered)) return remembered;
  return board.value.scenes.find((scene) => scene.id === store.activeSceneId)?.id ?? board.value.scenes[0]?.id ?? null;
});
const viewedScene = computed(() => board.value?.scenes.find((scene) => scene.id === selectedSceneId.value));
const visibleIds = computed(() => viewTrackIds(board.value, selectedSceneId.value));
const tracks = computed(() => visibleIds.value.flatMap((id) => { const track = store.findTrack(id); return track ? [track] : []; }));
const activeScene = computed(() => store.activeSceneId ? store.findScene(store.activeSceneId) : undefined);
const adjusted = computed(() => !!board.value && !!viewedScene.value && store.activeSceneId === viewedScene.value.id && !sceneMatches(viewedScene.value.tracks, store.captureScene(board.value.id, viewedScene.value.id)));
const elsewhere = computed(() => store.tracks.filter((track) => !visibleIds.value.includes(track.id) && (store.playbackStates.get(track.id)?.isPlaying || store.playbackStates.get(track.id)?.isPaused)));
watch(() => store.activeSceneId, (id) => {
  if (!id) return;
  const parent = store.boards.find((item) => item.scenes.some((scene) => scene.id === id));
  if (parent) ui.selectScene(parent.id, id);
});
const showAddTrack = ref(false);
const showNewBoard = ref(false);
const showNewScene = ref(false);
const showEmptyScene = ref(false);
const showRenameBoard = ref(false);
const showDeleteBoard = ref(false);
const busy = ref(false);
const selectedBoard = ref('');
const selectedBoardName = ref('');
const { menuVisible, menuX, menuY, openMenu } = useContextMenuState();
const scenesContainer = ref<HTMLElement | null>(null);
const tracksContainer = ref<HTMLElement | null>(null);
useListReorder(scenesContainer, {
  item: '.scene-item', handle: '.scene-drag-handle',
  context: () => board.value?.id ?? '',
  ids: () => board.value?.scenes.map((scene) => scene.id) ?? [],
  move: (sceneId, newIndex) => store.command('reorder_scene', { sceneId, newIndex }),
});
useListReorder(tracksContainer, {
  item: '.track-card', handle: '.track-drag-handle',
  context: () => `${board.value?.id}:${selectedSceneId.value ?? 'all'}`,
  ids: () => visibleIds.value,
  move: (trackId, newIndex) => store.command(viewedScene.value ? 'reorder_scene_track' : 'reorder_track', {
    boardId: board.value?.id, sceneId: viewedScene.value?.id, trackId, newIndex,
  }),
});
async function createBoard(name: string): Promise<void> {
  const before = new Set(store.boards.map((item) => item.id));
  if (await store.command('add_board', { name })) ui.boardId = store.boards.find((item) => !before.has(item.id))?.id ?? null;
}
async function saveAsScene(name: string): Promise<void> {
  if (!board.value || busy.value) return;
  busy.value = true;
  try {
    const before = new Set(board.value.scenes.map((scene) => scene.id));
    const boardId = board.value.id;
    if (await store.command('add_scene', { boardId, name, tracks: store.captureScene(boardId, selectedSceneId.value) })) {
      const created = store.boards.find((item) => item.id === boardId)?.scenes.find((scene) => !before.has(scene.id));
      if (created) ui.selectScene(boardId, created.id);
      useToast().showInfo('Scene saved from the current mix.');
    }
  } finally { busy.value = false; }
}
async function createEmptyScene(name: string): Promise<void> {
  if (!board.value || busy.value) return;
  busy.value = true;
  try {
    const before = new Set(board.value.scenes.map((scene) => scene.id));
    const boardId = board.value.id;
    if (await store.command('add_scene', { boardId, name, tracks: [], markActive: false })) {
      const created = store.boards.find((item) => item.id === boardId)?.scenes.find((scene) => !before.has(scene.id));
      if (created) ui.selectScene(boardId, created.id);
    }
  } finally { busy.value = false; }
}
async function saveScene(): Promise<void> {
  if (!viewedScene.value || !board.value || busy.value) return;
  busy.value = true;
  try { if (await store.command('save_scene', { sceneId: viewedScene.value.id, tracks: store.captureScene(board.value.id, viewedScene.value.id) })) useToast().showInfo('Scene changes saved.'); }
  finally { busy.value = false; }
}
</script>
<template>
  <div class="soundstage">
    <Teleport to="#app-bar-main">
      <ConfigurationPicker /><VDivider vertical class="mx-2 my-2" />
      <VTabs :model-value="board?.id" color="primary" density="compact" show-arrows class="board-tabs">
        <VTab v-for="item in store.boards" :key="item.id" :value="item.id" :class="{ 'board-playing': isBoardActive(item, store.playbackStates) }" size="small" @click="ui.boardId = item.id" @contextmenu.prevent="selectedBoard = item.id; selectedBoardName = item.name; openMenu($event)">{{ item.name }}</VTab>
      </VTabs>
      <VBtn icon="mdi-plus" aria-label="New board" title="New board" variant="text" size="x-small" class="ml-1" @click="showNewBoard = true" />
    </Teleport>
    <template v-if="board">
      <aside class="scenes-pane">
        <button class="all-tracks mixer-surface" :class="{ 'mixer-active': selectedSceneId === null }" :aria-pressed="selectedSceneId === null" @click="ui.selectScene(board.id, null)"><VIcon size="18">mdi-view-grid-outline</VIcon><span>All tracks<small>Freeform workspace</small></span></button>
        <div class="pane-heading">Scenes</div>
        <div ref="scenesContainer" :key="board.id" class="scenes-list"><SceneCard v-for="(scene, index) in board.scenes" :key="scene.id" :scene="scene" :board-id="board.id" :index="index" :count="board.scenes.length" :selected="selectedSceneId === scene.id" /><p v-if="!board.scenes.length" class="empty-scenes">Build a mix in All tracks, or create an empty scene.</p></div>
        <VBtn text="New scene" prepend-icon="mdi-plus" variant="tonal" rounded="xl" block :disabled="busy" @click="showEmptyScene = true" />
        <VBtn text="Save mix as scene" variant="text" size="small" class="mt-2" :disabled="busy" @click="showNewScene = true" />
      </aside>
      <main class="board-workspace">
        <div class="workspace-heading">
          <div class="workspace-title"><h1>{{ viewedScene?.name ?? board.name }}</h1><p v-if="viewedScene">{{ board.name }} · {{ store.activeSceneId === viewedScene.id ? 'Loaded scene' : 'Viewing cards' }}<span v-if="adjusted" class="text-primary"> · Adjusted</span></p><p v-else>All tracks · Freeform mix<span v-if="activeScene"> · {{ activeScene.name }} loaded</span></p></div>
          <div class="workspace-actions"><VBtn v-if="viewedScene && store.activeSceneId !== viewedScene.id" text="Load scene" prepend-icon="mdi-play" variant="tonal" color="primary" size="small" @click="audio.activateScene(viewedScene.id)" /><VBtn v-if="adjusted" text="Save scene changes" variant="tonal" color="primary" size="small" :loading="busy" @click="saveScene" /><VBtn text="Add track" prepend-icon="mdi-plus" variant="tonal" rounded="xl" size="small" @click="showAddTrack = true" /></div>
        </div>
        <div v-if="elsewhere.length" class="elsewhere"><span>Also playing</span><VChip v-for="track in elsewhere" :key="track.id" size="small" closable @click:close="audio.stopTrack(track.id)">{{ track.name }}</VChip></div>
        <div ref="tracksContainer" :key="`${board.id}:${selectedSceneId ?? 'all'}`" class="track-list"><TrackCard v-for="(track, index) in tracks" :key="track.id" :track="track" :board-id="board.id" :scene-id="viewedScene?.id" :index="index" :count="tracks.length" /></div>
        <div v-if="!tracks.length" class="board-empty"><VIcon size="36" class="mb-3">mdi-tune-vertical</VIcon><h2>Add the sounds you want within reach</h2><p>Reuse a track from your inventory, or create one from audio files in the Library.</p><VBtn text="Add track" prepend-icon="mdi-plus" color="primary" variant="tonal" class="mt-4" @click="showAddTrack = true" /></div>
      </main>
      <TrackPickerDialog v-model="showAddTrack" :board-id="board.id" :scene-id="viewedScene?.id" />
    </template>
    <div v-else class="board-empty"><h1>Your Soundstage</h1><p>Create a board for a setting such as City, Wilderness, or Battle.</p><VBtn text="Create board" prepend-icon="mdi-plus" variant="tonal" color="primary" class="mt-4" @click="showNewBoard = true" /></div>
    <PromptDialog v-model="showNewBoard" title="New board" label="Board name" confirm-label="Create" @submit="createBoard" />
    <PromptDialog v-model="showEmptyScene" title="New scene" label="Scene name" confirm-label="Create scene" @submit="createEmptyScene" />
    <PromptDialog v-model="showNewScene" title="Save mix as scene" label="Scene name" confirm-label="Save scene" @submit="saveAsScene" />
    <VMenu v-model="menuVisible" :target="[menuX, menuY]"><VList density="compact"><VListItem prepend-icon="mdi-pencil-outline" title="Rename board" @click="showRenameBoard = true" /><VListItem prepend-icon="mdi-delete-outline" title="Delete board" class="text-error" @click="showDeleteBoard = true" /></VList></VMenu>
    <PromptDialog v-model="showRenameBoard" title="Rename board" label="Board name" :initial="selectedBoardName" confirm-label="Rename" @submit="(name) => store.command('rename_board', { boardId: selectedBoard, name })" />
    <ConfirmDialog v-model="showDeleteBoard" title="Delete board" :message="`Delete “${selectedBoardName}” and its saved scenes? Shared tracks stay in the inventory.`" confirm-label="Delete board" confirm-color="error" @confirm="store.command('delete_board', { boardId: selectedBoard })" />
  </div>
</template>
<style scoped>
.soundstage { position: fixed; inset: var(--v-layout-top) var(--v-layout-right) var(--v-layout-bottom) var(--v-layout-left); display: flex; overflow: hidden; }
.board-tabs { min-width: 0; height: 100%; }
.board-playing { background: var(--mixer-active-fill); }
.scenes-pane { width: 235px; flex: 0 0 235px; display: flex; flex-direction: column; border-right: 1px solid rgba(var(--v-theme-on-surface), .1); padding: 20px 12px 14px; }
.all-tracks { display: flex; align-items: center; gap: 10px; width: 100%; padding: 12px 10px; margin-bottom: 20px; text-align: left; color: inherit; cursor: pointer; }
.all-tracks span { font-size: 14px; }
.all-tracks small { display: block; margin-top: 4px; font-size: 12px; opacity: .55; }
.all-tracks:focus-visible { outline: 2px solid rgb(var(--v-theme-primary)); }
.pane-heading { padding: 0 10px 14px; font-size: 12px; color: rgba(var(--v-theme-on-surface), .6); }
.scenes-list { flex: 1; min-height: 0; overflow-y: auto; margin-bottom: 14px; }
.scenes-pane > :deep(.v-btn) { flex: 0 0 auto; height: 36px; min-height: 36px; }
.empty-scenes { padding: 10px; font-size: 13px; opacity: .65; }
.board-workspace { flex: 1; min-width: 0; padding: 24px; overflow-y: auto; }
.workspace-heading { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 20px; }
.workspace-title { min-width: 0; }
.workspace-title h1 { font-size: 21px; font-weight: 500; margin: 0; overflow-wrap: anywhere; }
.workspace-title p { font-size: 12px; opacity: .65; margin: 5px 0 0; }
.workspace-actions { display: flex; flex-wrap: wrap; gap: 8px; flex: 0 0 auto; }
.track-list { display: flex; flex-direction: column; gap: 10px; }
.board-empty { margin: auto; padding: 48px 24px; text-align: center; max-width: 620px; }
.board-empty h1,.board-empty h2 { font-size: 20px; font-weight: 500; }
.board-empty p { font-size: 14px; opacity: .65; margin-top: 10px; }
.elsewhere { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 12px; margin-bottom: 16px; }
@media(max-width: 1000px) { .scenes-pane { width: 195px; flex-basis: 195px; } .board-workspace { padding: 18px; } .workspace-heading { align-items: flex-start; flex-direction: column; } }

</style>

<style>
.stage-drag-placeholder { opacity: .25; background: rgba(var(--v-theme-primary), .1); }
.stage-drag-preview { opacity: .94 !important; color: #e8e4dc; background: #141418; box-shadow: 0 12px 30px #0008; pointer-events: none; }
</style>
