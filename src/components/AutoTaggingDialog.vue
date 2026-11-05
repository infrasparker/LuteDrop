<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/appStore';
import { useToast } from '../composables/useToast';
import { useAutoTagEvents } from '../composables/useAutoTagEvents';
import type { AutoTagReport, AutoTagSettings, SceneConfig, TagQuality } from '@generated/index';

const props = defineProps<{
  modelValue: boolean;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const store = useAppStore();
const toast = useToast();
const { progress, completedRuns } = useAutoTagEvents();

// Examples needed per side before a tag can be trusted.
const MIN_EXAMPLES = 5;

// --- State ---

const settings = ref<AutoTagSettings>({
  enabled: false,
  analyzerUrl: '',
  token: '',
  localPrefix: '',
  remotePrefix: '',
});
const scene = ref<SceneConfig | null>(null);
const vocabulary = ref<string[]>([]);
const report = ref<AutoTagReport | null>(null);

const loading = ref(false);
const saving = ref(false);
const savingVocabulary = ref(false);
const testing = ref(false);
const starting = ref(false);
// Result of the last connection test: the analyzer's status line, or the error text.
const testResult = ref<{ ok: boolean; text: string } | null>(null);

// Bumped when the dialog closes so a slow load cannot overwrite a newer one.
let requestId = 0;

// --- Loading ---

async function loadAll(): Promise<void> {
  const id = ++requestId;
  loading.value = true;
  try {
    const [s, sc, r] = await Promise.all([
      invoke<AutoTagSettings>('get_autotag_settings'),
      invoke<SceneConfig>('get_scene_config'),
      invoke<AutoTagReport>('get_autotag_report'),
    ]);
    if (id !== requestId) return;
    settings.value = s;
    scene.value = sc;
    vocabulary.value = [...sc.vocabulary];
    report.value = r;
  } catch (e) {
    if (id !== requestId) return;
    toast.showError(`Could not load auto-tagging settings: ${e}`);
  } finally {
    if (id === requestId) loading.value = false;
  }
}

async function loadReport(): Promise<void> {
  try {
    report.value = await invoke<AutoTagReport>('get_autotag_report');
  } catch (e) {
    toast.showError(`Could not load the auto-tagging report: ${e}`);
  }
}

watch(
  () => props.modelValue,
  (isOpen) => {
    if (!isOpen) {
      requestId++;
      return;
    }
    testResult.value = null;
    void loadAll();
  },
  { immediate: true },
);

// A run that finishes while the dialog is open refreshes the summary.
watch(completedRuns, () => {
  if (props.modelValue) void loadReport();
});

// --- Settings ---

async function testConnection(): Promise<void> {
  testing.value = true;
  testResult.value = null;
  try {
    const status = await invoke<string>('test_autotag_connection', { settings: settings.value });
    testResult.value = { ok: true, text: status };
  } catch (e) {
    testResult.value = { ok: false, text: String(e) };
  } finally {
    testing.value = false;
  }
}

async function saveSettings(): Promise<void> {
  saving.value = true;
  try {
    await invoke('save_autotag_settings', { settings: settings.value });
    toast.showSuccess('Auto-tagging settings saved');
  } catch (e) {
    toast.showError(`Could not save auto-tagging settings: ${e}`);
  } finally {
    saving.value = false;
  }
}

// --- Analyze ---

// A run is in progress from the first progress event until the complete event.
const running = computed(() => progress.value !== null);
const progressPercent = computed(() => {
  const p = progress.value;
  if (!p || p.total <= 0) return 0;
  return Math.min(100, (p.processed / p.total) * 100);
});
const phaseLabel = computed(() => {
  switch (progress.value?.phase) {
    case 'embedding':
      return 'Analyzing audio';
    case 'scoring':
      return 'Scoring tags';
    case 'error':
      return 'Error';
    default:
      return 'Working';
  }
});

async function analyze(): Promise<void> {
  starting.value = true;
  try {
    await invoke('analyze_library');
  } catch (e) {
    toast.showError(`Could not start analysis: ${e}`);
  } finally {
    starting.value = false;
  }
}

// --- Last run ---

// `finishedAt` is milliseconds since the Unix epoch, sent as a string.
const finishedLabel = computed(() => {
  const at = report.value?.finishedAt;
  if (!at) return null;
  const d = new Date(Number(at));
  return Number.isNaN(d.getTime()) ? at : d.toLocaleString();
});

const summary = computed(() => {
  const r = report.value;
  if (!r) return [];
  return [
    { label: 'Analyzed', value: r.analyzed },
    { label: 'Tags added', value: r.added },
    { label: 'Tags removed', value: r.removed },
    { label: 'Failed', value: r.failed },
    { label: 'Unreachable', value: r.unreachable },
  ];
});

// --- Quality table ---

// Gate first, then each vocabulary tag.
const qualityRows = computed<TagQuality[]>(() => {
  const r = report.value;
  if (!r) return [];
  return [...(r.gate ? [r.gate] : []), ...r.tags];
});

function reason(q: TagQuality): string {
  if (q.trusted) return '';
  if (q.pos < MIN_EXAMPLES || q.neg < MIN_EXAMPLES) {
    const needed = Math.max(MIN_EXAMPLES - q.pos, MIN_EXAMPLES - q.neg);
    return `needs ${needed} more examples`;
  }
  return 'not distinctive enough yet';
}

// --- Vocabulary ---

const vocabularyChanged = computed(
  () => !!scene.value && vocabulary.value.join('|') !== scene.value.vocabulary.join('|'),
);

function cleanVocabulary(tags: string[]): string[] {
  return [...new Set(tags.map((t) => t.trim()).filter((t) => t.length > 0))];
}

async function saveVocabulary(): Promise<void> {
  if (!scene.value) return;
  const config: SceneConfig = { ...scene.value, vocabulary: cleanVocabulary(vocabulary.value) };
  savingVocabulary.value = true;
  try {
    await invoke('save_scene_config', { config });
    scene.value = config;
    vocabulary.value = [...config.vocabulary];
    toast.showSuccess('Vocabulary saved');
  } catch (e) {
    toast.showError(`Could not save the vocabulary: ${e}`);
  } finally {
    savingVocabulary.value = false;
  }
}

function close(): void {
  emit('update:modelValue', false);
}
</script>

<template>
  <VDialog :model-value="modelValue" max-width="760" scrollable @update:model-value="(v) => emit('update:modelValue', v)">
    <VCard title="Auto-tagging" prepend-icon="mdi-creation">
      <VCardText>
        <div v-if="loading" class="d-flex justify-center pa-8">
          <VProgressCircular indeterminate />
        </div>

        <template v-else>
          <!-- Connection settings -->
          <VSwitch
            v-model="settings.enabled"
            label="Enable auto-tagging"
            color="primary"
            density="compact"
            hide-details
            class="mb-2"
          />
          <VTextField
            v-model="settings.analyzerUrl"
            label="Analyzer URL"
            placeholder="http://analyzer.example:8765"
            variant="outlined"
            density="compact"
            hide-details
            class="mb-3"
          />
          <VTextField
            v-model="settings.token"
            label="Token"
            type="password"
            variant="outlined"
            density="compact"
            hide-details
            autocomplete="off"
            class="mb-3"
          />
          <div class="d-flex ga-3 mb-1">
            <VTextField
              v-model="settings.localPrefix"
              label="Local path prefix"
              placeholder="D:\Music"
              variant="outlined"
              density="compact"
              hide-details
            />
            <VTextField
              v-model="settings.remotePrefix"
              label="Analyzer path prefix"
              placeholder="/media/audio"
              variant="outlined"
              density="compact"
              hide-details
            />
          </div>
          <div class="text-caption text-medium-emphasis mb-3">
            File paths start with the local prefix on this machine; the analyzer sees the same folder under its own prefix.
          </div>
          <div class="d-flex align-center ga-3 mb-2">
            <VBtn
              variant="tonal"
              size="small"
              prepend-icon="mdi-lan-connect"
              :loading="testing"
              :disabled="!settings.analyzerUrl.trim()"
              @click="testConnection"
            >
              Test connection
            </VBtn>
            <span
              v-if="testResult"
              class="text-caption"
              :class="testResult.ok ? 'text-success' : 'text-error'"
            >{{ testResult.text }}</span>
            <VSpacer />
            <VBtn color="primary" variant="tonal" size="small" :loading="saving" @click="saveSettings">
              Save
            </VBtn>
          </div>

          <VDivider class="my-4" />

          <!-- Analyze + last run -->
          <div class="d-flex align-center ga-3 mb-2">
            <div class="text-subtitle-2">Library analysis</div>
            <VSpacer />
            <VBtn
              variant="tonal"
              size="small"
              prepend-icon="mdi-waveform"
              :loading="starting"
              :disabled="running || !settings.enabled"
              :title="settings.enabled ? undefined : 'Enable and save auto-tagging first'"
              @click="analyze"
            >
              Analyze library
            </VBtn>
          </div>
          <div v-if="progress" class="mb-3">
            <div class="d-flex text-caption text-medium-emphasis mb-1">
              <span>{{ phaseLabel }}</span>
              <VSpacer />
              <span v-if="progress.total > 0">{{ progress.processed }} / {{ progress.total }}</span>
            </div>
            <VProgressLinear
              :model-value="progressPercent"
              :indeterminate="progress.total <= 0"
              color="primary"
              height="6"
              rounded
            />
            <div v-if="progress.message" class="text-caption text-medium-emphasis mt-1">{{ progress.message }}</div>
          </div>
          <div v-if="report && finishedLabel" class="mb-2">
            <div class="text-caption text-medium-emphasis mb-1">Last run finished {{ finishedLabel }}</div>
            <div class="d-flex flex-wrap ga-2">
              <VChip v-for="item in summary" :key="item.label" size="small" label>
                {{ item.label }}: {{ item.value }}
              </VChip>
            </div>
            <VAlert v-if="report.error" type="error" variant="tonal" density="compact" class="mt-2">
              {{ report.error }}
            </VAlert>
            <div v-if="report.sampleErrors.length > 0" class="mt-2">
              <div class="text-caption text-medium-emphasis">Examples of entries that could not be analyzed:</div>
              <ul class="sample-errors text-caption">
                <li v-for="(line, i) in report.sampleErrors" :key="i">{{ line }}</li>
              </ul>
            </div>
          </div>
          <div v-else class="text-caption text-medium-emphasis mb-2">No run yet.</div>

          <VDivider class="my-4" />

          <!-- Quality table -->
          <div class="text-subtitle-2 mb-2">Tag quality</div>
          <VTable v-if="qualityRows.length > 0" density="compact" class="quality-table">
            <thead>
              <tr>
                <th>Tag</th>
                <th class="text-right">Pos</th>
                <th class="text-right">Neg</th>
                <th class="text-right">AUC</th>
                <th>Trusted</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(q, i) in qualityRows" :key="q.tag">
                <td>
                  {{ q.tag }}
                  <VChip v-if="i === 0 && report?.gate" size="x-small" class="ml-1">gate</VChip>
                </td>
                <td class="text-right">{{ q.pos }}</td>
                <td class="text-right">{{ q.neg }}</td>
                <td class="text-right">{{ q.auc === null ? '-' : q.auc.toFixed(2) }}</td>
                <td>
                  <VIcon v-if="q.trusted" color="success" size="small">mdi-check</VIcon>
                  <span v-else class="text-caption text-medium-emphasis">{{ reason(q) }}</span>
                </td>
              </tr>
            </tbody>
          </VTable>
          <div v-else class="text-caption text-medium-emphasis">
            Tag quality appears after the first analysis.
          </div>

          <VDivider class="my-4" />

          <!-- Vocabulary -->
          <div class="text-subtitle-2 mb-1">Vocabulary</div>
          <div class="text-caption text-medium-emphasis mb-2">
            Tags that can be applied automatically to music entries.
          </div>
          <div class="d-flex align-start ga-3">
            <VCombobox
              v-model="vocabulary"
              :items="store.tags"
              label="Vocabulary tags"
              variant="outlined"
              density="compact"
              hide-details
              multiple
              chips
              closable-chips
              :disabled="!scene"
            />
            <VBtn
              variant="tonal"
              size="small"
              class="mt-1"
              :disabled="!vocabularyChanged"
              :loading="savingVocabulary"
              @click="saveVocabulary"
            >
              Save vocabulary
            </VBtn>
          </div>
        </template>
      </VCardText>
      <VDivider />
      <VCardActions>
        <VSpacer />
        <VBtn text="Close" variant="plain" @click="close" />
      </VCardActions>
    </VCard>
  </VDialog>
</template>

<style scoped>
.sample-errors {
  padding-left: 20px;
  word-break: break-all;
}

.quality-table {
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 4px;
}
</style>
