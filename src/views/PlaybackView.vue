<script setup lang="ts">
import { ref, nextTick } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import GroupTab from '../components/GroupTab.vue';

const store = useAppStore();
const { groups } = storeToRefs(store);

const activeTab = ref(0);
const showNewGroupDialog = ref(false);
const newGroupName = ref('');
const showNewCompDialog = ref(false);
const newCompName = ref('');
const newCompGroupId = ref('');

// Context menu state
const contextMenu = ref(false);
const contextMenuX = ref(0);
const contextMenuY = ref(0);
const contextGroupId = ref('');
const contextGroupName = ref('');

// Rename dialog state
const showRenameDialog = ref(false);
const renameGroupName = ref('');

// Delete confirmation dialog state
const showDeleteDialog = ref(false);

function openContextMenu(event: MouseEvent, groupId: string, groupName: string): void {
  event.preventDefault();
  contextGroupId.value = groupId;
  contextGroupName.value = groupName;
  contextMenuX.value = event.clientX;
  contextMenuY.value = event.clientY;
  nextTick(() => {
    contextMenu.value = true;
  });
}

function openRenameDialog(): void {
  contextMenu.value = false;
  renameGroupName.value = contextGroupName.value;
  showRenameDialog.value = true;
}

async function renameGroup(): Promise<void> {
  if (!renameGroupName.value.trim()) return;
  await store.command('rename_group', { groupId: contextGroupId.value, name: renameGroupName.value.trim() });
  renameGroupName.value = '';
  showRenameDialog.value = false;
}

function openDeleteDialog(): void {
  contextMenu.value = false;
  showDeleteDialog.value = true;
}

async function createGroup(): Promise<void> {
  if (!newGroupName.value.trim()) return;
  await store.command('add_group', { name: newGroupName.value.trim() });
  newGroupName.value = '';
  showNewGroupDialog.value = false;
}

async function deleteGroup(): Promise<void> {
  showDeleteDialog.value = false;
  await store.command('delete_group', { groupId: contextGroupId.value });
  if (activeTab.value >= groups.value.length) {
    activeTab.value = Math.max(0, groups.value.length - 1);
  }
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
  <VContainer fluid>
    <div class="d-flex align-center mb-4 ga-3">
      <VTabs v-model="activeTab" density="compact" v-if="groups.length > 0">
        <VTab
          v-for="(group, i) in groups"
          :key="group.id"
          :value="i"
          @contextmenu="openContextMenu($event, group.id, group.name)"
        >
          {{ group.name }}
        </VTab>
      </VTabs>
      <VBtn
        icon="mdi-plus"
        variant="tonal"
        size="small"
        @click="showNewGroupDialog = true"
      />
    </div>

    <GroupTab
      v-if="groups.length > 0 && groups[activeTab]"
      :group="groups[activeTab]"
      @add-comp="openNewCompDialog"
    />

    <div v-else class="text-center text-medium-emphasis pa-8">
      No groups yet. Click + to create one.
    </div>

    <!-- Right-click context menu for tabs -->
    <VMenu
      v-model="contextMenu"
      :style="{ position: 'fixed', left: contextMenuX + 'px', top: contextMenuY + 'px' }"
      :target="[contextMenuX, contextMenuY]"
      location="bottom start"
    >
      <VList density="compact">
        <VListItem prepend-icon="mdi-pencil" @click="openRenameDialog">
          Rename
        </VListItem>
        <VListItem prepend-icon="mdi-delete" class="text-error" @click="openDeleteDialog">
          Delete Group
        </VListItem>
      </VList>
    </VMenu>

    <!-- Rename group dialog -->
    <VDialog v-model="showRenameDialog" max-width="400">
      <VCard>
        <VCardTitle>Rename Group</VCardTitle>
        <VCardText>
          <VTextField
            v-model="renameGroupName"
            label="Group Name"
            variant="outlined"
            autofocus
            @keydown.enter="renameGroup"
          />
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showRenameDialog = false">Cancel</VBtn>
          <VBtn color="primary" @click="renameGroup">Rename</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>

    <!-- Delete group confirmation dialog -->
    <VDialog v-model="showDeleteDialog" max-width="450">
      <VCard>
        <VCardTitle>Delete Group</VCardTitle>
        <VCardText>
          Are you sure you want to delete group "{{ contextGroupName }}"? This will remove all compositions and tracks in it.
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showDeleteDialog = false">Cancel</VBtn>
          <VBtn color="error" @click="deleteGroup">Delete</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>

    <!-- New group dialog -->
    <VDialog v-model="showNewGroupDialog" max-width="400">
      <VCard>
        <VCardTitle>New Group</VCardTitle>
        <VCardText>
          <VTextField
            v-model="newGroupName"
            label="Group Name"
            variant="outlined"
            autofocus
            @keydown.enter="createGroup"
          />
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showNewGroupDialog = false">Cancel</VBtn>
          <VBtn color="primary" @click="createGroup">Create</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>

    <!-- New comp dialog -->
    <VDialog v-model="showNewCompDialog" max-width="400">
      <VCard>
        <VCardTitle>New Composition</VCardTitle>
        <VCardText>
          <VTextField
            v-model="newCompName"
            label="Composition Name"
            variant="outlined"
            autofocus
            @keydown.enter="createComp"
          />
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showNewCompDialog = false">Cancel</VBtn>
          <VBtn color="primary" @click="createComp">Create</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </VContainer>
</template>
