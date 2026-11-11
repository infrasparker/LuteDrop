<script setup lang="ts">
// Creates a reusable shuffle track from selected audio files, or appends clips to an existing track.
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { AppState } from '@generated/index';
import { playbackLabel, trackLocations, trackSearchText } from '../utils/soundstage';
import { useAppStore } from '../stores/appStore';
import { useToast } from '../composables/useToast';

type PlaylistTarget = 'new' | 'existing';

const props = withDefaults(
  defineProps<{
    modelValue: boolean;
    entryIds: string[];
    target?: PlaylistTarget;
    suggestedName?: string;
  }>(),
  { target: 'new', suggestedName: '' },
);

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
  done: [];
}>();

const store = useAppStore();
const toast = useToast();

const tab = ref<PlaylistTarget>('new');
const name = ref('');
const boardId = ref<string | null>(null);
const trackId = ref<string | null>(null);
const busy = ref(false);
const error = ref<string | null>(null);

const configurationName = computed(
  () => store.configurations.find((c) => c.id === store.activeConfigurationId)?.name ?? 'Current configuration',
);

const boardItems = computed(() => store.boards.map((g) => ({ title: g.name, value: g.id })));

const trackItems = computed(() => store.tracks.map((track) => ({ title: track.name, subtitle: `${playbackLabel(track.mode)} · ${trackLocations(track.id, store.boards).join(' · ') || 'Not on a board in this configuration'}`, searchText: trackSearchText(track, store.boards), value: track.id })));

watch(
  () => props.modelValue,
  (open) => {
    if (!open) return;
    tab.value = props.target;
    name.value = props.suggestedName;
    error.value = null;
    if (!boardId.value || !store.boards.some((g) => g.id === boardId.value)) {
      boardId.value = store.boards[0]?.id ?? null;
    }
    if (trackId.value && !store.findTrack(trackId.value)) trackId.value = null;
  },
  { immediate: true },
);

const canSubmit = computed(() => {
  if (busy.value || props.entryIds.length === 0) return false;
  if (tab.value === 'new') return name.value.trim().length > 0;
  return !!trackId.value;
});

// Runs one hierarchy command and hydrates the store; throws on failure so the
// caller can stop the sequence.
async function run(cmd: string, args: Record<string, unknown>): Promise<AppState> {
  const state = await invoke<AppState>(cmd, args);
  store.hydrate(state);
  return state;
}

async function createPlaylist(): Promise<void> {
  await run('add_track', { boardId: boardId.value, name: name.value.trim(), libraryEntryIds: props.entryIds, mode: 'shuffle' });
  toast.showSuccess(`Created track “${name.value.trim()}” with ${props.entryIds.length} clips.`);
}

async function addToTrack(): Promise<void> {
  const track = store.findTrack(trackId.value!);
  if (!track) throw new Error('That track no longer exists.');
  const existing = new Set(track.clips.map((c) => c.libraryEntryId));
  const toAdd = props.entryIds.filter((id) => !existing.has(id));
  const skipped = props.entryIds.length - toAdd.length;
  if (toAdd.length > 0) {
    await run('add_clips', { trackId: track.id, libraryEntryIds: toAdd });
  }
  const skippedNote = skipped > 0 ? ` (${skipped} already there)` : '';
  toast.showSuccess(`Added ${toAdd.length} clip${toAdd.length === 1 ? '' : 's'} to "${track.name}"${skippedNote}`);
}

async function submit(): Promise<void> {
  if (!canSubmit.value) return;
  busy.value = true;
  error.value = null;
  try {
    if (tab.value === 'new') await createPlaylist();
    else await addToTrack();
    emit('done');
    emit('update:modelValue', false);
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <VDialog :model-value="modelValue" max-width="480" :persistent="busy" @update:model-value="(v) => emit('update:modelValue', v)">
    <VCard>
      <VCardTitle class="pt-4">
        Add {{ entryIds.length }} as clip{{ entryIds.length === 1 ? '' : 's' }}
      </VCardTitle>
      <VTabs v-model="tab" density="compact" color="primary" class="px-2">
        <VTab value="new" prepend-icon="mdi-playlist-plus">New track</VTab>
        <VTab value="existing" prepend-icon="mdi-playlist-music">Existing track</VTab>
      </VTabs>
      <VDivider />
      <VCardText>
        <div class="text-caption text-medium-emphasis mb-3">
          Configuration: <strong>{{ configurationName }}</strong>
        </div>

        <template v-if="tab === 'new'">
          <VSelect v-model="boardId" :items="boardItems" label="Add to board (optional)" clearable variant="outlined" density="compact" class="mb-2" />
          <VTextField v-model="name" label="Track name" variant="outlined" density="compact" autofocus @keydown.enter="submit" />
          <div class="text-caption text-medium-emphasis">Creates a reusable shuffle track; each selection becomes a clip.</div>
        </template>

        <template v-else>
          <div v-if="trackItems.length === 0" class="text-body-2 text-medium-emphasis">
            This configuration has no tracks yet.
          </div>
          <VAutocomplete
            v-else
            v-model="trackId"
            :items="trackItems"
            item-title="title"
            item-value="value"
            label="Track"
            :custom-filter="(value, query, item) => (item?.raw.searchText ?? value).includes(query.trim().normalize('NFKC').toLocaleLowerCase())"
            hint="Already-present clips are skipped"
            persistent-hint
            variant="outlined"
            density="compact"
          >
            <template #item="{ props: itemProps, item }">
              <VListItem v-bind="itemProps" :subtitle="item.subtitle" />
            </template>
          </VAutocomplete>
        </template>

        <VAlert v-if="error" type="error" variant="tonal" density="compact" class="mt-3">
          {{ error }}
        </VAlert>
      </VCardText>
      <VDivider />
      <VCardActions>
        <VSpacer />
        <VBtn text="Cancel" variant="plain" :disabled="busy" @click="emit('update:modelValue', false)" />
        <VBtn
          :text="tab === 'new' ? 'Create track' : 'Add clips'"
          color="primary"
          variant="tonal"
          :loading="busy"
          :disabled="!canSubmit"
          @click="submit"
        />
      </VCardActions>
    </VCard>
  </VDialog>
</template>
