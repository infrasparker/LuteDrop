<script setup lang="ts">
import { computed, ref } from 'vue';
import type { Track } from '@generated/index';
import { useRemote } from '../composables/useRemote';
import { useLocalVolume } from '../../shared/useLocalVolume';
import { playbackLabel } from '../../utils/soundstage';
const props = defineProps<{ track: Track }>();
const remote = useRemote();
const expanded = ref(false);
const playback = computed(() => remote.playbackStates.get(props.track.id));
const active = computed(() => !!(playback.value?.isPlaying || playback.value?.isPaused));
const liveMix = computed(() => remote.mix.value.find((state) => state.trackId === props.track.id));
const selected = computed(() => playback.value?.currentClipId ?? liveMix.value?.selectedClipId ?? props.track.clips[0]?.id);
const { local: volume, set: setVolume } = useLocalVolume(() => liveMix.value?.volume ?? props.track.volume, (value) => remote.setTrackVolume(props.track.id, value));
</script>
<template>
  <article class="remote-track mixer-surface" :class="{ 'mixer-active': active }">
    <div class="d-flex align-center ga-3"><VBtn :icon="active ? 'mdi-stop' : 'mdi-play'" :aria-label="`${active ? 'Stop' : 'Play'} ${track.name}`" :color="active ? 'primary' : undefined" variant="tonal" size="small" :disabled="!track.clips.length" @click="active ? remote.stopTrack(track.id) : remote.playTrack(track.id)" /><div class="track-name"><strong>{{ track.name }}</strong><div class="text-caption text-medium-emphasis">{{ playbackLabel(track.mode) }}</div></div><span class="text-caption">{{ Math.round(volume * 100) }}%</span></div>
    <VSlider :model-value="volume" :aria-label="`${track.name} volume`" @update:model-value="setVolume" :min="0" :max="1" :step=".01" hide-details density="compact" class="my-3" />
    <div class="clip-choices"><VBtn v-for="clip in track.clips.slice(0, expanded ? undefined : 5)" :key="clip.id" :text="clip.name" :title="clip.name" :aria-label="`Play ${clip.name} on ${track.name}`" variant="outlined" class="mixer-choice" :class="{ 'mixer-choice-active': active && selected === clip.id }" :color="active && selected === clip.id ? 'primary' : undefined" size="small" :disabled="remote.isEntryMissing(clip.libraryEntryId)" @click="remote.playTrackClip(track.id, clip.id)" /><VBtn v-if="track.clips.length > 5" :text="expanded ? 'Show fewer' : `+${track.clips.length - 5} clips`" variant="text" size="small" @click="expanded = !expanded" /></div>
  </article>
</template>
<style scoped>
.remote-track { padding: 16px; margin-bottom: 12px; }
.track-name { flex: 1; min-width: 0; overflow-wrap: anywhere; }
.track-name strong { font-weight: 500; font-size: 14px; }
.clip-choices { display: flex; flex-wrap: wrap; gap: 6px; }
.clip-choices :deep(.v-btn) { text-transform: none; max-width: 100%; letter-spacing: 0; min-height: 36px; }
.clip-choices :deep(.v-btn__content) { overflow: hidden; display: block; text-overflow: ellipsis; }
</style>
