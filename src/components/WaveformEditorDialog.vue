<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import WaveformEditor from './WaveformEditor.vue';

const props = defineProps<{
  modelValue: boolean;
  entryId: string;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const store = useAppStore();

const entry = computed(() => store.findLibraryEntry(props.entryId));

// Local editable fields
const editName = ref('');
const editTags = ref('');

watch(() => props.modelValue, (open) => {
  if (open && entry.value) {
    editName.value = entry.value.name;
    editTags.value = entry.value.tags.join(', ');
  }
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

// --- Volume ---
const entryVolume = computed(() => entry.value?.volume ?? 1);

async function updateVolume(v: number): Promise<void> {
  await store.command('update_library_entry_volume', {
    entryId: props.entryId,
    volume: v,
  });
}

// --- Trim ---
const trimStart = computed(() => entry.value?.trimStart ?? null);
const trimEnd = computed(() => entry.value?.trimEnd ?? null);

async function updateTrimStart(v: number | null): Promise<void> {
  await store.command('update_library_entry_trim', {
    entryId: props.entryId,
    trimStart: v,
    trimEnd: trimEnd.value,
  });
}

async function updateTrimEnd(v: number | null): Promise<void> {
  await store.command('update_library_entry_trim', {
    entryId: props.entryId,
    trimStart: trimStart.value,
    trimEnd: v,
  });
}

// --- Loop ---
const loopStart = computed(() => entry.value?.loopStart ?? 0);
const loopEnd = computed(() => entry.value?.loopEnd ?? 0);

async function updateLoopStart(v: number): Promise<void> {
  await store.command('update_library_entry_loop_points', {
    entryId: props.entryId,
    loopStart: v,
    loopEnd: loopEnd.value,
  });
}

async function updateLoopEnd(v: number): Promise<void> {
  await store.command('update_library_entry_loop_points', {
    entryId: props.entryId,
    loopStart: loopStart.value,
    loopEnd: v,
  });
}

// --- Numeric field helpers ---
function parseTrimField(v: string | number, fallback: number | null): number | null {
  const n = Number(v);
  if (v === '' || v === null || isNaN(n)) return fallback;
  return Math.round(n * 100) / 100;
}

function parseLoopField(v: string | number): number {
  const n = Number(v);
  if (isNaN(n)) return 0;
  return Math.round(n * 100) / 100;
}

function close(): void {
  emit('update:modelValue', false);
}
</script>

<template>
  <VDialog :model-value="modelValue" @update:model-value="emit('update:modelValue', $event)" max-width="700" persistent>
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
            :model-value="entryVolume"
            @update:model-value="updateVolume"
            :min="0"
            :max="2"
            :step="0.01"
            hide-details
            thumb-label
          />
          <span class="text-caption" style="min-width: 40px; text-align: right;">
            {{ entryVolume.toFixed(2) }}
          </span>
        </div>

        <!-- Waveform -->
        <WaveformEditor
          :file-path="entry.filePath"
          :loop-start="loopStart"
          :loop-end="loopEnd"
          :trim-start="trimStart"
          :trim-end="trimEnd"
          @update:loop-start="updateLoopStart"
          @update:loop-end="updateLoopEnd"
          @update:trim-start="updateTrimStart"
          @update:trim-end="updateTrimEnd"
        />

        <!-- Numeric inputs for trim/loop -->
        <div class="d-flex flex-wrap ga-3 mt-3">
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="trimStart ?? ''"
              @update:model-value="(v: string | number) => updateTrimStart(parseTrimField(v, null))"
              label="Trim Start (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
            />
          </div>
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="trimEnd ?? ''"
              @update:model-value="(v: string | number) => updateTrimEnd(parseTrimField(v, null))"
              label="Trim End (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
            />
          </div>
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="loopStart"
              @update:model-value="(v: string | number) => updateLoopStart(parseLoopField(v))"
              label="Loop Start (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
            />
          </div>
          <div style="min-width: 110px; max-width: 130px;">
            <VTextField
              :model-value="loopEnd"
              @update:model-value="(v: string | number) => updateLoopEnd(parseLoopField(v))"
              label="Loop End (s)"
              type="number"
              variant="outlined"
              density="compact"
              hide-details
            />
          </div>
        </div>
      </VCardText>
    </VCard>
  </VDialog>
</template>
