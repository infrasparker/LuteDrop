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

// --- Waveform commit (single batch from the editor) ---
async function onWaveformCommit(values: {
  trimStart: number | null;
  trimEnd: number | null;
  loopStart: number;
  loopEnd: number;
}): Promise<void> {
  // Update trim first (it may clamp loop points in the DB)
  await store.command('update_library_entry_trim', {
    entryId: props.entryId,
    trimStart: values.trimStart,
    trimEnd: values.trimEnd,
  });
  // Then update loop points with the values from the editor (not from store)
  await store.command('update_library_entry_loop_points', {
    entryId: props.entryId,
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

function parseLoopField(v: string | number): number {
  const n = Number(v);
  if (isNaN(n)) return 0;
  return Math.round(n * 100) / 100;
}

// Numeric inputs commit as a batch too
async function commitNumericValues(): Promise<void> {
  const ts = entry.value?.trimStart ?? null;
  const te = entry.value?.trimEnd ?? null;
  const ls = entry.value?.loopStart ?? 0;
  const le = entry.value?.loopEnd ?? 0;
  await onWaveformCommit({ trimStart: ts, trimEnd: te, loopStart: ls, loopEnd: le });
}

async function updateTrimStartNumeric(v: string | number): Promise<void> {
  await store.command('update_library_entry_trim', {
    entryId: props.entryId,
    trimStart: parseTrimField(v),
    trimEnd: entry.value?.trimEnd ?? null,
  });
}

async function updateTrimEndNumeric(v: string | number): Promise<void> {
  await store.command('update_library_entry_trim', {
    entryId: props.entryId,
    trimStart: entry.value?.trimStart ?? null,
    trimEnd: parseTrimField(v),
  });
}

async function updateLoopStartNumeric(v: string | number): Promise<void> {
  await store.command('update_library_entry_loop_points', {
    entryId: props.entryId,
    loopStart: parseLoopField(v) || null,
    loopEnd: entry.value?.loopEnd ?? null,
  });
}

async function updateLoopEndNumeric(v: string | number): Promise<void> {
  await store.command('update_library_entry_loop_points', {
    entryId: props.entryId,
    loopStart: entry.value?.loopStart ?? null,
    loopEnd: parseLoopField(v) || null,
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
  <VDialog :model-value="modelValue" @update:model-value="emit('update:modelValue', $event)" max-width="1100" persistent>
    <VCard v-if="entry">
      <VCardTitle class="d-flex align-center">
        <span>Edit: {{ entry.name }}</span>
        <VSpacer />
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
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="entry.trimStart ?? ''"
              label="Trim Start (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateTrimStartNumeric((e.target as HTMLInputElement).value)"
            />
          </div>
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="entry.trimEnd ?? ''"
              label="Trim End (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateTrimEndNumeric((e.target as HTMLInputElement).value)"
            />
          </div>
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="entry.loopStart ?? ''"
              label="Loop Start (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateLoopStartNumeric((e.target as HTMLInputElement).value)"
            />
          </div>
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="entry.loopEnd ?? ''"
              label="Loop End (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
              @change="(e: Event) => updateLoopEndNumeric((e.target as HTMLInputElement).value)"
            />
          </div>
        </div>
      </VCardText>
    </VCard>
  </VDialog>
</template>
