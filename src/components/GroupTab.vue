<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Group } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import CompCard from './CompCard.vue';
import CompDetails from './CompDetails.vue';

const props = defineProps<{
  group: Group;
}>();

const emit = defineEmits<{
  'add-comp': [groupId: string];
  'group-deleted': [];
}>();

const store = useAppStore();

const selectedCompId = ref<string | null>(null);

const selectedComp = computed(() =>
  props.group.comps.find((c) => c.id === selectedCompId.value) ?? null
);

// Rename state
const showRenameDialog = ref(false);
const renameGroupName = ref('');

function openRenameDialog(): void {
  renameGroupName.value = props.group.name;
  showRenameDialog.value = true;
}

async function renameGroup(): Promise<void> {
  if (!renameGroupName.value.trim()) return;
  await store.command('rename_group', { groupId: props.group.id, name: renameGroupName.value.trim() });
  renameGroupName.value = '';
  showRenameDialog.value = false;
}

// Delete state
const showDeleteDialog = ref(false);

async function deleteGroup(): Promise<void> {
  showDeleteDialog.value = false;
  await store.command('delete_group', { groupId: props.group.id });
  emit('group-deleted');
}
</script>

<template>
  <div class="d-flex" style="height: calc(100vh - 120px);">
    <!-- Left pane: group header + comp grid -->
    <div style="width: 300px; min-width: 300px; overflow-y: auto;" class="pr-3">
      <!-- Group header row -->
      <div class="d-flex align-center mb-3 pb-2" style="border-bottom: 1px solid rgba(232, 228, 220, 0.1);">
        <span class="text-subtitle-1 font-weight-bold flex-grow-1">{{ group.name }}</span>
        <VBtn
          icon="mdi-pencil"
          variant="text"
          size="x-small"
          @click="openRenameDialog"
        />
        <VBtn
          icon="mdi-delete"
          variant="text"
          size="x-small"
          color="error"
          @click="showDeleteDialog = true"
        />
      </div>

      <!-- Add comp + comp cards -->
      <div class="d-flex align-center mb-3">
        <VSpacer />
        <VBtn
          prepend-icon="mdi-plus"
          variant="tonal"
          size="small"
          @click="emit('add-comp', group.id)"
        >
          Add Comp
        </VBtn>
      </div>

      <div class="d-flex flex-wrap ga-2">
        <CompCard
          v-for="comp in group.comps"
          :key="comp.id"
          :comp="comp"
          :selected="comp.id === selectedCompId"
          @select="selectedCompId = $event"
        />
      </div>

      <div v-if="group.comps.length === 0" class="text-medium-emphasis text-center pa-4">
        No comps yet.
      </div>
    </div>

    <!-- Right pane: comp details -->
    <VDivider vertical class="mx-2" />
    <div style="flex: 1; overflow-y: auto;" class="pl-3">
      <CompDetails
        v-if="selectedComp"
        :comp="selectedComp"
        :group-id="group.id"
      />
      <div v-else class="text-medium-emphasis text-center pa-8">
        Select a composition to see its tracks.
      </div>
    </div>
  </div>

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
          Are you sure you want to delete group "{{ group.name }}"? This will remove all compositions and tracks in it.
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showDeleteDialog = false">Cancel</VBtn>
          <VBtn color="error" @click="deleteGroup">Delete</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>
