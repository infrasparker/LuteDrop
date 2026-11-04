<script setup lang="ts">
import { ref, nextTick } from 'vue';
import { useAppStore } from '../stores/appStore';

/// Tag chips for one library row, plus an inline "+" that turns into a
/// small autocomplete so tags can be added without opening the editor.
const props = defineProps<{
  entryId: string;
  tags: string[];
}>();

const store = useAppStore();

const editing = ref(false);
const draft = ref<string | null>(null);
const inputRef = ref<{ focus: () => void } | null>(null);

async function persist(tags: string[]): Promise<void> {
  await store.command('update_library_entry_tags', { entryId: props.entryId, tags });
}

function removeTag(tag: string): void {
  persist(props.tags.filter((t) => t !== tag));
}

async function startEditing(): Promise<void> {
  editing.value = true;
  await nextTick();
  inputRef.value?.focus();
}

function stopEditing(): void {
  editing.value = false;
  draft.value = null;
}

/// VCombobox emits the typed text on Enter and the picked item on
/// selection; either way we add it and clear the field so the next tag
/// can be typed straight away.
function onDraft(value: string | null): void {
  const tag = (value ?? '').trim();
  if (tag.length > 0 && !props.tags.includes(tag)) {
    persist([...props.tags, tag]);
  }
  // Clear after the combobox has settled its own model update.
  nextTick(() => { draft.value = null; });
}

function onBlur(): void {
  // VCombobox commits pending text to the model on blur (which triggers
  // onDraft); close once that has had a chance to run.
  nextTick(stopEditing);
}
</script>

<template>
  <div class="d-flex align-center flex-wrap ga-1 tag-cell" @click.stop>
    <VChip
      v-for="tag in tags"
      :key="tag"
      size="small"
      closable
      @click:close.stop="removeTag(tag)"
    >
      {{ tag }}
    </VChip>

    <VCombobox
      v-if="editing"
      ref="inputRef"
      :model-value="draft"
      :items="store.tags"
      placeholder="Add tag"
      variant="plain"
      density="compact"
      hide-details
      hide-no-data
      auto-select-first
      class="tag-input"
      @update:model-value="onDraft"
      @blur="onBlur"
      @keydown.esc.stop="stopEditing"
    />
    <VBtn
      v-else
      icon="mdi-plus"
      size="24"
      variant="text"
      class="tag-add-btn"
      title="Add tag"
      @click="startEditing"
    />
  </div>
</template>

<style scoped>
.tag-cell {
  min-height: 32px;
}

.tag-input {
  width: 140px;
  flex: 0 0 auto;
}

/* Keep the plain-variant combobox the same height as a chip row. */
.tag-input :deep(.v-field__input) {
  padding-top: 0;
  padding-bottom: 0;
  min-height: 32px;
  font-size: 0.875rem;
}

/* Subdued until the row is hovered so the column stays quiet. */
.tag-add-btn {
  opacity: 0.35;
  transition: opacity 0.15s;
}
.tag-cell:hover .tag-add-btn,
.tag-add-btn:focus-visible {
  opacity: 1;
}
</style>
