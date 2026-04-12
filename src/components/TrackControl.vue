<script setup lang="ts">
import { computed, ref, watch, nextTick, onMounted, onUnmounted } from 'vue';
import type { Track, TrackMode, TrackTransition } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { open } from '@tauri-apps/plugin-dialog';
import Sortable from 'sortablejs';
import ClipRow from './ClipRow.vue';
import WaveformEditor from './WaveformEditor.vue';

const props = defineProps<{
  track: Track;
  compId: string;
}>();

const store = useAppStore();
const audio = useAudio();

const playbackState = computed(() => store.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);

// --- Track settings ---

const modeOptions = [
  { title: 'Seamless', value: 'seamless' },
  { title: 'Sequence', value: 'sequence' },
  { title: 'Shuffle', value: 'shuffle' },
];

const transitionOptions = [
  { title: 'Gapless', value: 'gapless' },
  { title: 'Gap', value: 'gap' },
  { title: 'Crossfade', value: 'crossfade' },
];

const modeType = computed(() => props.track.mode.type);
const hasTransition = computed(() => modeType.value !== 'seamless');
const transitionType = computed(() =>
  hasTransition.value ? (props.track.mode as any).transition.type : 'gapless'
);

const isSeamless = computed(() => modeType.value === 'seamless');
const hasClips = computed(() => props.track.clips.length > 0);
const seamlessClip = computed(() => props.track.clips[0] ?? null);
const showWaveform = computed(() => isSeamless.value && seamlessClip.value !== null);
const canAddClips = computed(() => !isSeamless.value || !hasClips.value);

async function updateMode(type: string): Promise<void> {
  let mode: TrackMode;
  if (type === 'seamless') {
    mode = { type: 'seamless', loopStart: 0, loopEnd: 0 };
  } else {
    const transition = hasTransition.value
      ? (props.track.mode as any).transition
      : { type: 'gapless' as const };
    mode = { type: type as 'sequence' | 'shuffle', repeat: true, transition };
  }
  await updateField({ mode });
}

async function updateTransition(type: string): Promise<void> {
  let transition: TrackTransition;
  switch (type) {
    case 'gap': transition = { type: 'gap', min: 0, max: 5 }; break;
    case 'crossfade': transition = { type: 'crossfade', duration: 2 }; break;
    default: transition = { type: 'gapless' }; break;
  }
  const mode = props.track.mode as { type: 'sequence' | 'shuffle'; repeat: boolean; transition: TrackTransition };
  await updateField({ mode: { ...mode, transition } });
}

async function updateField(updates: Partial<Track>): Promise<void> {
  await store.command('update_track', {
    compId: props.compId,
    trackId: props.track.id,
    updates,
  });
}

async function updateLoopStart(v: string): Promise<void> {
  const currentMode = props.track.mode as { type: 'seamless'; loopStart: number; loopEnd: number };
  await updateField({ mode: { type: 'seamless', loopStart: Number(v), loopEnd: currentMode.loopEnd } });
}

async function updateLoopEnd(v: string): Promise<void> {
  const currentMode = props.track.mode as { type: 'seamless'; loopStart: number; loopEnd: number };
  await updateField({ mode: { type: 'seamless', loopStart: currentMode.loopStart, loopEnd: Number(v) } });
}

async function updateRepeat(v: boolean): Promise<void> {
  const mode = props.track.mode as any;
  await updateField({ mode: { ...mode, repeat: v } });
}

async function updateGapMin(v: string): Promise<void> {
  const mode = props.track.mode as any;
  await updateField({ mode: { ...mode, transition: { type: 'gap', min: Number(v), max: mode.transition.max } } });
}

async function updateGapMax(v: string): Promise<void> {
  const mode = props.track.mode as any;
  await updateField({ mode: { ...mode, transition: { type: 'gap', min: mode.transition.min, max: Number(v) } } });
}

async function updateCrossfade(v: string): Promise<void> {
  const mode = props.track.mode as any;
  await updateField({ mode: { ...mode, transition: { type: 'crossfade', duration: Number(v) } } });
}

async function updateLoopStartFromWaveform(v: number): Promise<void> {
  const currentMode = props.track.mode as { type: 'seamless'; loopStart: number; loopEnd: number };
  await updateField({ mode: { type: 'seamless', loopStart: v, loopEnd: currentMode.loopEnd } });
}

async function updateLoopEndFromWaveform(v: number): Promise<void> {
  const currentMode = props.track.mode as { type: 'seamless'; loopStart: number; loopEnd: number };
  await updateField({ mode: { type: 'seamless', loopStart: currentMode.loopStart, loopEnd: v } });
}

async function updateFadeIn(v: string): Promise<void> {
  await updateField({ fadeIn: Number(v) });
}

async function updateFadeOut(v: string): Promise<void> {
  await updateField({ fadeOut: Number(v) });
}

