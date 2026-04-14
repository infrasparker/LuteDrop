<script setup lang="ts">
import { ref, watch } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import { useContextMenuState } from '../composables/useContextMenu';
import type { Group } from '@generated/index';
import GroupTab from '../components/GroupTab.vue';
import ConfigurationPicker from '../components/ConfigurationPicker.vue';

const store = useAppStore();
const { groups, activeConfigurationId } = storeToRefs(store);

const activeTab = ref(0);

watch(activeConfigurationId, () => {
  activeTab.value = 0;
});

// New group dialog state
const showNewGroupDialog = ref(false);
const newGroupName = ref('');

// Context menu state
const { menuVisible: contextMenu, menuX: contextMenuX, menuY: contextMenuY, openMenu } = useContextMenuState();
const contextGroupId = ref('');
const contextGroupName = ref('');

// Rename group state
const showRenameDialog = ref(false);
const renameGroupName = ref('');

// Delete group state
const showDeleteConfirm = ref(false);

// New comp dialog state
const showNewCompDialog = ref(false);
const newCompName = ref('');
const newCompGroupId = ref('');

function isGroupActive(group: Group): boolean {
  return group.comps.some((comp) =>
    comp.tracks.some((track) => {
      const s = store.playbackStates.get(track.id);
      return s?.isPlaying || s?.isPaused;
    })
  );
}

function openContextMenu(event: MouseEvent, group: Group): void {
  contextGroupId.value = group.id;
  contextGroupName.value = group.name;
  openMenu(event);
}

function openRenameDialog(): void {
  renameGroupName.value = contextGroupName.value;
  showRenameDialog.value = true;
}

async function renameGroup(): Promise<void> {
  if (!renameGroupName.value.trim()) return;
  await store.command('rename_group', { groupId: contextGroupId.value, name: renameGroupName.value.trim() });
  renameGroupName.value = '';
  showRenameDialog.value = false;
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
  showDeleteConfirm.value = false;
  await store.command('delete_group', { groupId: contextGroupId.value });
  if (activeTab.value >= groups.value.length) {
    activeTab.value = Math.max(0, groups.value.length - 1);
  }
}

async function createGroup(): Promise<void> {
  if (!newGroupName.value.trim()) return;
  await store.command('add_group', { name: newGroupName.value.trim() });
  newGroupName.value = '';
  showNewGroupDialog.value = false;
  activeTab.value = groups.value.length - 1;
}

function openNewCompDialog(groupId: string): void {
  newCompGroupId.value = groupId;
  newCompName.value = '';
  showNewCompDialog.value = true;
}

async function createComp(): Promise<void> {
  if (!newCompName.value.trim()) return;
  await store.command('add_comp', { groupId: newCompGroupId.value, name: newCompName.value.trim() });
  newCompName.value = '';
  showNewCompDialog.value = false;
}
</script>

<template>
  <div class="d-flex flex-column" style="height: 100%;">
    <!-- Teleport group tabs and config picker into the app bar -->
    <Teleport to="#app-bar-extension">
      <ConfigurationPicker />
      <VDivider vertical class="mx-1 my-2" />
      <VTabs v-model="activeTab" color="primary" density="compact" class="app-bar-tabs">
        <VTab
          v-for="(group, i) in groups"
          :key="group.id"
          :value="i"
          @contextmenu.prevent="openContextMenu($event, group)"
          :class="{ 'tab-truncate': true, 'tab-active': isGroupActive(group) }"
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
    <VDialog v-model="showNewGroupDialog" max-width="400">
      <VCard title="New Group">
        <VCardText>
          <VTextField
            v-model="newGroupName"
            label="Group Name"
            variant="outlined"
            autofocus
            @keydown.enter="createGroup"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showNewGroupDialog = false" />
          <VBtn text="Create" color="primary" variant="tonal" @click="createGroup" />
        </VCardActions>
      </VCard>
    </VDialog>

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
    <VDialog v-model="showRenameDialog" max-width="400">
      <VCard title="Rename Group">
        <VCardText>
          <VTextField
            v-model="renameGroupName"
            label="Group Name"
            variant="outlined"
            autofocus
            @keydown.enter="renameGroup"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showRenameDialog = false" />
          <VBtn text="Rename" color="primary" variant="tonal" @click="renameGroup" />
        </VCardActions>
      </VCard>
    </VDialog>

    <!-- Delete group confirmation dialog -->
    <VDialog v-model="showDeleteConfirm" max-width="450">
      <VCard title="Delete Group">
        <VCardText>
          Are you sure you want to delete group "{{ contextGroupName }}"? This will remove all compositions and tracks in it.
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showDeleteConfirm = false" />
          <VBtn text="Delete" color="error" variant="tonal" @click="deleteGroup" />
        </VCardActions>
      </VCard>
    </VDialog>

    <!-- New comp dialog -->
    <VDialog v-model="showNewCompDialog" max-width="400">
      <VCard title="New Composition">
        <VCardText>
          <VTextField
            v-model="newCompName"
            label="Composition Name"
            variant="outlined"
            autofocus
            @keydown.enter="createComp"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showNewCompDialog = false" />
          <VBtn text="Create" color="primary" variant="tonal" @click="createComp" />
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>

<style scoped>
.tab-truncate {
  max-width: 150px !important;
}
.tab-truncate :deep(.v-btn__content) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  display: block !important;
}
.tab-active {
  border-left: 3px solid #F0A830;
  background: rgba(240, 168, 48, 0.08);
}
.app-bar-tabs {
  height: 100%;
}
</style>
