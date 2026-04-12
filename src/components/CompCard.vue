<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import { useContextMenuState } from '../composables/useContextMenu';

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

const borderStyle = computed(() => {
  switch (status.value) {
    case 'active': return '3px solid #4CAF50';
    case 'partial': return '3px solid #FF6D00';
    default: return '3px solid transparent';
  }
});

// Context menu state
const { menuVisible: contextMenu, menuX: contextMenuX, menuY: contextMenuY, openMenu: openContextMenu } = useContextMenuState();
const showDeleteConfirm = ref(false);

// Rename state
const showRenameDialog = ref(false);
const renameCompName = ref('');

function openRenameDialog(): void {
  renameCompName.value = props.comp.name;
  showRenameDialog.value = true;
}

async function renameComp(): Promise<void> {
  if (!renameCompName.value.trim()) return;
  await store.command('rename_comp', { compId: props.comp.id, name: renameCompName.value.trim() });
  renameCompName.value = '';
  showRenameDialog.value = false;
}

async function deleteComp(): Promise<void> {
  showDeleteConfirm.value = false;
  await audio.stopComp(props.comp.id);
  await store.command('delete_comp', { groupId: props.groupId, compId: props.comp.id });
}

function toggleComp(): void {
  if (status.value === 'active') {
    audio.stopComp(props.comp.id);
  } else {
    audio.activateComp(props.comp.id);
  }
}

function stopComp(): void {
  audio.stopComp(props.comp.id);
}
</script>

<template>
  <VCard
    :color="selected ? 'surface-variant' : 'surface-light'"
    variant="flat"
    class="comp-card pa-2 text-center"
    :style="{ width: '130px', cursor: 'pointer', border: borderStyle }"
    @click="emit('select', comp.id)"
    @contextmenu.prevent="openContextMenu($event)"
  >
    <div class="d-flex justify-center ga-1 mb-2">
      <VBtn
        :icon="status === 'active' ? 'mdi-pause' : 'mdi-play'"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="toggleComp"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="stopComp"
      />
    </div>
    <div class="text-subtitle-2 font-weight-bold text-truncate">{{ comp.name }}</div>
    <div class="text-caption" style="opacity: 0.6;">{{ comp.tracks.length }} tracks</div>

    <!-- Context menu -->
    <VMenu v-model="contextMenu" :target="[contextMenuX, contextMenuY]" close-on-content-click>
      <VList density="compact">
        <VListItem @click="openRenameDialog">
          <template #prepend><VIcon>mdi-pencil</VIcon></template>
          <VListItemTitle>Rename</VListItemTitle>
        </VListItem>
        <VListItem @click="showDeleteConfirm = true" class="text-error">
          <template #prepend><VIcon color="error">mdi-delete</VIcon></template>
          <VListItemTitle>Delete</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>

    <!-- Rename dialog -->
    <VDialog v-model="showRenameDialog" max-width="400">
      <VCard title="Rename Composition">
        <VCardText>
          <VTextField
            v-model="renameCompName"
            label="Composition Name"
            variant="outlined"
            autofocus
            @keydown.enter="renameComp"
          />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showRenameDialog = false" />
          <VBtn text="Rename" color="primary" variant="tonal" @click="renameComp" />
        </VCardActions>
      </VCard>
    </VDialog>

    <!-- Delete confirmation dialog -->
    <VDialog v-model="showDeleteConfirm" max-width="450">
      <VCard title="Delete Composition">
        <VCardText>
          Are you sure you want to delete "{{ comp.name }}"? This will remove all tracks in it.
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showDeleteConfirm = false" />
          <VBtn text="Delete" color="error" variant="tonal" @click="deleteComp" />
        </VCardActions>
      </VCard>
    </VDialog>
  </VCard>
</template>
