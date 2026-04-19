<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import { useLibraryImport } from '../composables/useLibraryImport';

const props = defineProps<{
  modelValue: boolean;
  paths: string[];
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
  import: [payload: { paths: string[]; tags: string[]; entryType: string }];
  cancel: [];
}>();

const store = useAppStore();
const { tags: existingTags } = storeToRefs(store);
const { pickFiles, pickFolder } = useLibraryImport();

const stagedPaths = ref<string[]>([]);
const selectedTags = ref<string[]>([]);
const selectedEntryType = ref<string>('music');
const isAdding = ref(false);

watch(
  () => props.modelValue,
  (open) => {
    if (open) {
      stagedPaths.value = [...props.paths];
      selectedTags.value = [];
      selectedEntryType.value = 'music';
    }
  },
);

// Keep stagedPaths in sync if the parent seeds more paths while the dialog is
// already open (e.g. user opened Import > Files, then reopens Import > Folder).
watch(
  () => props.paths,
  (next) => {
    if (props.modelValue) {
      mergePaths(next);
    }
  },
);

const fileNames = computed(() =>
  stagedPaths.value.map((p) => p.split(/[/\\]/).pop() ?? p),
);

function mergePaths(more: string[]): void {
  if (more.length === 0) return;
  const set = new Set(stagedPaths.value);
  for (const p of more) set.add(p);
  stagedPaths.value = [...set];
}

async function addFiles(): Promise<void> {
  isAdding.value = true;
  try {
    mergePaths(await pickFiles());
  } finally {
    isAdding.value = false;
  }
}

async function addFolder(): Promise<void> {
  isAdding.value = true;
  try {
    mergePaths(await pickFolder());
  } finally {
    isAdding.value = false;
  }
}

function removePath(idx: number): void {
  stagedPaths.value.splice(idx, 1);
}

function onCancel(): void {
  emit('update:modelValue', false);
  emit('cancel');
}

function onImport(): void {
  const normalized = selectedTags.value
    .map((t) => t.trim().toLowerCase())
    .filter((t) => t.length > 0);
  const deduped = Array.from(new Set(normalized));
  const paths = [...stagedPaths.value];
  emit('update:modelValue', false);
  emit('import', { paths, tags: deduped, entryType: selectedEntryType.value });
}
</script>

<template>
  <VDialog :model-value="modelValue" max-width="520" @update:model-value="(v) => emit('update:modelValue', v)">
    <VCard :title="`Import ${stagedPaths.length} ${stagedPaths.length === 1 ? 'file' : 'files'}`">
      <VCardText>
        <VSelect
          v-model="selectedEntryType"
          :items="[{ title: 'Music', value: 'music' }, { title: 'Ambiance', value: 'ambiance' }]"
          label="Type"
          variant="outlined"
          density="comfortable"
          hide-details
          class="mb-3"
        />
        <VCombobox
          v-model="selectedTags"
          :items="existingTags"
          label="Tags (optional)"
          hint="Applied to every imported file. Leave empty to skip tagging."
          persistent-hint
          multiple
          chips
          closable-chips
          clearable
          variant="outlined"
          density="comfortable"
          class="mb-3"
        />
        <div class="d-flex align-center justify-space-between mb-1">
          <div class="text-caption text-medium-emphasis">Files</div>
          <div class="d-flex ga-1">
            <VBtn
              size="x-small"
              variant="tonal"
              prepend-icon="mdi-file-music"
              :loading="isAdding"
              @click="addFiles"
            >
              Add files
            </VBtn>
            <VBtn
              size="x-small"
              variant="tonal"
              prepend-icon="mdi-folder-music"
              :loading="isAdding"
              @click="addFolder"
            >
              Add folder
            </VBtn>
          </div>
        </div>
        <VList density="compact" class="import-file-list pa-0" max-height="220">
          <VListItem
            v-for="(name, idx) in fileNames"
            :key="stagedPaths[idx]"
            :title="name"
            density="compact"
          >
            <template #prepend>
              <VIcon size="small">mdi-file-music-outline</VIcon>
            </template>
            <template #append>
              <VBtn
                icon="mdi-close"
                size="x-small"
                variant="text"
                title="Remove from import"
                @click="removePath(idx)"
              />
            </template>
          </VListItem>
          <VListItem v-if="stagedPaths.length === 0" density="compact" class="text-medium-emphasis">
            <VListItemTitle class="text-caption">
              No files staged. Use "Add files" or "Add folder" to stage more.
            </VListItemTitle>
          </VListItem>
        </VList>
      </VCardText>
      <VDivider />
      <VCardActions>
        <VSpacer />
        <VBtn text="Cancel" variant="plain" @click="onCancel" />
        <VBtn
          text="Import"
          color="primary"
          variant="tonal"
          :disabled="stagedPaths.length === 0"
          @click="onImport"
        />
      </VCardActions>
    </VCard>
  </VDialog>
</template>

<style scoped>
.import-file-list {
  overflow-y: auto;
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 4px;
}
</style>
