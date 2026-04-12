<script setup lang="ts">
import { computed } from 'vue';
import type { Track } from '@generated/index';
import { useTrackUpdates } from '../composables/useTrackUpdates';
import WaveformEditor from './WaveformEditor.vue';

const props = defineProps<{
  track: Track;
  compId: string;
}>();

const {
  updateMode,
  updateTransition,
  updateLoopStart,
  updateLoopEnd,
  updateLoopStartFromWaveform,
  updateLoopEndFromWaveform,
  updateRepeat,
  updateGapMin,
  updateGapMax,
  updateCrossfade,
  updateFadeIn,
  updateFadeOut,
} = useTrackUpdates(() => props.track.id, () => props.compId);

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
const isSeamless = computed(() => modeType.value === 'seamless');
const hasTransition = computed(() => modeType.value !== 'seamless');
const transitionType = computed(() =>
  hasTransition.value ? (props.track.mode as any).transition.type : 'gapless'
);

const seamlessClip = computed(() => props.track.clips[0] ?? null);
const showWaveform = computed(() => isSeamless.value && seamlessClip.value !== null);
</script>

<template>
  <!-- Order: Mode, Transition, Fade In, Fade Out, Loop Start, Loop End, Repeat, Gap/Crossfade -->
  <div class="d-flex flex-wrap ga-3 mb-3 align-center">
    <div style="min-width: 130px; max-width: 150px;">
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

    <div v-if="hasTransition" style="min-width: 130px; max-width: 150px;">
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

    <div style="min-width: 90px; max-width: 110px;">
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

    <div style="min-width: 90px; max-width: 110px;">
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

    <div v-if="isSeamless" style="min-width: 100px; max-width: 120px;">
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

    <div v-if="isSeamless" style="min-width: 100px; max-width: 120px;">
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

    <template v-if="hasTransition && transitionType === 'gap'">
      <div style="min-width: 90px; max-width: 110px;">
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
      <div style="min-width: 90px; max-width: 110px;">
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
    </template>

    <div v-if="hasTransition && transitionType === 'crossfade'" style="min-width: 100px; max-width: 120px;">
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

    <VCheckbox
      v-if="hasTransition"
      :model-value="(track.mode as any).repeat"
      @update:model-value="updateRepeat"
      label="Repeat"
      density="compact"
      hide-details
    />
  </div>

  <!-- Waveform: only shown in seamless mode with exactly one clip -->
  <div v-if="showWaveform" class="mb-3">
    <div class="text-caption text-medium-emphasis mb-1">{{ seamlessClip!.name }}</div>
    <WaveformEditor
      :file-path="seamlessClip!.filePath"
      :loop-start="(track.mode as any).loopStart"
      :loop-end="(track.mode as any).loopEnd"
      :track-id="track.id"
      @update:loop-start="(v: number) => updateLoopStartFromWaveform(v)"
      @update:loop-end="(v: number) => updateLoopEndFromWaveform(v)"
    />
  </div>
</template>
