<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import { breaksLabel, columnChips, useLabel, verdictText, type ColumnModel } from '../../utils/moodPage';
import MoodLinesEditor from './MoodLinesEditor.vue';

/// One column of the Moods page's versions: a header (name, source icon,
/// Discard), the lines, and a footer with the verdict, chips and Use.
const props = defineProps<{
  col: ColumnModel;
  selected: boolean;
  currentLines: string[];
  moodName: string;
  moodNameOf: (id: string) => string;
  busy: boolean;
}>();
const emit = defineEmits<{
  select: [];
  rename: [name: string];
  discard: [];
  'update:text': [text: string];
  commit: [];
  fork: [text: string, line: number, caret: number];
  use: [];
  cancel: [];
  neverSuggest: [line: string];
  openSplit: [];
}>();

const SOURCE: Record<string, { icon: string; label: string }> = {
  typed: { icon: 'mdi-keyboard-outline', label: 'Typed' },
  arranged: { icon: 'mdi-shuffle-variant', label: 'Arranged' },
  suggested: { icon: 'mdi-lightbulb-on-outline', label: 'Suggested' },
  before: { icon: 'mdi-history', label: 'Before' },
};

const isCurrent = computed(() => props.col.kind === 'current');
const chips = computed(() =>
  props.col.review && !isCurrent.value && props.col.source
    ? columnChips(props.col.review, props.moodName, props.col.source)
    : props.col.review && isCurrent.value
      ? { ...columnChips(props.col.review, props.moodName, 'typed'), overlaps: [] }
      : null,
);
const verdict = computed(() => (props.col.review ? verdictText(props.col.review, isCurrent.value) : null));
const fmt = (v: number): string => v.toFixed(2);

const editor = ref<InstanceType<typeof MoodLinesEditor> | null>(null);
defineExpose({ focusLine: (i: number, caret: number) => editor.value?.focusLine(i, caret) });

// --- Rename (double-click the name) ---
const renaming = ref(false);
const nameDraft = ref('');
const nameField = ref<HTMLInputElement | null>(null);
async function startRename(): Promise<void> {
  if (props.col.kind !== 'version') return;
  nameDraft.value = props.col.name;
  renaming.value = true;
  await nextTick();
  nameField.value?.select();
}
function finishRename(save: boolean): void {
  if (!renaming.value) return;
  renaming.value = false;
  const name = nameDraft.value.trim();
  if (save && name && name !== props.col.name) emit('rename', name);
}

const splitKeepVerdict = computed(() => (props.col.split ? verdictText(props.col.split.keep, false) : ''));
const splitNewVerdict = computed(() => (props.col.split ? verdictText(props.col.split.new, false) : ''));
</script>

