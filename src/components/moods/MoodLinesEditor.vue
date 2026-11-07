<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { lineDiff, minusLinesLabel } from '../../utils/moodPage';

/// The lines of one version column, one prompt per line, each its own
/// wrapping field (hanging indent). Enter splits a line, Backspace at the
/// start of a line joins it to the one above, pasted text splits on new
/// lines. Lines not in Current carry an accent bar; Current's lines missing
/// here show as a muted "−n lines" line at the end. Each line has a ⋯ menu
/// (Remove, Never suggest).
///
/// `fork` (the Current column): the lines are read-only; typing emits
/// `fork` with the edited text, and the field goes back to the saved lines.
const props = defineProps<{
  text: string;
  /// Current's lines, for the accent bars and the missing-lines footer.
  current: string[];
  fork?: boolean;
  disabled?: boolean;
  /// Never suggest is off (the preferences could not be read).
  neverSuggestOff?: boolean;
  label: string;
}>();
const emit = defineEmits<{
  'update:text': [text: string];
  /// A line was finished (Enter, or focus left the column).
  commit: [];
  fork: [text: string, line: number, caret: number];
  neverSuggest: [line: string];
}>();

const rows = ref<string[]>([]);
watch(
  () => props.text,
  (t) => {
    const next = t.length === 0 ? [] : t.split('\n');
    if (next.join('\n') !== rows.value.join('\n')) rows.value = next;
  },
  { immediate: true },
);

const diff = computed(() => lineDiff(rows.value.map((r) => r.trim()), props.current));
const root = ref<HTMLElement | null>(null);

function fields(): HTMLTextAreaElement[] {
  return Array.from(root.value?.querySelectorAll<HTMLTextAreaElement>('textarea.line-input') ?? []);
}

async function focusLine(i: number, caret: number): Promise<void> {
  await nextTick();
  const el = fields()[i];
  if (!el) return;
  el.focus();
  const at = Math.min(caret, el.value.length);
  el.setSelectionRange(at, at);
}
defineExpose({ focusLine });

/// Sends the new rows up (or, forking, asks for a new version with them).
function change(next: string[], line: number, caret: number): void {
  if (props.fork) {
    emit('fork', next.join('\n'), line, caret);
    // Current stays as saved.
    rows.value = props.text.length === 0 ? [] : props.text.split('\n');
    return;
  }
  rows.value = next;
  emit('update:text', next.join('\n'));
  void focusLine(line, caret);
}

function onInput(i: number, e: Event): void {
  const el = e.target as HTMLTextAreaElement;
  const caret = el.selectionStart ?? el.value.length;
  const parts = el.value.split('\n');
  const next = [...rows.value];
  if (i >= next.length) next.push(...parts);
  else next.splice(i, 1, ...parts);
  // A paste or a typed new line moves the caret to the last part.
  const line = i + parts.length - 1;
  const lastCaret = parts.length > 1 ? caret - (el.value.length - parts[parts.length - 1].length) : caret;
  if (props.fork) {
    change(next, line, lastCaret);
    // The field already shows the typing; put the saved line back.
    el.value = rows.value[i] ?? '';
    return;
  }
  rows.value = next;
  emit('update:text', next.join('\n'));
  if (parts.length > 1) void focusLine(line, lastCaret);
}

function onKeydown(i: number, e: KeyboardEvent): void {
  const el = e.target as HTMLTextAreaElement;
  if (e.key === 'Enter' && !e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey) {
    e.preventDefault();
    const at = el.selectionStart ?? el.value.length;
    const text = rows.value[i] ?? el.value;
    const next = [...rows.value];
    if (i >= next.length) {
      if (text.trim() === '') return;
      next.push(text.slice(0, at), text.slice(at));
    } else {
      next.splice(i, 1, text.slice(0, at), text.slice(at));
    }
    change(next, i + 1, 0);
    emit('commit');
  } else if (e.key === 'Backspace' && i > 0 && i < rows.value.length && el.selectionStart === 0 && el.selectionEnd === 0) {
    e.preventDefault();
    const next = [...rows.value];
    const prevLen = next[i - 1].length;
    next.splice(i - 1, 2, next[i - 1] + next[i]);
    change(next, i - 1, prevLen);
  } else if (e.key === 'ArrowUp' && i > 0 && el.selectionStart === 0) {
    e.preventDefault();
    void focusLine(i - 1, Number.MAX_SAFE_INTEGER);
  } else if (e.key === 'ArrowDown' && el.selectionStart === el.value.length) {
    e.preventDefault();
    void focusLine(i + 1, 0);
  }
}

