<script setup lang="ts">
import { ref, computed } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import ConfirmDialog from './ConfirmDialog.vue';
import PromptDialog from './PromptDialog.vue';

const store = useAppStore();
const { configurations, activeConfigurationId } = storeToRefs(store);

// Dialog state
const showNewDialog = ref(false);
const showDuplicateDialog = ref(false);
const showRenameDialog = ref(false);
const showDeleteConfirm = ref(false);

const activeConfig = computed(() =>
  configurations.value.find((c) => c.id === activeConfigurationId.value)
);

async function switchConfig(configId: string): Promise<void> {
  if (configId === activeConfigurationId.value) return;
  await store.command('load_configuration', { configId });
}

async function createNew(name: string): Promise<void> {
  await store.command('create_configuration', { name });
}

async function duplicateConfig(name: string): Promise<void> {
  await store.command('duplicate_configuration', { name });
}

async function renameConfig(name: string): Promise<void> {
  await store.command('rename_configuration', {
    configId: activeConfigurationId.value,
    name,
  });
}

function openDuplicateDialog(): void {
  showDuplicateDialog.value = true;
}

function openRenameDialog(): void {
  showRenameDialog.value = true;
}

async function deleteConfig(): Promise<void> {
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

    <VBtn icon="mdi-content-duplicate" variant="text" size="x-small" title="Duplicate configuration" @click="openDuplicateDialog" />
    <VBtn icon="mdi-pencil" variant="text" size="x-small" title="Rename configuration" @click="openRenameDialog" />
    <VBtn icon="mdi-delete" variant="text" size="x-small" title="Delete configuration" :disabled="configurations.length <= 1" color="error" @click="showDeleteConfirm = true" />

    <PromptDialog
      v-model="showNewDialog"
      title="New Configuration"
      label="Configuration Name"
      confirm-label="Create"
      @submit="createNew"
    />

    <PromptDialog
      v-model="showDuplicateDialog"
      title="Duplicate configuration"
      confirm-label="Duplicate"
      label="New Name"
      :initial="(activeConfig?.name ?? '') + ' Copy'"
      @submit="duplicateConfig"
    />

    <PromptDialog
      v-model="showRenameDialog"
      title="Rename Configuration"
      label="Configuration Name"
      :initial="activeConfig?.name ?? ''"
      confirm-label="Rename"
      @submit="renameConfig"
    />

    <ConfirmDialog
      v-model="showDeleteConfirm"
      title="Delete Configuration"
      :message="`Are you sure you want to delete &quot;${activeConfig?.name}&quot;? Its boards and scenes will be removed. Shared tracks stay in the inventory; audio files stay in the Library.`"
      confirm-label="Delete"
      confirm-color="error"
      @confirm="deleteConfig"
    />
  </div>
</template>
