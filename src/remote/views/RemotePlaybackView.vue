<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRemote } from '../composables/useRemote';
import { viewTrackIds } from '../../utils/soundstage';
import RemoteTrackControl from '../components/RemoteTrackControl.vue';
const props = defineProps<{ activeTab: number }>();
const remote = useRemote();
const board = computed(() => remote.boards.value[props.activeTab]);
const sceneByBoard = ref<Record<string,string|null>>({});
const selected = computed(() => { if (!board.value) return null; const value=sceneByBoard.value[board.value.id]; return value === null || board.value.scenes.some((scene) => scene.id === value) ? value : board.value.scenes.find((scene) => scene.id === remote.activeSceneId.value)?.id ?? board.value.scenes[0]?.id ?? null; });
watch(remote.activeSceneId, (id) => { const parent=remote.boards.value.find((item) => item.scenes.some((scene) => scene.id === id)); if (parent && id) sceneByBoard.value[parent.id]=id; });
function loadScene(id:string):void { if(board.value) sceneByBoard.value[board.value.id]=id; remote.activateScene(id); }
const tracks = computed(() => viewTrackIds(board.value, selected.value).flatMap((id) => { const track = remote.tracks.value.find((t) => t.id === id); return track ? [track] : []; }));
</script>
<template>
  <div v-if="board" class="remote-stage">
    <aside class="remote-scenes"><div class="text-caption text-medium-emphasis mb-3">Scenes</div><div class="remote-scene-buttons"><VBtn text="All tracks" prepend-icon="mdi-view-grid-outline" :variant="selected === null ? 'tonal' : 'text'" @click="sceneByBoard[board.id] = null" /><div v-for="scene in board.scenes" :key="scene.id" class="remote-scene mixer-surface" :class="{ 'mixer-active': selected === scene.id }"><VBtn icon="mdi-play" :aria-label="`Play scene ${scene.name}`" :title="`Play scene ${scene.name}`" :color="remote.activeSceneId.value === scene.id ? 'primary' : undefined" variant="text" size="small" @click="loadScene(scene.id)" /><button :aria-label="`View scene ${scene.name}`" :aria-pressed="selected === scene.id" @click="sceneByBoard[board.id] = scene.id">{{ scene.name }}<small v-if="remote.activeSceneId.value === scene.id">Loaded</small></button></div></div></aside>
    <main class="remote-tracks"><RemoteTrackControl v-for="track in tracks" :key="track.id" :track="track" /><p v-if="!tracks.length" class="text-medium-emphasis pa-4">No tracks in this layout yet. Add them in the desktop app.</p></main>
  </div>
  <p v-else class="text-medium-emphasis pa-8">No boards yet.</p>
</template>
<style scoped>
.remote-stage { display: flex; gap: 20px; height: 100%; overflow-y: auto; padding: 20px; }
.remote-scenes { flex: 0 0 200px; }
.remote-scene-buttons { display: flex; flex-direction: column; align-items: stretch; gap: 8px; }
.remote-scene { display: flex; align-items: center; }
.remote-scene button:not(.v-btn) { flex: 1; text-align: left; padding: 12px 8px; background: transparent; border: 0; color: inherit; cursor: pointer; font-size: 14px; overflow-wrap: anywhere; }
.remote-scene small { display: block; opacity: .6; font-size: 11px; margin-top: 3px; }
.remote-scene button:focus-visible { outline: 2px solid rgb(var(--v-theme-primary)); border-radius: 6px; }
.remote-tracks { min-width: 0; flex: 1; }
@media(max-width:760px) { .remote-stage { display: block; padding: 12px; } .remote-scenes { padding-bottom: 16px; } .remote-scene-buttons { flex-direction: row; flex-wrap: wrap; gap: 4px; } }
</style>