function removeLine(i: number): void {
  const next = [...rows.value];
  next.splice(i, 1);
  change(next, Math.max(0, i - 1), 0);
  emit('commit');
}

function onFocusOut(e: FocusEvent): void {
  if (!root.value?.contains(e.relatedTarget as Node | null)) emit('commit');
}
</script>

<template>
  <div ref="root" class="lines-editor" role="group" :aria-label="label" @focusout="onFocusOut">
    <div v-for="(line, i) in rows" :key="i" class="line" :class="{ added: diff.added[i] && line.trim() !== '' }">
      <textarea
        class="line-input"
        rows="1"
        :value="line"
        :disabled="disabled"
        :aria-label="`${label}, line ${i + 1}`"
        spellcheck="false"
        @input="onInput(i, $event)"
        @keydown="onKeydown(i, $event)"
      />
      <VMenu v-if="line.trim() !== ''" location="bottom end">
        <template #activator="{ props: menuProps }">
          <VBtn
            v-bind="menuProps"
            class="line-menu"
            icon
            size="x-small"
            variant="text"
            :disabled="disabled"
            :aria-label="`Line ${i + 1} actions`"
          >
            <VIcon size="small">mdi-dots-horizontal</VIcon>
          </VBtn>
        </template>
        <VList density="compact">
          <VListItem v-if="!fork" title="Remove" @click="removeLine(i)" />
          <VListItem title="Never suggest" :disabled="neverSuggestOff" @click="emit('neverSuggest', line.trim())">
            <VTooltip activator="parent" location="end" open-delay="300">Arrange and Suggest leave these words out</VTooltip>
          </VListItem>
        </VList>
      </VMenu>
    </div>
    <div v-if="!fork" class="line">
      <textarea
        class="line-input"
        rows="1"
        value=""
        placeholder="Add a line"
        :disabled="disabled"
        :aria-label="`${label}, new line`"
        spellcheck="false"
        @input="onInput(rows.length, $event)"
        @keydown="onKeydown(rows.length, $event)"
      />
    </div>
    <div v-if="diff.missing.length > 0" class="missing text-medium-emphasis" tabindex="0">
      {{ minusLinesLabel(diff.missing.length) }}
      <VTooltip activator="parent" location="bottom" open-on-focus>
        <div v-for="m in diff.missing" :key="m">{{ m }}</div>
      </VTooltip>
    </div>
  </div>
</template>

<style scoped>
.lines-editor {
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.line {
  position: relative;
  display: flex;
  align-items: flex-start;
  border-left: 3px solid transparent;
  border-radius: 2px;
}
.line.added {
  border-left-color: rgb(var(--v-theme-primary));
}
.line-input {
  flex: 1 1 auto;
  min-width: 0;
  resize: none;
  overflow: hidden;
  field-sizing: content;
  min-height: 28px;
  padding: 4px 4px 4px 18px;
  text-indent: -12px;
  font: inherit;
  font-size: 0.875rem;
  line-height: 1.35;
  color: inherit;
  background: transparent;
  border: 0;
  outline: none;
  border-radius: 4px;
}
.line-input:focus {
  background: rgba(var(--v-theme-on-surface), 0.06);
}
.line-input::placeholder {
  color: rgba(var(--v-theme-on-surface), 0.38);
}
.line-menu {
  min-width: 28px;
  min-height: 28px;
  opacity: 0;
  flex: 0 0 auto;
}
.line:hover .line-menu,
.line:focus-within .line-menu,
.line-menu[aria-expanded='true'] {
  opacity: 1;
}
.missing {
  font-size: 0.8125rem;
  padding: 4px 4px 4px 9px;
  min-height: 28px;
  cursor: default;
  width: fit-content;
}
</style>
