<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { playbackLabel, trackClipSummary, trackLocations, trackSearchText } from '../utils/soundstage';
const open = defineModel<boolean>({ default: false });
const props = defineProps<{ boardId?: string; sceneId?: string; createOnly?: boolean }>();
const store = useAppStore();
const ui = useSoundstageUiStore();
const tab = ref('existing');
const selected = ref<string | null>(null);
const name = ref('');
const files = ref<string[]>([]);
const mode = ref('seamless');
const busy = ref(false);
const existing = computed(() => store.tracks);
function alreadyAdded(id: string): boolean { return !!(props.sceneId ? store.findScene(props.sceneId)?.tracks.some((state) => state.trackId === id) : store.boards.find((board) => board.id === props.boardId)?.trackIds.includes(id)); }
const existingItems = computed(() => existing.value.map((track) => ({ ...track, props: { disabled: alreadyAdded(track.id) }, added: alreadyAdded(track.id),
  detail: `${playbackLabel(track.mode)} · ${trackLocations(track.id, store.boards).join(' · ') || 'Not on a board in this configuration'}`,
  clipSummary: trackClipSummary(track), searchText: trackSearchText(track, store.boards) })));
const canSubmit = computed(() => tab.value === 'existing' ? !!selected.value && !alreadyAdded(selected.value) : !!(name.value.trim() || files.value.length));
watch(open, (value) => {
  if (!value) return;
  tab.value = props.createOnly || !existing.value.length ? 'new' : 'existing';
  selected.value = null; name.value = ''; files.value = []; mode.value = 'seamless';
});
async function submit(): Promise<void> {
  if (!canSubmit.value || busy.value) return;
  busy.value = true;
  try {
    if (tab.value === 'existing' && props.boardId) {
      if (await store.command(props.sceneId ? 'attach_scene_track' : 'attach_track', { boardId: props.boardId, sceneId: props.sceneId, trackId: selected.value })) open.value = false;
    } else {
      const before = new Set(store.tracks.map((track) => track.id));
      const ok = await store.command('add_track', { boardId: props.boardId ?? null, sceneId: props.sceneId ?? null,
        name: name.value.trim() || store.findLibraryEntry(files.value[0])?.name || 'New track',
        libraryEntryIds: files.value, mode: mode.value });
      if (ok) {
        open.value = false;
        const created = store.tracks.find((track) => !before.has(track.id));
        if (created && !created.clips.length) { if (props.boardId) ui.setTrackExpanded(created.id, true); else ui.editTrackId = created.id; }
      }
    }
  } finally { busy.value = false; }
}
</script>
<template>
  <VDialog v-model="open" max-width="540" :persistent="busy">
    <VCard :title="createOnly ? 'New track' : sceneId ? 'Add track to scene' : 'Add track to board'" rounded="lg">
      <VTabs v-if="!createOnly" v-model="tab" color="primary" density="compact" class="px-4"><VTab value="existing">Existing track</VTab><VTab value="new">New track</VTab></VTabs>
      <VForm @submit.prevent="submit">
        <VCardText>
          <template v-if="tab === 'existing'">
            <VAutocomplete v-model="selected" :items="existingItems" item-title="name" item-value="id" item-props="props" :disabled="busy" label="Track inventory" variant="outlined" no-data-text="No matching tracks available in this view" hide-details autofocus :custom-filter="(value, query, item) => (item?.raw.searchText ?? value).includes(query.trim().normalize('NFKC').toLocaleLowerCase())"><template #item="{ props: itemProps, item }"><VListItem v-bind="itemProps" :subtitle="item.detail"><template #append><span v-if="item.added" class="text-caption ml-4">Already in this {{ sceneId ? 'scene' : 'board' }}</span><span v-else class="picker-clips" :title="item.clipSummary">{{ item.clipSummary }}</span></template></VListItem></template></VAutocomplete>
            <p class="text-body-2 text-medium-emphasis mt-4">Reuses the shared track and its clips. Scenes keep their own volume, selection, and on/off settings.</p>
          </template>
          <template v-else>
            <VTextField v-model="name" label="Track name" placeholder="Weather, travel music, distant thunder…" variant="outlined" hide-details autofocus class="mb-4" />
            <VAutocomplete v-model="files" :items="store.libraryEntries" item-title="name" item-value="id" label="Audio files from Library" multiple chips closable-chips variant="outlined" hide-details class="mb-4" />
            <VSelect v-model="mode" :items="[{ title: 'Seamless', value: 'seamless' }, { title: 'Shuffle clips', value: 'shuffle' }, { title: 'Sequence clips', value: 'sequence' }, { title: 'One shot', value: 'oneShot' }]" label="Playback" variant="outlined" hide-details />
            <p class="text-caption text-medium-emphasis mt-4">Add clips and change shared playback settings from the track’s editor.</p>
          </template>
        </VCardText>
        <VDivider />
        <VCardActions class="px-4 pb-4"><VSpacer /><VBtn text="Cancel" :disabled="busy" @click="open = false" /><VBtn :text="tab === 'existing' ? 'Add track' : 'Create track'" color="primary" variant="tonal" type="submit" :disabled="!canSubmit || busy" :loading="busy" /></VCardActions>
      </VForm>
    </VCard>
  </VDialog>
</template>
<style scoped>
.picker-clips { max-width: 210px; padding-left: 16px; font-size: 11px; color: rgba(var(--v-theme-on-surface), .6); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
@media(max-width:700px) { .picker-clips { max-width: 130px; } }
</style>
