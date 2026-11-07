<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import WaveformEditor from './WaveformEditor.vue';
import TagChip from './TagChip.vue';
import TagScoresPanel from './TagScoresPanel.vue';
import RoleControl from './RoleControl.vue';
import MoodProfile from './MoodProfile.vue';
import { open } from '@tauri-apps/plugin-dialog';
import { AUDIO_EXTENSIONS } from '../constants';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';
import { useLocalVolume } from '../shared/useLocalVolume';

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
// Auto tag names currently shown; sent with each save so only auto tags
// removed in this dialog are rejected.
const seenAutoTags = ref<string[]>([]);
const editEntryType = ref('music');
const { local: localGain } = useLocalVolume(() => entry.value?.volume);

const previewState = computed(() => store.playbackStates.get(LIBRARY_PLAYBACK_ID));
const isPreviewPlaying = computed(() => store.libraryPlayingEntryId === props.entryId && previewState.value?.isPlaying);

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
    editEntryType.value = entry.value.entryType;
    localGain.value = entry.value.volume;
  }
}, { immediate: true });

// Tag edits save immediately, so the store is the source of truth: keep the
// combobox and the seen auto tags in step with it (this also picks up Keep /
// Remove on an auto chip and auto-tag runs finishing while the dialog is open).
watch(
  () => [entry.value?.tags.join('|'), entry.value?.autoTags.map((a) => a.tag).join('|')],
  () => {
    if (!entry.value) return;
    editTags.value = [...entry.value.tags];
    seenAutoTags.value = entry.value.autoTags.map((a) => a.tag);
  },
  { immediate: true },
);

// --- Name ---
async function saveName(): Promise<void> {
  if (!entry.value || editName.value === entry.value.name) return;
  await store.command('rename_library_entry', {
    entryId: props.entryId,
    name: editName.value,
  });
}

// --- Tags ---
function removeEditTag(index: number): void {
  editTags.value = editTags.value.filter((_, i) => i !== index);
  saveTags();
}

async function saveTags(): Promise<void> {
  if (!entry.value) return;
  const tags = editTags.value
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  await store.command('update_library_entry_tags', {
    entryId: props.entryId,
    tags,
    seenAutoTags: seenAutoTags.value,
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
    // The editor uses 0 as its "no loop" sentinel on loopEnd; the DB uses
    // null. A loop exists only when loopEnd is set — loopStart may
    // legitimately be 0.0 (the old `loopStart || null` erased that case).
    loopStart: values.loopEnd === 0 ? null : values.loopStart,
    loopEnd: values.loopEnd === 0 ? null : values.loopEnd,
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

// Find similar: Scry seeded with this entry. Only analyzed music has a sound
// fingerprint to compare.
const router = useRouter();
const canFindSimilar = computed(
  () => entry.value?.entryType === 'music' && entry.value.analysis?.status === 'embedded',
);

function findSimilar(): void {
  const id = props.entryId;
  close();
  router.push({ name: 'scry', query: { seed: id } });
}

function close(): void {
  // Persist gain on close if changed
  if (entry.value && localGain.value !== entry.value.volume) {
    persistGain();
  }
  emit('update:modelValue', false);
}

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
        <!-- The span carries the hover text, since a disabled button gets no hover. -->
        <span
          class="mr-2"
          :title="canFindSimilar
            ? 'Find similar in Scry'
            : entry.entryType === 'music'
              ? 'Find similar: not analyzed yet (run Analyze library in Auto-tagging)'
              : 'Find similar: Scry compares music only'"
        >
          <VBtn
            icon="mdi-compass-outline"
            variant="tonal"
            size="small"
            aria-label="Find similar in Scry"
            :disabled="!canFindSimilar"
            @click="findSimilar"
          />
        </span>
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
        >
          <template #chip="{ index }">
            <TagChip
              :tag="editTags[index]"
              :auto="entry?.autoTags.find((a) => a.tag === editTags[index]) ?? null"
              :entry-id="props.entryId"
              closable
              @remove="removeEditTag(index)"
            />
          </template>
        </VCombobox>

        <!-- Role and the mood profile (music only). Role: label | options | status grid -->
        <template v-if="editEntryType === 'music'">
          <div class="facet-grid mb-3">
            <span class="text-caption text-medium-emphasis">Role</span>
            <RoleControl
              :entry-ids="[props.entryId]"
              :role-auto="entry.roleAuto"
              :role-auto-score="entry.roleAutoScore"
              :role-user="entry.roleUser"
            />
          </div>
          <div class="text-caption text-medium-emphasis mb-1">Moods</div>
          <MoodProfile :key="entry.id" :entries="[entry]" class="mb-4" />
        </template>

        <!-- Volume -->
        <div class="d-flex align-center ga-3 mb-4">
          <span class="text-caption text-medium-emphasis" style="min-width: 90px;">Gain</span>
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

        <TagScoresPanel :entry-id="entryId" :is-ambiance="editEntryType === 'ambiance'" />
      </VCardText>
    </VCard>
  </VDialog>
</template>

<style scoped>
/* Label | options | status. Each facet control renders its options and status
   as two cells (display: contents), so the columns line up across rows. */
.facet-grid {
  display: grid;
  /* The options take one line when the dialog has room and wrap (down to
     their widest button) when it has not; the status never wraps. */
  grid-template-columns: max-content minmax(min-content, max-content) minmax(max-content, 1fr);
  align-items: center;
  column-gap: 12px;
  row-gap: 12px;
}
</style>
