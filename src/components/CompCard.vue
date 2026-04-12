<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted } from 'vue';
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';

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
    case 'active': return '2px solid #FF6D00';
    case 'partial': return '2px solid #BF360C';
    default: return '2px solid transparent';
  }
});

// Context menu state
const contextMenu = ref(false);
const contextMenuX = ref(0);
const contextMenuY = ref(0);
const showDeleteConfirm = ref(false);

function closeMenu(): void {
  contextMenu.value = false;
}

async function openContextMenu(event: MouseEvent): Promise<void> {
  contextMenu.value = false;
  await nextTick();
  contextMenuX.value = event.clientX;
  contextMenuY.value = event.clientY;
  contextMenu.value = true;
}

onMounted(() => {
  // Close menu when clicking or right-clicking anywhere else
  document.addEventListener('click', closeMenu);
  document.addEventListener('contextmenu', closeMenu);
});

onUnmounted(() => {
  document.removeEventListener('click', closeMenu);
  document.removeEventListener('contextmenu', closeMenu);
});

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
    :color="selected ? 'primary' : 'surface-light'"
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
        <VListItem @click="showDeleteConfirm = true" class="text-error">
          <template #prepend><VIcon color="error">mdi-delete</VIcon></template>
          <VListItemTitle>Delete</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>

    <!-- Delete confirmation dialog -->
    <VDialog v-model="showDeleteConfirm" max-width="450">
      <VCard>
        <VCardTitle>Delete Composition</VCardTitle>
        <VCardText>
          Are you sure you want to delete "{{ comp.name }}"? This will remove all tracks in it.
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showDeleteConfirm = false">Cancel</VBtn>
          <VBtn color="error" @click="deleteComp">Delete</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </VCard>
</template>
