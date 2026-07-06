<script setup lang="ts">
import { computed } from 'vue';
import type { Track } from '@generated/index';
import { useTrackUpdates } from '../composables/useTrackUpdates';

const props = defineProps<{
  track: Track;
}>();

const {
  updateMode,
  updateTransition,
  updateRepeat,
  updateGapMin,
  updateGapMax,
  toggleFadeIn,
  toggleFadeOut,
} = useTrackUpdates(() => props.track.id);

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

    <VCheckbox
      :model-value="track.fadeIn > 0"
      @update:model-value="toggleFadeIn"
      label="Fade In"
      density="compact"
      hide-details
    />

    <VCheckbox
      :model-value="track.fadeOut > 0"
      @update:model-value="toggleFadeOut"
      label="Fade Out"
      density="compact"
      hide-details
    />

    <VCheckbox
      v-if="hasTransition"
      :model-value="(track.mode as any).repeat"
      @update:model-value="updateRepeat"
      label="Repeat"
      density="compact"
      hide-details
    />
  </div>

</template>
