<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import WaveformEditor from './WaveformEditor.vue';
import { open } from '@tauri-apps/plugin-dialog';

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

const loopEnabled = computed(() => {
  return entry.value?.loopStart !== null || entry.value?.loopEnd !== null;
});

async function toggleLoop(): Promise<void> {
  if (!entry.value) return;
  if (loopEnabled.value) {
    // Turn off: clear loop points
    await store.command('update_library_entry_markers', {
      entryId: props.entryId,
      trimStart: entry.value.trimStart ?? null,
      trimEnd: entry.value.trimEnd ?? null,
      loopStart: null,
      loopEnd: null,
    });
  } else {
    // Turn on: set loop points to trim bounds (or file bounds)
    // Use explicit numbers so the waveform draws markers at the right positions
    const ls = entry.value.trimStart ?? 0;
    const le = entry.value.trimEnd ?? (entry.value.duration ?? 0);
    await store.command('update_library_entry_markers', {
      entryId: props.entryId,
      trimStart: entry.value.trimStart ?? null,
      trimEnd: entry.value.trimEnd ?? null,
      loopStart: ls,
      loopEnd: le,
    });
  }
}

// Local editable fields
const editName = ref('');
const editTags = ref<string[]>([]);
const editEntryType = ref('music');
const localGain = ref(1.0);

const previewState = computed(() => store.playbackStates.get('__library'));
const isPreviewPlaying = computed(() => store.libraryPlayingEntryId === props.entryId && previewState.value?.isPlaying);

watch(previewState, (state) => {
  if (!state?.isPlaying && store.libraryPlayingEntryId === props.entryId) {
    store.libraryPlayingEntryId = null;
  }
});

function togglePreview(): void {
  if (isPreviewPlaying.value) {
    audio.stopLibrarySound();
  } else if (entry.value) {
    audio.playLibrarySound(
      props.entryId,
      entry.value.filePath,
      entry.value.trimStart ?? 0,
      localGain.value,
      entry.value.trimStart, entry.value.trimEnd,
      entry.value.loopStart, entry.value.loopEnd,
    );
  }
}

watch(() => props.modelValue, (open) => {
  if (open && entry.value) {
    editName.value = entry.value.name;
    editTags.value = [...entry.value.tags];
    editEntryType.value = entry.value.entryType;
    localGain.value = entry.value.volume;
  }
}, { immediate: true });

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
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  await store.command('update_library_entry_tags', {
    entryId: props.entryId,
    tags,
  });
}

// --- Type ---
async function saveEntryType(v: string): Promise<void> {
  editEntryType.value = v;
  if (!entry.value || v === entry.value.entryType) return;
  await store.command('update_library_entry_type', {
    entryId: props.entryId,
    entryType: v,
  });
}

// --- Volume (local ref for smooth slider, persist on change) ---
function setGain(v: number): void {
  localGain.value = v;
  if (isPreviewPlaying.value) {
    audio.setLibraryVolume(v);
  }
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

const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];
const isMissing = computed(() => store.isEntryMissing(props.entryId));

async function relocateFile(): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
  });
  if (!path || Array.isArray(path)) return;
  await store.command('relocate_library_entry', { entryId: props.entryId, newFilePath: path });
}
</script>

