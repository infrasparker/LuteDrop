<script setup lang="ts">
import { ref, computed } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';

const store = useAppStore();
const { configurations, activeConfigurationId } = storeToRefs(store);

// Dialog state
const showNewDialog = ref(false);
const showSaveAsDialog = ref(false);
const showRenameDialog = ref(false);
const showDeleteConfirm = ref(false);

const newName = ref('');
const saveAsName = ref('');
const renameName = ref('');

const activeConfig = computed(() =>
  configurations.value.find((c) => c.id === activeConfigurationId.value)
);

async function switchConfig(configId: string): Promise<void> {
  if (configId === activeConfigurationId.value) return;
  await store.command('load_configuration', { configId });
}

async function createNew(): Promise<void> {
  if (!newName.value.trim()) return;
  await store.command('create_configuration', { name: newName.value.trim() });
  newName.value = '';
  showNewDialog.value = false;
}

async function saveAs(): Promise<void> {
  if (!saveAsName.value.trim()) return;
  await store.command('duplicate_configuration', { name: saveAsName.value.trim() });
  saveAsName.value = '';
  showSaveAsDialog.value = false;
}

async function renameConfig(): Promise<void> {
  if (!renameName.value.trim()) return;
  await store.command('rename_configuration', {
    configId: activeConfigurationId.value,
    name: renameName.value.trim(),
  });
  renameName.value = '';
  showRenameDialog.value = false;
}

function openSaveAsDialog(): void {
  saveAsName.value = (activeConfig.value?.name ?? '') + ' Copy';
  showSaveAsDialog.value = true;
}

function openRenameDialog(): void {
  renameName.value = activeConfig.value?.name ?? '';
  showRenameDialog.value = true;
}

async function deleteConfig(): Promise<void> {
  showDeleteConfirm.value = false;
  await store.command('delete_configuration', { configId: activeConfigurationId.value });
}
</script>

<template>
  <div class="d-flex align-center ga-1">
    <VMenu>
      <template #activator="{ props }">
        <VBtn
          v-bind="props"
          variant="text"
          size="small"
          append-icon="mdi-chevron-down"
          class="text-none"
        >
          {{ activeConfig?.name ?? 'No Configuration' }}
        </VBtn>
      </template>
      <VList density="compact" min-width="200">
        <VListItem
          v-for="config in configurations"
          :key="config.id"
          :active="config.id === activeConfigurationId"
          @click="switchConfig(config.id)"
        >
          <VListItemTitle>{{ config.name }}</VListItemTitle>
        </VListItem>
        <VDivider />
        <VListItem @click="showNewDialog = true">
          <template #prepend><VIcon size="small">mdi-plus</VIcon></template>
          <VListItemTitle>New Configuration</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>

    <VBtn icon="mdi-content-copy" variant="text" size="x-small" title="Duplicate" @click="openSaveAsDialog" />
    <VBtn icon="mdi-pencil" variant="text" size="x-small" title="Rename" @click="openRenameDialog" />
    <VBtn icon="mdi-delete" variant="text" size="x-small" title="Delete" :disabled="configurations.length <= 1" color="error" @click="showDeleteConfirm = true" />

    <VDialog v-model="showNewDialog" max-width="400">
      <VCard title="New Configuration">
        <VCardText>
          <VTextField
            v-model="newName"
            label="Configuration Name"
            variant="outlined"
            autofocus
            @keydown.enter="createNew"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showNewDialog = false" />
          <VBtn text="Create" color="primary" variant="tonal" @click="createNew" />
        </VCardActions>
      </VCard>
    </VDialog>

    <VDialog v-model="showSaveAsDialog" max-width="400">
      <VCard title="Save Configuration As">
        <VCardText>
          <VTextField
            v-model="saveAsName"
            label="New Name"
            variant="outlined"
            autofocus
            @keydown.enter="saveAs"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showSaveAsDialog = false" />
          <VBtn text="Save" color="primary" variant="tonal" @click="saveAs" />
        </VCardActions>
      </VCard>
    </VDialog>

    <VDialog v-model="showRenameDialog" max-width="400">
      <VCard title="Rename Configuration">
        <VCardText>
          <VTextField
            v-model="renameName"
            label="Configuration Name"
            variant="outlined"
            autofocus
            @keydown.enter="renameConfig"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showRenameDialog = false" />
          <VBtn text="Rename" color="primary" variant="tonal" @click="renameConfig" />
        </VCardActions>
      </VCard>
    </VDialog>

    <VDialog v-model="showDeleteConfirm" max-width="450">
      <VCard title="Delete Configuration">
        <VCardText>
          Are you sure you want to delete "{{ activeConfig?.name }}"? All groups, compositions, and tracks in it will be permanently removed.
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showDeleteConfirm = false" />
          <VBtn text="Delete" color="error" variant="tonal" @click="deleteConfig" />
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>
