<script setup lang="ts">
import { ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/appStore';

const model = defineModel<boolean>({ required: true });
const store = useAppStore();

const normalizeLoudness = ref(true);
const loading = ref(false);
const error = ref('');

watch(model, async (open) => {
  if (!open) return;
  error.value = '';
  loading.value = true;
  try {
    normalizeLoudness.value = await invoke<boolean>('get_normalize_music_loudness');
  } catch (e) {
    error.value = `Could not read settings: ${e}`;
  } finally {
    loading.value = false;
  }
});

// Takes effect at once, also for music playing now.
async function setNormalizeLoudness(v: boolean | null): Promise<void> {
  const enabled = !!v;
  const previous = normalizeLoudness.value;
  normalizeLoudness.value = enabled;
  if (!(await store.command('set_normalize_music_loudness', { enabled }))) {
    normalizeLoudness.value = previous;
  }
}
</script>

<template>
  <VDialog v-model="model" max-width="520">
    <VCard title="Settings">
      <VCardText>
        <div class="text-overline text-medium-emphasis">Playback</div>
        <VSwitch
          :model-value="normalizeLoudness"
          :disabled="loading"
          label="Even out music loudness (ReplayGain)"
          color="primary"
          hide-details
          density="compact"
          @update:model-value="setNormalizeLoudness"
        />
        <p class="text-caption text-medium-emphasis mt-1">
          Turns down music tracks that are louder than your library's typical level,
          using the files' ReplayGain tags. Ambiance is not changed.
        </p>
        <p v-if="error" class="text-error mt-2">{{ error }}</p>
      </VCardText>
      <VCardActions>
        <VSpacer />
        <VBtn @click="model = false">Close</VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
</template>
