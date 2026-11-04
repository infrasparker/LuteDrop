<script setup lang="ts">
import { ref, nextTick } from 'vue';
import { useAppStore } from '../stores/appStore';

/// Tag chips for one library row. Clicking the cell (or its "+") swaps the
/// static chips for the same multi-select chip combobox the editor dialog
/// uses: type, Enter adds a chip, Backspace removes the last one, picking a
/// suggestion adds it. Clicking away returns to the static chips.
const props = defineProps<{
  entryId: string;
  tags: string[];
}>();

const store = useAppStore();

const editing = ref(false);
const inputRef = ref<{ focus: () => void } | null>(null);

async function persist(tags: string[]): Promise<void> {
  const clean = [...new Set(tags.map((t) => t.trim()).filter((t) => t.length > 0))];
  await store.command('update_library_entry_tags', { entryId: props.entryId, tags: clean });
}

function removeTag(tag: string): void {
  persist(props.tags.filter((t) => t !== tag));
}

async function startEditing(): Promise<void> {
  if (editing.value) return;
  editing.value = true;
  await nextTick();
  inputRef.value?.focus();
}

function stopEditing(): void {
  editing.value = false;
}

// VCombobox (multiple) only emits when a chip is added or removed, never
// per keystroke, so every emit is a real change worth saving.
function onChange(tags: string[]): void {
  persist(tags);
}
</script>

<template>
  <div
    class="d-flex align-center flex-wrap ga-1 tag-cell"
    :class="{ 'tag-cell--editing': editing }"
    @click.stop="startEditing"
  >
    <VCombobox
      v-if="editing"
      ref="inputRef"
      :model-value="tags"
      :items="store.tags"
      placeholder="Add tag"
      variant="plain"
      density="compact"
      hide-details
      hide-no-data
      multiple
      chips
      closable-chips
      class="tag-input"
      @update:model-value="onChange"
      @blur="stopEditing"
      @keydown.esc.stop="stopEditing"
    />
    <template v-else>
      <VChip
        v-for="tag in tags"
        :key="tag"
        size="small"
        closable
        @click:close.stop="removeTag(tag)"
      >
        {{ tag }}
      </VChip>
      <VBtn
        icon="mdi-plus"
        size="24"
        variant="text"
        class="tag-add-btn"
        title="Add tag"
      />
    </template>
  </div>
</template>

<style scoped>
.tag-cell {
  min-height: 32px;
  cursor: text;
}

.tag-input {
  flex: 1 1 auto;
  min-width: 160px;
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
.tag-cell:hover .tag-add-btn {
  opacity: 1;
}
</style>
