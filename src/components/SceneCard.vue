<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Scene } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { useAudio } from '../composables/useAudio';
import ConfirmDialog from './ConfirmDialog.vue';
import PromptDialog from './PromptDialog.vue';
import SceneEditorDialog from './SceneEditorDialog.vue';
import ReorderGrip from './ReorderGrip.vue';
const props = defineProps<{ scene: Scene; boardId: string; index: number; count: number; selected?: boolean }>();
const store = useAppStore();
const audio = useAudio();
const ui = useSoundstageUiStore();
const loaded = computed(() => store.activeSceneId === props.scene.id);
const rename = ref(false);
const remove = ref(false);
const edit = ref(false);
const transfer = ref(false);
const transferMode = ref<'copy' | 'move'>('copy');
const destination = ref<string | null>(null);
const copyName = ref('');
const transferBoards = computed(() => transferMode.value === 'copy' ? store.boards : otherBoards.value);
const otherBoards = computed(() => store.boards.filter((board) => board.id !== props.boardId));
const busy = ref(false);
const summary = computed(() => props.scene.tracks.filter((state) => state.enabled).map((state) => store.findTrack(state.trackId)?.name).filter(Boolean).join(' · ') || 'All tracks off');
async function applyTransfer(): Promise<void> {
  if (!destination.value || (transferMode.value === 'copy' && !copyName.value.trim()) || busy.value) return;
  busy.value = true;
  try { if (await store.command(`${transferMode.value}_scene`, { sceneId: props.scene.id, destinationBoardId: destination.value, ...(transferMode.value === 'copy' ? { name: copyName.value.trim() } : {}) })) transfer.value = false; }
  finally { busy.value = false; }
}
function openTransfer(mode: 'copy' | 'move'): void { transferMode.value = mode; destination.value = mode === 'copy' ? props.boardId : null; copyName.value = `${props.scene.name} (copy)`; transfer.value = true; }
function play(): void { ui.selectScene(props.boardId, props.scene.id); void audio.activateScene(props.scene.id); }
</script>
<template>
  <div class="scene-item mixer-surface" :data-reorder-id="scene.id" :class="{ 'mixer-active': selected }">
    <ReorderGrip class="scene-drag-handle" :label="`Drag scene ${scene.name} to reorder`" />
    <VBtn icon="mdi-play" :aria-label="`Play scene ${scene.name}`" :title="`Play scene ${scene.name}`" :color="loaded ? 'primary' : undefined" variant="text" size="x-small" class="scene-play" @click="play" />
    <button class="scene-load" :aria-label="`View scene ${scene.name}`" :aria-pressed="!!selected" @click="ui.selectScene(boardId, scene.id)"><span class="scene-text"><span class="scene-name" :title="scene.name">{{ scene.name }}</span><span class="scene-summary" :title="summary">{{ loaded ? 'Loaded' : summary }}</span></span></button>
    <VMenu location="bottom end"><template #activator="{ props: menuProps }"><VBtn v-bind="menuProps" icon="mdi-dots-vertical" size="x-small" variant="text" :aria-label="`Actions for scene ${scene.name}`" /></template>
      <VList density="compact" min-width="220">
        <VListItem prepend-icon="mdi-playlist-plus" title="Add scene to mix" subtitle="Keep other tracks playing" @click="audio.activateScene(scene.id, true)" />
        <VDivider class="my-1" />
        <VListItem prepend-icon="mdi-tune" title="Scene setup…" @click="edit = true" />
        <VListItem prepend-icon="mdi-pencil-outline" title="Rename" @click="rename = true" />
        <VListItem prepend-icon="mdi-content-duplicate" title="Duplicate scene…" @click="openTransfer('copy')" />
        <VListItem prepend-icon="mdi-folder-move-outline" title="Move to board…" :disabled="!otherBoards.length" @click="openTransfer('move')" />
        <VDivider class="my-1" />
        <VListItem prepend-icon="mdi-arrow-up" title="Move up" :disabled="index === 0" @click="store.command('reorder_scene', { sceneId: scene.id, newIndex: index - 1 })" />
        <VListItem prepend-icon="mdi-arrow-down" title="Move down" :disabled="index === count - 1" @click="store.command('reorder_scene', { sceneId: scene.id, newIndex: index + 1 })" />
        <VDivider class="my-1" /><VListItem prepend-icon="mdi-delete-outline" title="Delete scene" class="text-error" @click="remove = true" />
      </VList>
    </VMenu>
    <SceneEditorDialog v-model="edit" :scene="scene" :board-id="boardId" />
    <PromptDialog v-model="rename" title="Rename scene" label="Scene name" :initial="scene.name" confirm-label="Rename" @submit="(name) => store.command('rename_scene', { sceneId: scene.id, name })" />
    <ConfirmDialog v-model="remove" title="Delete scene" :message="`Delete “${scene.name}”? Its shared tracks stay in the inventory and on this board.`" confirm-label="Delete scene" confirm-color="error" @confirm="store.command('delete_scene', { sceneId: scene.id })" />
    <VDialog v-model="transfer" max-width="440" :persistent="busy"><VCard :title="transferMode === 'copy' ? 'Duplicate scene' : 'Move scene to board'"><VCardText><VTextField v-if="transferMode === 'copy'" v-model="copyName" label="Scene name" variant="outlined" hide-details class="mb-4" :disabled="busy" /><VSelect v-model="destination" :disabled="busy" :items="transferBoards" item-title="name" item-value="id" label="Board" variant="outlined" hide-details /><p class="text-body-2 text-medium-emphasis mt-4">Track references come along. Shared track definitions stay the same.</p></VCardText><VCardActions><VSpacer /><VBtn text="Cancel" :disabled="busy" @click="transfer = false" /><VBtn :text="transferMode === 'copy' ? 'Duplicate scene' : 'Move scene'" :disabled="!destination || (transferMode === 'copy' && !copyName.trim())" :loading="busy" color="primary" @click="applyTransfer" /></VCardActions></VCard></VDialog>
  </div>
</template>
<style scoped>
.scene-item { display: flex; align-items: center; padding: 0 3px; margin-bottom: 6px; }
.scene-play { flex: 0 0 auto; }
.scene-load { display: flex; align-items: flex-start; padding: 13px 5px; min-width: 0; flex: 1; text-align: left; color: inherit; background: transparent; border: 0; cursor: pointer; }
.scene-load:focus-visible { outline: 2px solid rgb(var(--v-theme-primary)); border-radius: 6px; }
.scene-text { display: block; min-width: 0; }
.scene-name { display: block; font-size: 14px; font-weight: 500; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.scene-summary { display: block; margin-top: 4px; font-size: 12px; color: rgba(var(--v-theme-on-surface), .6); overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
</style>
