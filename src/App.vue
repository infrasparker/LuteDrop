<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { storeToRefs } from 'pinia';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import { useToast } from './composables/useToast';
import { useContextMenuState } from './composables/useContextMenu';
import type { AppState, Group } from '@generated/index';
import PlaybackView from './views/PlaybackView.vue';

const store = useAppStore();
const { groups } = storeToRefs(store);
const audio = useAudio();
const toast = useToast();
const serverUrl = ref('');
const paused = ref(false);

const activeTab = ref(0);

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

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);

  try {
    const info = await invoke<{ ip: string; port: number }>('get_server_info');
    serverUrl.value = `http://${info.ip}:${info.port}`;
  } catch (e) {
    console.warn('Could not get server info:', e);
  }
});

function setMasterVolume(v: number): void {
  audio.masterVolume.value = v;
  audio.setMasterVolume(v);
}

function stopAll(): void {
  paused.value = false;
  audio.stopAll();
}

function togglePause(): void {
  if (paused.value) {
    audio.resumeAll();
    paused.value = false;
  } else {
    audio.pauseAll();
    paused.value = true;
  }
}

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
</script>

<template>
  <VApp @contextmenu.prevent>
    <VAppBar color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
        <span v-if="serverUrl" class="text-caption text-medium-emphasis ml-3">
          Remote: {{ serverUrl }}
        </span>
      </VAppBarTitle>

      <template #append>
        <VIcon class="mr-1">mdi-volume-high</VIcon>
        <VSlider
          :model-value="audio.masterVolume.value"
          @update:model-value="setMasterVolume"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 120px"
        />
        <VBtn
          :icon="paused ? 'mdi-play' : 'mdi-pause'"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="togglePause"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="stopAll"
        />
      </template>
      <template #extension>
        <VTabs v-model="activeTab" color="primary">
          <VTab
            v-for="(group, i) in groups"
            :key="group.id"
            :value="i"
            @contextmenu.prevent="openContextMenu($event, group)"
            class="tab-truncate"
            :style="isGroupActive(group) ? { color: '#FF6D00' } : {}"
          >
            {{ group.name }}
          </VTab>
          <VTab :value="activeTab" @click.stop.prevent="showNewGroupDialog = true" style="min-width: 0;" class="px-2">
            <VIcon>mdi-plus</VIcon>
          </VTab>
        </VTabs>
      </template>
    </VAppBar>

    <VMain class="d-flex flex-column" style="height: 100vh;">
      <PlaybackView :active-tab="activeTab" class="flex-grow-1" style="overflow: hidden;" />
    </VMain>

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

    <VSnackbar v-model="toast.visible.value" :color="toast.color.value" :timeout="3000">
      {{ toast.message.value }}
    </VSnackbar>
  </VApp>
</template>

<style>
.tab-truncate {
  max-width: 150px !important;
}
.tab-truncate .v-btn__content {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  display: block !important;
}
</style>
