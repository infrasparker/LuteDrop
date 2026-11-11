<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Clip, SceneReference } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from '../composables/useToast';
const open = defineModel<boolean>({ default: false });
const props = defineProps<{ trackId: string; clip: Clip }>();
const store = useAppStore();
const references = ref<SceneReference[]>([]);
const resolution = ref<string | null>(null);
const busy = ref(false);
const loading = ref(false);
const loadFailed = ref(false);
const replacements = computed(() => [
  ...(store.findTrack(props.trackId)?.clips.filter((c) => c.id !== props.clip.id).map((c) => ({ title: c.name, value: c.id })) ?? []),
  { title: 'Disable this track in affected scenes', value: '__disable__' },
]);
watch(open, async (value) => {
  if (!value) return;
  loading.value = true; loadFailed.value = false; resolution.value = null;
  try { references.value = await invoke<SceneReference[]>('get_track_references', { trackId: props.trackId, clipId: props.clip.id }); }
  catch (error) { loadFailed.value = true; useToast().showError(String(error)); }
  finally { loading.value = false; }
});
async function remove(): Promise<void> {
  if (busy.value || loading.value || loadFailed.value || (references.value.length > 0 && !resolution.value)) return;
  busy.value = true;
  try {
    const ok = await store.command('delete_clip', { trackId: props.trackId, clipId: props.clip.id,
      replacementClipId: resolution.value && resolution.value !== '__disable__' ? resolution.value : null,
      disableAffectedScenes: resolution.value === '__disable__' });
    if (ok) open.value = false;
  } finally { busy.value = false; }
}
</script>
<template>
  <VDialog v-model="open" max-width="540" :persistent="busy">
    <VCard title="Remove clip" rounded="lg">
      <VCardText>
        <p class="mb-4">Remove “{{ clip.name }}” from this shared track? The audio file stays in the Library.</p>
        <VProgressLinear v-if="loading" indeterminate color="primary" />
        <template v-else-if="references.length">
          <p class="text-body-2 mb-2">This clip is a saved selection in {{ references.length }} scene{{ references.length === 1 ? '' : 's' }}:</p>
          <VList density="compact" class="mb-4" max-height="220" style="overflow-y: auto;">
            <VListItem v-for="reference in references" :key="reference.sceneId" :title="reference.sceneName" :subtitle="`${reference.configurationName} / ${reference.boardName}`" />
          </VList>
          <VSelect v-model="resolution" label="Replace the saved selection with…" :items="replacements" variant="outlined" hide-details />
        </template>
        <p v-if="loadFailed" class="text-error">Could not check scene references. Close this dialog and try again.</p>
      </VCardText>
      <VDivider />
      <VCardActions class="px-4 pb-4"><VSpacer /><VBtn text="Cancel" :disabled="busy" @click="open = false" /><VBtn text="Remove clip" color="error" variant="tonal" :loading="busy" :disabled="loading || loadFailed || (references.length > 0 && !resolution)" @click="remove" /></VCardActions>
    </VCard>
  </VDialog>
</template>
