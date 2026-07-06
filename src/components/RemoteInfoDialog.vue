<script setup lang="ts">
import { ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';

const model = defineModel<boolean>({ required: true });

const url = ref('');
const error = ref('');

watch(model, async (open) => {
  if (!open) return;
  error.value = '';
  url.value = '';
  try {
    const info = await invoke<{ url: string }>('get_server_info');
    url.value = info.url;
  } catch (e) {
    error.value = `Could not read server info: ${e}`;
  }
});
</script>

<template>
  <VDialog v-model="model" max-width="520">
    <VCard title="Phone Remote">
      <VCardText>
        <p class="mb-2">Open this address in your phone's browser (same Wi-Fi network):</p>
        <code v-if="url" class="d-block pa-2 text-body-2">{{ url }}</code>
        <p v-if="error" class="text-error">{{ error }}</p>
        <p class="text-caption mt-2">
          The link contains a pairing code that changes each time LuteDrop starts —
          reopen this dialog for a fresh link after a restart.
        </p>
      </VCardText>
      <VCardActions>
        <VSpacer />
        <VBtn @click="model = false">Close</VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
</template>
