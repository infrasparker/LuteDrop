<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import WaveformEditor from './WaveformEditor.vue';

const props = defineProps<{
  modelValue: boolean;
  entryId: string;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const store = useAppStore();
const audio = useAudio();

const entry = computed(() => store.findLibraryEntry(props.entryId));

// Local editable fields
const editName = ref('');
const editTags = ref('');
const localGain = ref(1.0);

const previewState = computed(() => store.playbackStates.get('__preview'));
const isPreviewPlaying = computed(() => store.previewEntryId === props.entryId && previewState.value?.isPlaying);

watch(previewState, (state) => {
  if (!state?.isPlaying && store.previewEntryId === props.entryId) {
    store.previewEntryId = null;
  }
});

function togglePreview(): void {
  if (isPreviewPlaying.value) {
    audio.stopPreview();
    store.previewEntryId = null;
  } else if (entry.value) {
    audio.playPreview(
      entry.value.filePath, 0, localGain.value,
      entry.value.trimStart, entry.value.trimEnd,
      entry.value.loopStart, entry.value.loopEnd,
    );
    store.previewEntryId = props.entryId;
  }
}

watch(() => props.modelValue, (open) => {
  if (open && entry.value) {
    editName.value = entry.value.name;
    editTags.value = entry.value.tags.join(', ');
    localGain.value = entry.value.volume;
  }
});

// Sync gain from store when not dragging
watch(() => entry.value?.volume, (v) => {
  if (v !== undefined) localGain.value = v;
});

// --- Name ---
async function saveName(): Promise<void> {
  if (!entry.value || editName.value === entry.value.name) return;
  await store.command('rename_library_entry', {
    entryId: props.entryId,
    name: editName.value,
  });
}

// --- Tags ---
async function saveTags(): Promise<void> {
  if (!entry.value) return;
  const tags = editTags.value
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  await store.command('update_library_entry_tags', {
    entryId: props.entryId,
    tags,
  });
}

// --- Volume (local ref for smooth slider, persist on change) ---
function setGain(v: number): void {
  localGain.value = v;
}

async function persistGain(): Promise<void> {
  await store.command('update_library_entry_volume', {
    entryId: props.entryId,
    volume: localGain.value,
  });
}

// --- Waveform commit (single atomic command from the editor) ---
async function onWaveformCommit(values: {
  trimStart: number | null;
  trimEnd: number | null;
  loopStart: number;
  loopEnd: number;
}): Promise<void> {
  await store.command('update_library_entry_markers', {
    entryId: props.entryId,
    trimStart: values.trimStart,
    trimEnd: values.trimEnd,
    loopStart: values.loopStart || null,
    loopEnd: values.loopEnd || null,
  });
}

// --- Numeric field helpers ---
function parseTrimField(v: string | number): number | null {
  const n = Number(v);
  if (v === '' || v === null || isNaN(n)) return null;
  return Math.round(n * 100) / 100;
}

// Atomic update from numeric inputs — merges override into current entry values
async function updateMarkersFromNumeric(overrides: {
  trimStart?: number | null;
  trimEnd?: number | null;
  loopStart?: number | null;
  loopEnd?: number | null;
}): Promise<void> {
  if (!entry.value) return;
  await store.command('update_library_entry_markers', {
    entryId: props.entryId,
    trimStart: overrides.trimStart !== undefined ? overrides.trimStart : (entry.value.trimStart ?? null),
    trimEnd: overrides.trimEnd !== undefined ? overrides.trimEnd : (entry.value.trimEnd ?? null),
    loopStart: overrides.loopStart !== undefined ? overrides.loopStart : (entry.value.loopStart ?? null),
    loopEnd: overrides.loopEnd !== undefined ? overrides.loopEnd : (entry.value.loopEnd ?? null),
  });
}

function close(): void {
  // Persist gain on close if changed
  if (entry.value && localGain.value !== entry.value.volume) {
    persistGain();
  }
  emit('update:modelValue', false);
}
</script>

<template>
  <VDialog :model-value="modelValue" @update:model-value="emit('update:modelValue', $event)" max-width="1400" width="95%" persistent>
    <VCard v-if="entry">
      <VCardTitle class="d-flex align-center">
        <span>Edit: {{ entry.name }}</span>
        <VSpacer />
        <VBtn
          :icon="isPreviewPlaying ? 'mdi-stop' : 'mdi-play'"
          variant="tonal"
          size="small"
          class="mr-2"
          @click="togglePreview"
        />
        <VBtn icon="mdi-close" variant="text" size="small" @click="close" />
      </VCardTitle>

      <VCardText>
        <!-- Name -->
        <VTextField
          v-model="editName"
          label="Name"
          variant="outlined"
          density="compact"
          hide-details
          class="mb-3"
          @blur="saveName"
          @keydown.enter="saveName"
        />

        <!-- Tags -->
        <VTextField
          v-model="editTags"
          label="Tags (comma-separated)"
          variant="outlined"
          density="compact"
          hide-details
          class="mb-3"
          hint="e.g. ambient, combat, tavern"
          @blur="saveTags"
          @keydown.enter="saveTags"
        />

        <!-- Volume -->
        <div class="d-flex align-center ga-3 mb-4">
          <span class="text-caption text-medium-emphasis" style="min-width: 50px;">Gain</span>
          <VSlider
            :model-value="localGain"
            @update:model-value="setGain"
            @end="persistGain"
            :min="0"
            :max="2"
            :step="0.01"
            hide-details
            thumb-label
          />
          <span class="text-caption" style="min-width: 40px; text-align: right;">
            {{ localGain.toFixed(2) }}
          </span>
        </div>

        <!-- Waveform -->
        <WaveformEditor
          :file-path="entry.filePath"
          :loop-start="entry.loopStart ?? 0"
          :loop-end="entry.loopEnd ?? 0"
          :trim-start="entry.trimStart ?? null"
          :trim-end="entry.trimEnd ?? null"
          @commit="onWaveformCommit"
        />

        <!-- Numeric inputs for trim/loop -->
        <div class="d-flex flex-wrap ga-3 mt-3">
          <div style="min-width: 140px; max-width: 170px;">
            <VTextField
              :model-value="entry.trimStart ?? ''"
              label="Trim Start (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateMarkersFromNumeric({ trimStart: parseTrimField((e.target as HTMLInputElement).value) })"
            />
          </div>
          <div style="min-width: 140px; max-width: 170px;">
            <VTextField
              :model-value="entry.trimEnd ?? ''"
              label="Trim End (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateMarkersFromNumeric({ trimEnd: parseTrimField((e.target as HTMLInputElement).value) })"
            />
          </div>
          <div style="min-width: 140px; max-width: 170px;">
            <VTextField
              :model-value="entry.loopStart ?? ''"
              label="Loop Start (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateMarkersFromNumeric({ loopStart: parseTrimField((e.target as HTMLInputElement).value) })"
            />
          </div>
          <div style="min-width: 140px; max-width: 170px;">
            <VTextField
              :model-value="entry.loopEnd ?? ''"
              label="Loop End (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateMarkersFromNumeric({ loopEnd: parseTrimField((e.target as HTMLInputElement).value) })"
            />
          </div>
        </div>
      </VCardText>
    </VCard>
  </VDialog>
</template>