<template>
  <section
    class="version-col"
    :class="{ selected, current: isCurrent }"
    :aria-label="col.name"
    :data-col="col.key"
  >
    <header
      class="col-head"
      tabindex="0"
      :aria-pressed="selected"
      @click="emit('select')"
      @keydown.enter.self="emit('select')"
      @keydown.space.self.prevent="emit('select')"
    >
      <VIcon v-if="col.source && SOURCE[col.source]" size="small" class="text-medium-emphasis" :aria-label="SOURCE[col.source].label">
        {{ SOURCE[col.source].icon }}
        <VTooltip activator="parent" location="top" open-on-focus>{{ SOURCE[col.source].label }}</VTooltip>
      </VIcon>
      <VIcon v-else-if="col.kind === 'split'" size="small" class="text-medium-emphasis">mdi-call-split</VIcon>
      <input
        v-if="renaming"
        ref="nameField"
        v-model="nameDraft"
        class="name-input"
        aria-label="Version name"
        @click.stop
        @keydown.enter.prevent="finishRename(true)"
        @keydown.esc.prevent="finishRename(false)"
        @blur="finishRename(true)"
      >
      <span v-else class="col-name text-truncate" @dblclick.stop="startRename">{{ col.name }}</span>
      <VSpacer />
      <VProgressCircular v-if="col.evaluating" indeterminate size="14" width="2" class="mr-1" aria-label="Evaluating" />
      <VBtn
        v-if="!isCurrent"
        icon
        size="x-small"
        variant="text"
        aria-label="Discard"
        @click.stop="emit('discard')"
      >
        <VIcon size="small">mdi-close</VIcon>
        <VTooltip activator="parent" location="top" open-on-focus>Discard</VTooltip>
      </VBtn>
    </header>

    <!-- Arrange / Suggest still searching -->
    <div v-if="col.running" class="col-body running">
      <VProgressLinear indeterminate color="primary" class="mb-2" />
      <div class="text-body-2">{{ col.running === 'arrange' ? 'Arranging…' : 'Suggesting…' }}</div>
      <VBtn size="small" variant="text" class="mt-1 px-0" @click="emit('cancel')">Cancel</VBtn>
    </div>

    <!-- A split: the two halves stacked -->
    <div v-else-if="col.kind === 'split' && col.split" class="col-body">
      <div class="half-title text-caption text-medium-emphasis">{{ moodName }}</div>
      <div v-for="l in col.split.keep.lines" :key="`k${l}`" class="ro-line">{{ l }}</div>
      <div class="text-caption mt-1 mb-2 verdict">{{ splitKeepVerdict }}</div>
      <div class="half-title text-caption text-medium-emphasis">New</div>
      <div v-for="l in col.split.new.lines" :key="`n${l}`" class="ro-line">{{ l }}</div>
      <div class="text-caption mt-1 verdict">{{ splitNewVerdict }}</div>
    </div>

    <div v-else class="col-body" @click="emit('select')">
      <MoodLinesEditor
        ref="editor"
        :text="col.text"
        :current="currentLines"
        :fork="isCurrent"
        :label="col.name"
        :disabled="busy"
        @update:text="(t) => emit('update:text', t)"
        @commit="emit('commit')"
        @fork="(t, l, c) => emit('fork', t, l, c)"
        @never-suggest="(l) => emit('neverSuggest', l)"
      />
    </div>

    <footer class="col-foot" aria-live="polite">
      <template v-if="col.kind === 'split' && col.split">
        <VBtn
          size="small"
          variant="tonal"
          :color="col.regressionCount > 0 ? 'error' : 'primary'"
          :disabled="busy"
          @click="emit('use')"
        >
          Use split
        </VBtn>
      </template>
      <template v-else-if="!col.running">
        <div v-if="col.error" class="text-caption text-error">{{ col.error }}</div>
        <div v-else-if="col.review" class="verdict-block" :class="{ stale: col.stale }">
          <div class="verdict text-body-2">
            <span v-if="col.regressionCount > 0" class="breaks">✗ {{ breaksLabel(col.regressionCount) }} · </span>
            <span>{{ verdict }}</span>
          </div>
          <div v-if="chips" class="chips">
            <span v-if="chips.shaky" class="chip" tabindex="0">
              Shaky
              <VTooltip activator="parent" location="top" open-on-focus>
                <template v-if="col.review.loo.total > 0">
                  {{ col.review.loo.held }} of {{ col.review.loo.total }} flags hold when each is left out
                </template>
                <template v-else>Fewer than 5 flags</template>
              </VTooltip>
            </span>
            <span v-for="o in chips.overlaps" :key="o.moodId" class="chip" tabindex="0">
              Overlaps {{ moodNameOf(o.moodId) }}
              <VTooltip activator="parent" location="top" open-on-focus>{{ fmt(o.before) }} → {{ fmt(o.after) }}</VTooltip>
            </span>
            <span v-if="chips.readsLike" class="chip" tabindex="0">
              Reads like: {{ chips.readsLike }}
              <VTooltip activator="parent" location="top" open-on-focus>
                Name ideas: {{ col.review.names.slice(0, 3).map((n) => n.word).join(', ') }}
              </VTooltip>
            </span>
          </div>
          <button v-if="col.splitOffer" type="button" class="split-note text-caption" @click="emit('openSplit')">
            Fits two moods better
          </button>
        </div>
        <div v-else-if="col.evaluating" class="text-caption text-medium-emphasis">…</div>
        <VBtn
          v-if="!isCurrent && col.text.trim() !== ''"
          size="small"
          variant="tonal"
          class="mt-1"
          :color="col.regressionCount > 0 ? 'error' : 'primary'"
          :disabled="busy || col.error !== null"
          @click="emit('use')"
        >
          {{ useLabel(col.regressionCount) }}
        </VBtn>
      </template>
    </footer>
  </section>
</template>

<style scoped>
.version-col {
  flex: 0 0 280px;
  width: 280px;
  display: flex;
  flex-direction: column;
  min-height: 0;
  max-height: 100%;
  border: 1px solid rgba(var(--v-theme-on-surface), 0.12);
  border-top: 3px solid transparent;
  border-radius: 6px;
  background: rgb(var(--v-theme-surface));
}
.version-col.selected {
  border-top-color: rgb(var(--v-theme-primary));
}
.version-col.current {
  position: sticky;
  left: 0;
  z-index: 2;
}
.col-head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 36px;
  padding: 2px 4px 2px 10px;
  cursor: pointer;
  font-weight: 600;
  font-size: 0.875rem;
  border-bottom: 1px solid rgba(var(--v-theme-on-surface), 0.08);
  outline: none;
}
.col-head:focus-visible {
  box-shadow: inset 0 0 0 2px rgba(var(--v-theme-primary), 0.6);
}
.col-name {
  min-width: 0;
}
.name-input {
  flex: 1 1 auto;
  min-width: 0;
  font: inherit;
  color: inherit;
  border-bottom: 1px solid rgb(var(--v-theme-primary));
  outline: none;
}
.col-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 4px 4px 4px 4px;
}
.col-body.running {
  padding: 12px;
}
.ro-line {
  font-size: 0.875rem;
  padding: 2px 4px 2px 18px;
  text-indent: -12px;
}
.half-title {
  padding: 2px 6px;
}
.col-foot {
  padding: 6px 10px 8px;
  border-top: 1px solid rgba(var(--v-theme-on-surface), 0.08);
  min-height: 44px;
}
.verdict-block.stale {
  opacity: 0.45;
}
.verdict {
  font-variant-numeric: tabular-nums;
}
.breaks {
  color: #ef5350;
  font-weight: 700;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 4px;
}
.chip {
  font-size: 0.75rem;
  padding: 1px 8px;
  border-radius: 10px;
  background: rgba(var(--v-theme-warning), 0.14);
  cursor: default;
  outline: none;
}
.chip:focus-visible {
  box-shadow: 0 0 0 2px rgba(var(--v-theme-primary), 0.6);
}
.split-note {
  appearance: none;
  background: transparent;
  border: 0;
  display: block;
  margin-top: 4px;
  color: rgb(var(--v-theme-primary));
  text-decoration: underline;
  min-height: 28px;
}
</style>
