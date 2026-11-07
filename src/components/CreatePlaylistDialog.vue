<script setup lang="ts">
// Turns a set of library entries into clips: either a new comp (one shuffle
// track holding every entry) in a group of the active configuration, or
// clips appended to an existing track.
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { AppState, TrackMode } from '@generated/index';
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

// New tracks are created as sequence; this switches them to shuffle with the
// app's default transition (gapless, as `insert_track` uses).
const SHUFFLE_MODE: TrackMode = { type: 'shuffle', repeat: true, transition: { type: 'gapless' } };

const tab = ref<PlaylistTarget>('new');
const name = ref('');
const groupId = ref<string | null>(null);
const trackId = ref<string | null>(null);
const busy = ref(false);
const error = ref<string | null>(null);

const configurationName = computed(
  () => store.configurations.find((c) => c.id === store.activeConfigurationId)?.name ?? 'Current configuration',
);

const groupItems = computed(() => store.groups.map((g) => ({ title: g.name, value: g.id })));

const trackItems = computed(() =>
  store.groups.flatMap((g) =>
    g.comps.flatMap((c) =>
      c.tracks.map((t) => ({
        title: t.name,
        subtitle: `${g.name} › ${c.name}`,
        value: t.id,
      })),
    ),
  ),
);

watch(
  () => props.modelValue,
  (open) => {
    if (!open) return;
    tab.value = props.target;
    name.value = props.suggestedName;
    error.value = null;
    if (!groupId.value || !store.groups.some((g) => g.id === groupId.value)) {
      groupId.value = store.groups[0]?.id ?? null;
    }
    if (trackId.value && !store.findTrack(trackId.value)) trackId.value = null;
  },
  { immediate: true },
);

const canSubmit = computed(() => {
  if (busy.value || props.entryIds.length === 0) return false;
  if (tab.value === 'new') return !!groupId.value && name.value.trim().length > 0;
  return !!trackId.value;
});

// Runs one hierarchy command and hydrates the store; throws on failure so the
// caller can stop the sequence.
async function run(cmd: string, args: Record<string, unknown>): Promise<AppState> {
  const state = await invoke<AppState>(cmd, args);
  store.hydrate(state);
  return state;
}

function compIdsIn(state: AppState, gid: string): Set<string> {
  return new Set(state.groups.find((g) => g.id === gid)?.comps.map((c) => c.id) ?? []);
}

async function createPlaylist(): Promise<void> {
  const gid = groupId.value!;
  const playlistName = name.value.trim();
  const before = new Set(store.groups.find((g) => g.id === gid)?.comps.map((c) => c.id) ?? []);

  const afterComp = await run('add_comp', { groupId: gid, name: playlistName });
  const compId = [...compIdsIn(afterComp, gid)].find((id) => !before.has(id));
  if (!compId) throw new Error('The new comp was not found after creating it.');

  try {
    const afterTrack = await run('add_track', { compId, name: playlistName });
    const comp = afterTrack.groups.flatMap((g) => g.comps).find((c) => c.id === compId);
    const newTrackId = comp?.tracks[comp.tracks.length - 1]?.id;
    if (!newTrackId) throw new Error('The new track was not found after creating it.');
    await run('update_track', { trackId: newTrackId, updates: { mode: SHUFFLE_MODE } });
    await run('add_clips', { trackId: newTrackId, libraryEntryIds: props.entryIds });
  } catch (e) {
    // Remove the half-built comp so a failure leaves nothing behind.
    try {
      await run('delete_comp', { compId });
    } catch {
      // The original error is the one worth reporting.
    }
    throw e;
  }

  const groupName = store.groups.find((g) => g.id === gid)?.name ?? '';
  toast.showSuccess(`Created comp "${playlistName}" in ${groupName} with ${props.entryIds.length} clip${props.entryIds.length === 1 ? '' : 's'}`);
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
        <VTab value="new" prepend-icon="mdi-playlist-plus">New comp</VTab>
        <VTab value="existing" prepend-icon="mdi-playlist-music">Existing track</VTab>
      </VTabs>
      <VDivider />
      <VCardText>
        <div class="text-caption text-medium-emphasis mb-3">
          Configuration: <strong>{{ configurationName }}</strong>
        </div>

        <template v-if="tab === 'new'">
          <div v-if="groupItems.length === 0" class="text-body-2 text-medium-emphasis">
            This configuration has no groups. Add a group in the Soundstage first.
          </div>
          <template v-else>
            <VSelect
              v-model="groupId"
              :items="groupItems"
              label="Group"
              variant="outlined"
              density="compact"
              class="mb-2"
            />
            <VTextField
              v-model="name"
              label="Comp name"
              variant="outlined"
              density="compact"
              autofocus
              @keydown.enter="submit"
            />
            <div class="text-caption text-medium-emphasis">
              Makes a comp with one shuffle track; each selection becomes a clip on it.
            </div>
          </template>
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
          :text="tab === 'new' ? 'Create comp' : 'Add clips'"
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
