<script setup lang="ts">
import { ref, watch } from 'vue';
import type { Track } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useSoundstageUiStore } from '../stores/soundstageUi';
import { useToast } from '../composables/useToast';
const open = defineModel<boolean>({ default: false });
const props = defineProps<{ track: Track; sceneId?: string }>();
const store = useAppStore();
const ui = useSoundstageUiStore();
const destination = ref('inventory');
const busy = ref(false);
watch(open, (value) => { if (value) destination.value = props.sceneId ? 'scene' : 'inventory'; });
async function submit(): Promise<void> {
  if (busy.value) return;
  const sceneCopy = destination.value === 'scene' && !!props.sceneId;
  busy.value = true;
  const before = new Set(store.tracks.map((track) => track.id));
  try {
    const ok = await store.command(sceneCopy ? 'make_scene_track_independent' : 'duplicate_track', { trackId: props.track.id, sceneId: props.sceneId, name: `${props.track.name} (copy)` });
    if (!ok) return;
    open.value = false;
    const created = store.tracks.find((track) => !before.has(track.id));
    if (created) { if (sceneCopy) ui.setTrackExpanded(created.id, true); else ui.editTrackId = created.id; }
    if (sceneCopy) useToast().showInfo('Independent copy created. Load the scene to use its new track.');
  } finally { busy.value = false; }
}
</script>
<template>
  <VDialog v-model="open" max-width="500" :persistent="busy"><VCard title="Duplicate track"><VCardText>
    <p class="text-body-2 mb-4">Create an independent copy of “{{ track.name }}” and its clips. Audio files stay shared in the Library.</p>
    <VSelect v-if="sceneId" v-model="destination" :items="[{ title: 'This scene — replace shared track', value: 'scene' }, { title: 'Track inventory', value: 'inventory' }]" label="Destination" variant="outlined" hide-details :disabled="busy" />
    <p class="text-body-2 text-medium-emphasis mt-4">{{ destination === 'scene' ? 'Only this scene gets the new track. Its saved volume and clip selection are preserved. Load the scene to apply the replacement.' : 'The copy opens in the track editor. Add it to scenes or boards when ready.' }}</p>
  </VCardText><VCardActions><VSpacer /><VBtn text="Cancel" :disabled="busy" @click="open = false" /><VBtn text="Create copy" color="primary" variant="tonal" :loading="busy" @click="submit" /></VCardActions></VCard></VDialog>
</template>
