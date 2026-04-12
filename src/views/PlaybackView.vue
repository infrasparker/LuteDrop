<script setup lang="ts">
import { ref } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import GroupTab from '../components/GroupTab.vue';

const props = defineProps<{
  activeTab: number;
}>();

const store = useAppStore();
const { groups } = storeToRefs(store);

const showNewCompDialog = ref(false);
const newCompName = ref('');
const newCompGroupId = ref('');

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
  <VContainer fluid class="d-flex flex-column" style="height: 100%;">
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
  </VContainer>
</template>
