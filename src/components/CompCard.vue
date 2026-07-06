<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useContextMenuState } from '../composables/useContextMenu';
import ConfirmDialog from './ConfirmDialog.vue';
import PromptDialog from './PromptDialog.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
  selected: boolean;
}>();

const emit = defineEmits<{
  select: [compId: string];
}>();

const store = useAppStore();
const audio = useAudio();

const status = computed(() => store.getCompStatus(props.comp.id));

const cardClass = computed(() => ({
  'comp-selected': props.selected,
  'comp-active': status.value === 'active',
  'comp-partial': status.value === 'partial',
  'comp-paused': status.value === 'paused',
}));

// Context menu state
const { menuVisible: contextMenu, menuX: contextMenuX, menuY: contextMenuY, openMenu: openContextMenu } = useContextMenuState();
const showDeleteConfirm = ref(false);

// Rename state
const showRenameDialog = ref(false);

function openRenameDialog(): void {
  showRenameDialog.value = true;
}

async function renameComp(name: string): Promise<void> {
  await store.command('rename_comp', { compId: props.comp.id, name });
}

function confirmOrDeleteComp(): void {
  if (props.comp.tracks.length === 0) {
    deleteComp();
  } else {
    showDeleteConfirm.value = true;
  }
}

async function deleteComp(): Promise<void> {
  await audio.stopComp(props.comp.id);
  await store.command('delete_comp', { compId: props.comp.id });
}

function toggleComp(): void {
  if (status.value === 'active' || status.value === 'partial') {
    audio.pauseComp(props.comp.id);
  } else if (status.value === 'paused') {
    audio.resumeComp(props.comp.id);
  } else {
    audio.activateComp(props.comp.id);
  }
}

function stopComp(): void {
  audio.stopComp(props.comp.id);
}

const isCompMuted = computed(() => store.isCompMuted(props.comp.id));

function toggleMute(): void {
  audio.toggleCompMute(props.comp.id);
}
</script>

<template>
  <VListItem
    :class="cardClass"
    @click="emit('select', comp.id)"
    @contextmenu.prevent="openContextMenu($event)"
    rounded="lg"
    class="comp-item mb-1"
  >
    <template #prepend>
      <div class="d-flex align-center ga-1 mr-2">
        <VBtn
          :icon="status === 'active' || status === 'partial' ? 'mdi-pause' : 'mdi-play'"
          variant="tonal"
          size="small"
          rounded="circle"
          @click.stop="toggleComp"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          rounded="circle"
          :disabled="status === 'inactive'"
          @click.stop="stopComp"
        />
      </div>
    </template>

    <VListItemTitle :style="{ opacity: isCompMuted ? 0.5 : 1 }" class="text-body-1">
      {{ comp.name }}
    </VListItemTitle>
    <VListItemSubtitle>
      {{ comp.tracks.length }} {{ comp.tracks.length === 1 ? 'track' : 'tracks' }}
    </VListItemSubtitle>

    <template #append>
      <VBtn
        v-if="isCompMuted"
        :icon="isCompMuted ? 'mdi-volume-off' : 'mdi-volume-high'"
        :color="isCompMuted ? 'error' : undefined"
        variant="text"
        size="x-small"
        :title="isCompMuted ? 'Unmute comp' : 'Mute comp'"
        @click.stop="toggleMute"
      />
    </template>

    <!-- Context menu -->
    <VMenu v-model="contextMenu" :target="[contextMenuX, contextMenuY]" close-on-content-click>
      <VList density="compact">
        <VListItem @click="openRenameDialog">
          <template #prepend><VIcon>mdi-pencil</VIcon></template>
          <VListItemTitle>Rename</VListItemTitle>
        </VListItem>
        <VListItem @click="confirmOrDeleteComp" class="text-error">
          <template #prepend><VIcon color="error">mdi-delete</VIcon></template>
          <VListItemTitle>Delete</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>

    <!-- Rename dialog -->
    <PromptDialog
      v-model="showRenameDialog"
      title="Rename Composition"
      label="Composition Name"
      :initial="comp.name"
      confirm-label="Rename"
      @submit="renameComp"
    />

    <!-- Delete confirmation dialog -->
    <ConfirmDialog
      v-model="showDeleteConfirm"
      title="Delete Composition"
      :message="`Are you sure you want to delete &quot;${comp.name}&quot;? This will remove all tracks in it.`"
      confirm-label="Delete"
      confirm-color="error"
      @confirm="deleteComp"
    />
  </VListItem>
</template>

<style scoped>
.comp-item {
  border-left: 3px solid transparent;
  cursor: pointer;
}
.comp-selected {
  outline: 2px solid rgb(var(--v-theme-primary));
  outline-offset: -2px;
}
.comp-active {
  background: rgba(240, 168, 48, 0.08) !important;
  border-left: 3px solid #F0A830;
}
.comp-active.comp-selected {
  background: rgba(240, 168, 48, 0.14) !important;
}
.comp-partial {
  background: rgba(240, 168, 48, 0.05) !important;
  border-left: 3px solid rgba(240, 168, 48, 0.5);
}
.comp-paused {
  border-left: 3px solid rgba(240, 168, 48, 0.5);
}
</style>
