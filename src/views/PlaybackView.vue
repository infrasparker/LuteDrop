<script setup lang="ts">
import { ref } from 'vue';
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

async function createGroup(): Promise<void> {
  if (!newGroupName.value.trim()) return;
  await store.command('add_group', { name: newGroupName.value.trim() });
  newGroupName.value = '';
  showNewGroupDialog.value = false;
}

function onGroupDeleted(): void {
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
      @group-deleted="onGroupDeleted"
    />

    <div v-else class="text-center text-medium-emphasis pa-8">
      No groups yet. Click + to create one.
    </div>

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
