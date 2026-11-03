<script setup lang="ts">
import { ref, watch } from 'vue';
import { useBroadcast } from '../composables/useBroadcast';
import { useToast } from '../composables/useToast';
import ConfirmDialog from './ConfirmDialog.vue';

const model = defineModel<boolean>({ required: true });

const broadcast = useBroadcast();
const toast = useToast();
const { status, busy } = broadcast;

// Local drafts, so typing doesn't change anything until Save is pressed.
const publicBase = ref('');
const port = ref('');
const confirmRegenerate = ref(false);

function seedDrafts(): void {
  publicBase.value = status.value?.publicBase ?? '';
  port.value = String(status.value?.port ?? '');
}

// Seed the drafts only when the dialog opens (after a fresh status read), never
// on later status updates, so a live listener-count event can't wipe typing.
watch(model, async (open) => {
  if (!open) return;
  await broadcast.refresh();
  seedDrafts();
});

async function save(): Promise<void> {
  await broadcast.saveSettings(Number(port.value), publicBase.value.trim());
  seedDrafts();
}

async function copy(text: string | null | undefined): Promise<void> {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
    toast.showSuccess('Copied');
  } catch (e) {
    toast.showError(`Could not copy: ${e}`);
  }
}
</script>

<template>
  <VDialog v-model="model" max-width="520">
    <VCard title="Broadcast">
      <VCardText>
        <VSwitch
          label="Broadcast"
          color="primary"
          hide-details
          :model-value="status?.enabled ?? false"
          :disabled="busy || !status"
          @update:model-value="(on) => broadcast.setEnabled(!!on)"
        />
        <p v-if="status?.error" class="text-error mb-2">{{ status.error }}</p>

        <VTextField
          label="Players on the internet"
          class="mt-4"
          readonly
          density="compact"
          :model-value="status?.publicUrl ?? ''"
          placeholder="Set a public address below"
          persistent-placeholder
          append-inner-icon="mdi-content-copy"
          @click:append-inner="copy(status?.publicUrl)"
        />
        <VTextField
          label="Same Wi-Fi"
          readonly
          density="compact"
          :model-value="status?.lanUrl ?? ''"
          append-inner-icon="mdi-content-copy"
          @click:append-inner="copy(status?.lanUrl)"
        />

        <VTextField
          v-model="publicBase"
          label="Public address"
          density="compact"
          hint="The hostname Caddy points at this computer, e.g. https://stream.lutedrop.irslandia.duckdns.org"
          persistent-hint
        />
        <VTextField
          v-model="port"
          label="Port"
          type="number"
          density="compact"
          class="mt-4"
          :disabled="status?.enabled ?? false"
        />

        <div class="d-flex ga-2 mt-2">
          <VBtn variant="tonal" color="primary" :disabled="busy" @click="save">Save</VBtn>
          <VBtn variant="tonal" :disabled="busy" @click="confirmRegenerate = true">Regenerate key</VBtn>
        </div>

        <p v-if="status?.enabled" class="text-caption mt-3">{{ status.listenerCount }} listening</p>
      </VCardText>
      <VCardActions>
        <VSpacer />
        <VBtn @click="model = false">Close</VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
  <ConfirmDialog
    v-model="confirmRegenerate"
    title="Regenerate key?"
    message="Players will need the new link."
    confirm-label="Regenerate"
    @confirm="broadcast.regenerateKey()"
  />
</template>
