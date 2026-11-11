<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useAppStore } from '../stores/appStore';
import { trackDestinations } from '../utils/soundstage';
const open = defineModel<boolean>({ default: false });
const props = defineProps<{ trackId: string; boardId?: string }>();
const store = useAppStore();
const kind = ref<'scenes' | 'boards'>('scenes');
const selected = ref<string | null>(null);
const busy = ref(false);
const items = computed(() => trackDestinations(store.boards, props.trackId, props.boardId, kind.value));
const available = computed(() => items.value.some((item) => !item.added));
const canSubmit = computed(() => items.value.some((item) => item.id === selected.value && !item.added));
watch(open, (value) => { if (value) { kind.value = 'scenes'; selected.value = null; } });
watch(kind, () => { selected.value = null; });
async function submit(): Promise<void> {
  if (!canSubmit.value || busy.value) return;
  busy.value = true;
  try {
    if (await store.command(kind.value === 'scenes' ? 'attach_scene_track' : 'attach_track', { trackId: props.trackId, sceneId: selected.value, boardId: selected.value })) open.value = false;
  } finally { busy.value = false; }
}
</script>
<template>
  <VDialog v-model="open" max-width="520" :persistent="busy"><VCard title="Add shared track">
    <VTabs v-model="kind" color="primary" density="compact" class="px-4" :disabled="busy"><VTab value="scenes">Scenes</VTab><VTab value="boards">Boards</VTab></VTabs>
    <VForm @submit.prevent="submit"><VCardText>
      <VAutocomplete v-model="selected" :items="items" item-title="name" item-value="id" item-props="props" :label="kind === 'scenes' ? 'Scene' : 'Board'" variant="outlined" hide-details :disabled="busy" no-data-text="No matching destinations">
        <template #item="{ props: itemProps, item }"><VListItem v-bind="itemProps"><template #append><span v-if="item.added" class="text-caption ml-4">Already added</span></template></VListItem></template>
      </VAutocomplete>
      <p v-if="!items.length" class="text-body-2 mt-4">No {{ kind }} in this configuration yet.</p>
      <p v-else-if="!available" class="text-body-2 mt-4">This track is already on every {{ kind === 'scenes' ? 'scene' : 'board' }} in this configuration.</p>
      <p class="text-body-2 text-medium-emphasis mt-4">Reuses the shared clips and playback settings. {{ kind === 'scenes' ? 'The card starts switched off; scene volume and clip selection stay independent.' : 'The track appears in All tracks. Scene layouts stay as they are.' }}</p>
    </VCardText><VCardActions><VSpacer /><VBtn text="Cancel" :disabled="busy" @click="open = false" /><VBtn :text="kind === 'scenes' ? 'Add to scene' : 'Add to board'" color="primary" variant="tonal" type="submit" :disabled="!canSubmit || busy" :loading="busy" /></VCardActions></VForm>
  </VCard></VDialog>
</template>
