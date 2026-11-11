<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import type { Scene, SceneTrack } from '@generated/index';
import { useAppStore } from '../stores/appStore';
const open = defineModel<boolean>({ default: false });
const props = defineProps<{ scene: Scene; boardId: string }>();
const store = useAppStore();
const draft = ref<(SceneTrack & { included: boolean })[]>([]);
const busy = ref(false);
const boardTracks = computed(() => store.boards.find((board) => board.id === props.boardId)?.trackIds ?? []);
watch(open, (value) => {
  if (!value) return;
  draft.value = boardTracks.value.map((id) => {
    const state = props.scene.tracks.find((track) => track.trackId === id);
    return { ...(state ?? { trackId: id, enabled: false, volume: store.trackMix(id).volume, selectedClipId: null }), included: !!state };
  });
});
async function save(): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  try {
    const tracks = draft.value.filter((state) => state.included).map(({ trackId, enabled, volume, selectedClipId }) => ({ trackId, enabled, volume, selectedClipId }));
    if (await store.command('save_scene', { sceneId: props.scene.id, tracks })) open.value = false;
  } finally { busy.value = false; }
}
</script>
<template>
  <VDialog v-model="open" max-width="760" scrollable :persistent="busy">
    <VCard :title="`Edit scene · ${scene.name}`" rounded="lg">
      <VCardSubtitle style="white-space: normal">Choose this scene’s cards and starting mix. Unchecked cards stay in All tracks; Play controls whether an included card starts enabled.</VCardSubtitle>
      <VCardText>
        <div v-for="state in draft" :key="state.trackId" class="scene-track-row">
          <div class="d-flex align-center ga-2"><VCheckbox v-model="state.included" :label="store.findTrack(state.trackId)?.name" hide-details density="compact" /><VSpacer /><VSwitch v-model="state.enabled" :disabled="!state.included || !store.findTrack(state.trackId)?.clips.length" label="Play" color="primary" hide-details density="compact" inset /></div>
          <div v-if="state.included" class="scene-track-settings">
            <VSelect v-model="state.selectedClipId" :items="[{ title: store.findTrack(state.trackId)?.mode.type === 'shuffle' ? 'Shuffle on load' : 'Use track playback order', value: null }, ...(store.findTrack(state.trackId)?.clips.map((clip) => ({ title: clip.name, value: clip.id })) ?? [])]" label="Starting clip" variant="outlined" density="compact" hide-details />
            <div class="d-flex align-center ga-3"><VSlider v-model="state.volume" :aria-label="`${store.findTrack(state.trackId)?.name} scene volume`" :min="0" :max="1" :step=".01" hide-details /><span class="text-caption">{{ Math.round(state.volume * 100) }}%</span></div>
          </div>
        </div>
        <p v-if="!draft.length" class="text-medium-emphasis">Add tracks to this board first.</p>
      </VCardText>
      <VDivider /><VCardActions class="px-5 py-3"><VSpacer /><VBtn text="Cancel" :disabled="busy" @click="open = false" /><VBtn text="Save scene" color="primary" variant="tonal" :loading="busy" @click="save" /></VCardActions>
    </VCard>
  </VDialog>
</template>
<style scoped>
.scene-track-row { border-bottom: 1px solid rgba(var(--v-theme-on-surface), .12); padding: 10px 0; }
.scene-track-settings { display: grid; grid-template-columns: minmax(0,1fr) 180px; align-items: center; gap: 20px; padding: 6px 0 12px 32px; }
@media(max-width:600px) { .scene-track-settings { grid-template-columns: 1fr; gap: 10px; } }
</style>
