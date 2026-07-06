<script setup lang="ts">
import { ref, watch } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import { useContextMenuState } from '../composables/useContextMenu';
import type { Group } from '@generated/index';
import { isGroupActive } from '../shared/playback';
import GroupTab from '../components/GroupTab.vue';
import ConfigurationPicker from '../components/ConfigurationPicker.vue';
import ConfirmDialog from '../components/ConfirmDialog.vue';
import PromptDialog from '../components/PromptDialog.vue';

const store = useAppStore();
const { groups, activeConfigurationId } = storeToRefs(store);

const activeTab = ref(0);

watch(activeConfigurationId, () => {
  activeTab.value = 0;
});

// New group dialog state
const showNewGroupDialog = ref(false);

// Context menu state
const { menuVisible: contextMenu, menuX: contextMenuX, menuY: contextMenuY, openMenu } = useContextMenuState();
const contextGroupId = ref('');
const contextGroupName = ref('');

// Rename group state
const showRenameDialog = ref(false);

// Delete group state
const showDeleteConfirm = ref(false);

// New comp dialog state
const showNewCompDialog = ref(false);
const newCompGroupId = ref('');

function openContextMenu(event: MouseEvent, group: Group): void {
  contextGroupId.value = group.id;
  contextGroupName.value = group.name;
  openMenu(event);
}

function openRenameDialog(): void {
  showRenameDialog.value = true;
}

async function renameGroup(name: string): Promise<void> {
  await store.command('rename_group', { groupId: contextGroupId.value, name });
}

function confirmOrDeleteGroup(): void {
  const group = groups.value.find((g) => g.id === contextGroupId.value);
  if (group && group.comps.length === 0) {
    deleteGroup();
  } else {
    showDeleteConfirm.value = true;
  }
}

async function deleteGroup(): Promise<void> {
  await store.command('delete_group', { groupId: contextGroupId.value });
  if (activeTab.value >= groups.value.length) {
    activeTab.value = Math.max(0, groups.value.length - 1);
  }
}

async function createGroup(name: string): Promise<void> {
  await store.command('add_group', { name });
  activeTab.value = groups.value.length - 1;
}

function openNewCompDialog(groupId: string): void {
  newCompGroupId.value = groupId;
  showNewCompDialog.value = true;
}

async function createComp(name: string): Promise<void> {
  await store.command('add_comp', { groupId: newCompGroupId.value, name });
}
</script>

<template>
  <div class="d-flex flex-column" style="height: 100%;">
    <!-- Teleport config + tabs into app bar -->
    <Teleport to="#app-bar-main">
      <ConfigurationPicker />
      <VDivider vertical class="mx-2 my-2" />
      <VTabs v-model="activeTab" color="primary" density="compact" class="app-bar-tabs">
        <VTab
          v-for="(group, i) in groups"
          :key="group.id"
          :value="i"
          @contextmenu.prevent="openContextMenu($event, group)"
          :class="{ 'tab-active': isGroupActive(group, store.playbackStates) }"
          size="small"
        >
          {{ group.name }}
        </VTab>
        <VBtn
          icon="mdi-plus"
          variant="text"
          size="x-small"
          class="align-self-center ml-1"
          @click="showNewGroupDialog = true"
        />
      </VTabs>
    </Teleport>

    <!-- Main content area -->
    <div class="d-flex flex-column flex-grow-1 pa-3" style="overflow: hidden;">
      <GroupTab
        v-if="groups.length > 0 && groups[activeTab]"
        :group="groups[activeTab]"
        @add-comp="openNewCompDialog"
        class="flex-grow-1"
        style="overflow: hidden;"
      />
      <div v-else class="text-center text-medium-emphasis pa-8">
        No groups yet. Click + to create one.
      </div>
    </div>

    <!-- New group dialog -->
    <PromptDialog
      v-model="showNewGroupDialog"
      title="New Group"
      label="Group Name"
      :initial="''"
      confirm-label="Create"
      @submit="createGroup"
    />

    <!-- Group tab context menu -->
    <VMenu v-model="contextMenu" :target="[contextMenuX, contextMenuY]" close-on-content-click>
      <VList density="compact">
        <VListItem @click="openRenameDialog">
          <template #prepend><VIcon>mdi-pencil</VIcon></template>
          <VListItemTitle>Rename</VListItemTitle>
        </VListItem>
        <VListItem @click="confirmOrDeleteGroup" class="text-error">
          <template #prepend><VIcon color="error">mdi-delete</VIcon></template>
          <VListItemTitle>Delete</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>

    <!-- Rename group dialog -->
    <PromptDialog
      v-model="showRenameDialog"
      title="Rename Group"
      label="Group Name"
      :initial="contextGroupName"
      confirm-label="Rename"
      @submit="renameGroup"
    />

    <!-- Delete group confirmation dialog -->
    <ConfirmDialog
      v-model="showDeleteConfirm"
      title="Delete Group"
      :message="`Are you sure you want to delete group &quot;${contextGroupName}&quot;? This will remove all compositions and tracks in it.`"
      confirm-label="Delete"
      confirm-color="error"
      @confirm="deleteGroup"
    />

    <!-- New comp dialog -->
    <PromptDialog
      v-model="showNewCompDialog"
      title="New Composition"
      label="Composition Name"
      confirm-label="Create"
      @submit="createComp"
    />
  </div>
</template>

<style scoped>
.tab-active {
  border-left: 3px solid #F0A830;
  background: rgba(240, 168, 48, 0.08);
}
.app-bar-tabs {
  height: 100%;
}
</style>