<template>
  <VDialog :model-value="modelValue" @update:model-value="emit('update:modelValue', $event)" max-width="1400" width="95%">
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

      <!-- File path -->
      <div class="d-flex align-center ga-2 px-6 py-2" :style="isMissing ? 'background: rgba(var(--v-theme-warning), 0.08)' : ''">
        <VIcon v-if="isMissing" color="warning" size="small">mdi-alert</VIcon>
        <span
          class="text-caption text-medium-emphasis flex-grow-1 text-truncate"
          :class="isMissing ? 'text-warning' : ''"
          style="direction: rtl; text-align: left;"
          :title="entry.filePath"
        >{{ entry.filePath }}</span>
        <VBtn
          icon="mdi-folder-search"
          size="x-small"
          :variant="isMissing ? 'outlined' : 'text'"
          :color="isMissing ? 'warning' : undefined"
          title="Change file path"
          @click="relocateFile"
        />
      </div>

      <VCardText>
        <!-- Name + Type row -->
        <div class="d-flex ga-3 mb-3">
          <VTextField
            v-model="editName"
            label="Name"
            variant="outlined"
            density="compact"
            hide-details
            class="flex-grow-1"
            @blur="saveName"
            @keydown.enter="saveName"
          />
          <VSelect
            :model-value="editEntryType"
            @update:model-value="saveEntryType"
            :items="[{ title: 'Music', value: 'music' }, { title: 'Ambiance', value: 'ambiance' }]"
            label="Type"
            variant="outlined"
            density="compact"
            hide-details
            style="max-width: 160px;"
          />
        </div>

        <!-- Tags -->
        <VCombobox
          v-model="editTags"
          :items="store.tags"
          label="Tags"
          variant="outlined"
          density="compact"
          hide-details
          multiple
          chips
          closable-chips
          class="mb-3"
          @update:model-value="saveTags"
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
          <VBtn icon="mdi-restore" variant="text" size="x-small" title="Reset to 1.0" @click="setGain(1.0); persistGain();" />
        </div>

        <!-- Loop toggle -->
        <div class="d-flex align-center ga-3 mb-3">
          <VSwitch
            :model-value="loopEnabled"
            @update:model-value="toggleLoop"
            label="Loop"
            hide-details
            density="compact"
            color="primary"
          />
        </div>

        <!-- Waveform -->
        <div v-if="isMissing" class="d-flex flex-column align-center justify-center text-medium-emphasis pa-8" style="min-height: 200px; border: 1px dashed rgba(var(--v-theme-warning), 0.4); border-radius: 8px;">
          <VIcon size="48" color="warning" class="mb-3">mdi-file-alert-outline</VIcon>
          <span class="text-body-2 mb-2">Audio file not found</span>
          <VBtn
            prepend-icon="mdi-folder-search"
            variant="outlined"
            color="warning"
            size="small"
            @click="relocateFile"
          >
            Locate File
          </VBtn>
        </div>
        <WaveformEditor
          v-else
          :key="entry.filePath"
          :file-path="entry.filePath"
          :entry-id="entryId"
          :loop-start="entry.loopStart ?? 0"
          :loop-end="entry.loopEnd ?? 0"
          :trim-start="entry.trimStart ?? null"
          :trim-end="entry.trimEnd ?? null"
          :loop-enabled="loopEnabled"
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
              clearable
              @change="(e: Event) => updateMarkersFromNumeric({ trimStart: parseTrimField((e.target as HTMLInputElement).value) })"
              @click:clear="updateMarkersFromNumeric({ trimStart: null })"
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
              clearable
              @change="(e: Event) => updateMarkersFromNumeric({ trimEnd: parseTrimField((e.target as HTMLInputElement).value) })"
              @click:clear="updateMarkersFromNumeric({ trimEnd: null })"
            />
          </div>
          <template v-if="loopEnabled">
            <div style="min-width: 140px; max-width: 170px;">
              <VTextField
                :model-value="entry.loopStart ?? ''"
                label="Loop Start (s)"
                type="number"
                variant="outlined"
                density="compact"
                hide-details
                clearable
                @change="(e: Event) => updateMarkersFromNumeric({ loopStart: parseTrimField((e.target as HTMLInputElement).value) })"
                @click:clear="updateMarkersFromNumeric({ loopStart: null })"
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
                clearable
                @change="(e: Event) => updateMarkersFromNumeric({ loopEnd: parseTrimField((e.target as HTMLInputElement).value) })"
                @click:clear="updateMarkersFromNumeric({ loopEnd: null })"
              />
            </div>
          </template>
        </div>
      </VCardText>
    </VCard>
  </VDialog>
</template>