async function addClips(): Promise<void> {
  const paths = await open({
    multiple: !isSeamless.value,
    filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'] }],
  });
  if (!paths) return;
  const pathList = Array.isArray(paths) ? paths : [paths];
  for (const filePath of pathList) {
    const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';
    await store.command('add_clip', { trackId: props.track.id, name: fileName, filePath });
  }
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

async function deleteTrack(): Promise<void> {
  await audio.stopTrack(props.track.id);
  await store.command('delete_track', { compId: props.compId, trackId: props.track.id });
}

const clipsContainer = ref<HTMLElement | null>(null);
let sortableInstance: Sortable | null = null;

function initSortable(): void {
  if (sortableInstance) {
    sortableInstance.destroy();
    sortableInstance = null;
  }
  if (!clipsContainer.value) return;
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
      await updateField({ clips });
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
  <div>
    <VExpansionPanels>
      <VExpansionPanel color="surface-light" rounded="0" elevation="0">
        <template #title>
          <div class="d-flex align-center ga-2" style="width: 100%;">
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

            <VTextField
              :model-value="track.name"
              @update:model-value="(v: string) => updateField({ name: v })"
              @click.stop
              @keydown.stop
              variant="plain"
              density="compact"
              hide-details
              class="text-subtitle-2 font-weight-bold"
              style="flex: 1 1 0; min-width: 100px;"
            />

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
      <div class="d-flex flex-wrap ga-3 mb-2 align-center">
        <div style="max-width: 150px; min-width: 120px;">
          <VSelect
            :model-value="modeType"
            @update:model-value="updateMode"
            :items="modeOptions"
            label="Mode"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
        <div v-if="isSeamless" style="max-width: 150px; min-width: 120px;">
          <VTextField
            :model-value="(track.mode as any).loopStart"
            @update:model-value="updateLoopStart"
            label="Loop Start (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
        <div v-if="isSeamless" style="max-width: 150px; min-width: 120px;">
          <VTextField
            :model-value="(track.mode as any).loopEnd"
            @update:model-value="updateLoopEnd"
            label="Loop End (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
        <div v-if="hasTransition" style="max-width: 150px; min-width: 120px;">
          <VSelect
            :model-value="transitionType"
            @update:model-value="updateTransition"
            :items="transitionOptions"
            label="Transition"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
        <div v-if="hasTransition" style="max-width: 150px; min-width: 120px;">
          <VCheckbox
            :model-value="(track.mode as any).repeat"
            @update:model-value="updateRepeat"
            label="Repeat"
            density="compact"
            hide-details
          />
        </div>
      </div>

      <div v-if="hasTransition && transitionType === 'gap'" class="d-flex flex-wrap ga-3 mb-2 align-center">
        <div style="max-width: 150px; min-width: 120px;">
          <VTextField
            :model-value="(track.mode as any).transition.min"
            @update:model-value="updateGapMin"
            label="Gap Min (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
        <div style="max-width: 150px; min-width: 120px;">
          <VTextField
            :model-value="(track.mode as any).transition.max"
            @update:model-value="updateGapMax"
            label="Gap Max (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
      </div>

      <div v-if="hasTransition && transitionType === 'crossfade'" class="d-flex flex-wrap ga-3 mb-2 align-center">
        <div style="max-width: 150px; min-width: 120px;">
          <VTextField
            :model-value="(track.mode as any).transition.duration"
            @update:model-value="updateCrossfade"
            label="Crossfade (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
      </div>

      <div class="d-flex flex-wrap ga-3 mb-3 align-center">
        <div style="max-width: 150px; min-width: 120px;">
          <VTextField
            :model-value="track.fadeIn"
            @update:model-value="updateFadeIn"
            label="Fade In (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
        <div style="max-width: 150px; min-width: 120px;">
          <VTextField
            :model-value="track.fadeOut"
            @update:model-value="updateFadeOut"
            label="Fade Out (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
          />
        </div>
      </div>

      <!-- Waveform: only shown in seamless mode with exactly one clip -->
      <div v-if="showWaveform" class="mb-3">
        <div class="text-caption text-medium-emphasis mb-1">{{ seamlessClip!.name }}</div>
        <WaveformEditor
          :file-path="seamlessClip!.filePath"
          :loop-start="(track.mode as any).loopStart"
          :loop-end="(track.mode as any).loopEnd"
          @update:loop-start="(v: number) => updateLoopStartFromWaveform(v)"
          @update:loop-end="(v: number) => updateLoopEndFromWaveform(v)"
        />
      </div>

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

      <div class="d-flex justify-end mt-3">
        <VBtn
          icon="mdi-delete"
          variant="tonal"
          color="error"
          @click="deleteTrack"
        />
      </div>
        </template>
      </VExpansionPanel>
    </VExpansionPanels>
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
