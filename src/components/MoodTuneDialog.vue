<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { AppState, FlagBand, FlagKind, MoodFlag, TuneMode, TuneResult, TuneReview, TuneSong } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { flagLabel, FLAG_BANDS, reviewTargetLabel } from '../utils/moodFlags';
import {
  acceptLabel,
  albumOf,
  brokenFresh,
  canAddVersion,
  DEFAULT_TUNE_MODE,
  flaggedEntries,
  focusHint,
  focusNote,
  initialTuneMode,
  holdingEntryIds,
  isChangedSinceTune,
  leaveBand,
  looNote,
  nameNote,
  orderedFlags,
  overlapRises,
  parseExtraWords,
  parseTypedLines,
  proposalLabel,
  rankLabel,
  reviewTargetIsHigh,
  shakyNote,
  sourceCaption,
  splitAcceptLabel,
  splitBrokenFresh,
  splitName,
  topChanges,
  topRanks,
  TUNE_MODES,
  tuneTargetCount,
  tuneTargetIds,
  versionCandidates,
  versionProblem,
} from '../utils/moodTune';
import { newMoodId, WORD_BANK } from '../utils/moodWords';
import ConfirmDialog from './ConfirmDialog.vue';
import PromptDialog from './PromptDialog.vue';

/// Tune one mood: review word lines against the songs flagged on it before
/// accepting. Three modes (the switch at the top): Compare (the default)
/// reviews your own versions of the lines as written; Arrange my words
/// searches combinations of only the words you type and the current lines;
/// Suggest also searches the word bank, other moods' words, the phrase list
/// and the analyzer's built-in vocabulary. Top to bottom:
///
/// 0. The mood's Flags list (collapsible): change or remove each flag.
/// 1. Mode switch, flag count, the mode's inputs (versions, or words to try
///    and words to leave out) and the run button.
/// 2. Proposed lines (or versions) beside the current ones, each line with
///    where its words came from, and a switcher between them.
/// 3. The flagged songs before → after against their band; broken fresh
///    flags first and loudest, broken stale flags with Keep / Update / Delete.
/// 4. The old top 15 beside the new one.
/// 5. Biggest unflagged movers and the fire count.
/// 6. Notes, never gates: leave-one-out, name ideas, overlap rises,
///    unsatisfied and skipped flags, and a split proposal with its own Accept.
/// 7. Reject / Accept. Accept asks once when fresh flags leave their band.
///
/// Nothing is saved until Accept. After an accept the dialog closes and
/// emits `accepted` (with the new mood's id after a split) so the Moods panel
/// can offer "Recalibrate now?".
const open = defineModel<boolean>({ required: true });
const props = defineProps<{
  moodId: string | null;
  /// Open with the Flags list expanded and scrolled into view.
  showFlags?: boolean;
  /// Run Tune focused on this song's flag ("Tune around this flag"), rather
  /// than on the newest flag.
  focusEntryId?: string | null;
}>();
const emit = defineEmits<{ accepted: [newMoodId: string | null] }>();

const store = useAppStore();

const mood = computed(() => store.moodVocabulary.moods.find((m) => m.id === props.moodId) ?? null);
const moodName = (id: string): string => store.moodVocabulary.moods.find((m) => m.id === id)?.name ?? id;

// --- Flags list ---

const flags = computed(() =>
  props.moodId === null
    ? []
    : flaggedEntries(store.libraryEntries, props.moodId).sort((a, b) => a.entry.name.localeCompare(b.entry.name)),
);
/// Songs Tune grades against: flags plus hand-set levels without a flag.
const targets = computed(() => (props.moodId === null ? 0 : tuneTargetCount(store.libraryEntries, props.moodId)));
const handSet = computed(() => targets.value - flags.value.length);
const flagsOpen = ref(false);
const flagsSection = ref<HTMLElement | null>(null);

/// The flag on one song for this mood right now (it may have changed since
/// Tune ran), or null.
const currentFlag = (entryId: string): MoodFlag | null =>
  store.findLibraryEntry(entryId)?.moods.find((m) => m.mood === props.moodId)?.flag ?? null;
/// The level the user set on one song for this mood, or null. Without a
/// flag, Tune reads it as a band flag.
const currentLevel = (entryId: string): number | null =>
  store.findLibraryEntry(entryId)?.moods.find((m) => m.mood === props.moodId)?.user ?? null;

/// Stale flags acted on (Keep / Update / Delete) since the review was made:
/// the review no longer reflects them until Tune runs again.
const touched = reactive(new Set<string>());

function setFlag(entryId: string, kind: FlagKind, band: FlagBand | null): void {
  touched.add(entryId);
  void store.command('set_mood_flag', { entryId, mood: props.moodId, kind, band });
}
function clearFlag(entryId: string): void {
  touched.add(entryId);
  void store.command('clear_mood_flag', { entryId, mood: props.moodId });
}
function keepFlag(entryId: string): void {
  touched.add(entryId);
  void store.command('keep_mood_flag', { entryId, mood: props.moodId });
}

// --- Running Tune ---

const mode = ref<TuneMode>(DEFAULT_TUNE_MODE);
/// The run button's label for the current mode.
const runLabel = computed(() => TUNE_MODES.find((m) => m.mode === mode.value)?.run ?? 'Tune');
/// Compare mode's versions, one textarea each (one prompt per line).
const versions = ref<string[]>(['']);
const versionLines = computed(() => versionCandidates(versions.value));
/// Versions over the analyzer's limits, said plainly before anything is sent.
const versionError = computed(() => (mode.value === 'compare' ? versionProblem(versions.value) : null));
function addVersion(): void {
  if (canAddVersion(versions.value.length)) versions.value.push(mood.value?.prompts.join('\n') ?? '');
}
function removeVersion(i: number): void {
  versions.value.splice(i, 1);
  if (versions.value.length === 0) versions.value.push('');
}
/// Arrange mode's typed words and lines, one per line, kept whole.
const typedText = ref('');
/// Suggest mode's "Extra words to try".
const extraText = ref('');
/// "Words to leave out", remembered per mood while the app runs.
const excludeText = ref('');
const excludeByMood = new Map<string, string>();
watch(excludeText, (text) => {
  if (props.moodId !== null) excludeByMood.set(props.moodId, text);
});
/// Which flag the next run focuses on: null for the newest flag (the
/// backend's default), an entry id for a picked one, '' for none.
const focusRequest = ref<string | null>(null);
const running = ref(false);
const tuneError = ref<string | null>(null);
const result = ref<TuneResult | null>(null);
/// Which proposal is shown.
const selected = ref(0);
/// Bumped on every run and on reopening, so a slow run that finishes after
/// the dialog moved on is dropped.
let runToken = 0;

async function runTune(): Promise<void> {
  if (props.moodId === null || versionError.value !== null) return;
  const token = ++runToken;
  running.value = true;
  tuneError.value = null;
  acceptError.value = null;
  try {
    const r = await invoke<TuneResult>('tune_mood', {
      moodId: props.moodId,
      mode: mode.value,
      extraWords: mode.value === 'arrange' ? parseTypedLines(typedText.value) : parseExtraWords(extraText.value),
      bank: mode.value === 'suggest' ? WORD_BANK : [],
      focusEntryId: focusRequest.value,
      excludeWords: mode.value === 'compare' ? [] : parseExtraWords(excludeText.value),
      candidates: mode.value === 'compare' ? versionLines.value : [],
    });
    if (token !== runToken) return;
    result.value = r;
    selected.value = 0;
    touched.clear();
  } catch (e) {
    if (token === runToken) tuneError.value = String(e);
  } finally {
    if (token === runToken) running.value = false;
  }
}

/// "Focusing on <song> (...)" for the last run, or null.
const focusLine = computed(() => {
  const id = result.value?.focusEntryId ?? null;
  const name = id === null ? null : (store.findLibraryEntry(id)?.name ?? null);
  return focusNote(id, name, focusRequest.value !== null && focusRequest.value !== '');
});

/// Runs again with no focus at all.
function clearFocus(): void {
  focusRequest.value = '';
  void runTune();
}

// --- The review shown ---

/// The selected proposal, or the current lines' own review when nothing
/// beat them.
const review = computed<TuneReview | null>(() => {
  const r = result.value;
  if (!r) return null;
  return r.proposals[selected.value] ?? r.baseline;
});
const hasProposal = computed(() => (result.value?.proposals.length ?? 0) > 0);

const shownFlags = computed(() => (review.value ? orderedFlags(review.value) : []));
const changes = computed(() => (review.value ? topChanges(review.value) : { joined: new Set<string>(), dropped: new Set<string>() }));
const ranks = computed(() => (review.value ? topRanks(review.value) : new Map<string, { before: number | null; after: number | null }>()));
/// Songs Tune graded against: flagged, or with a hand-set level.
const flaggedIds = computed(() =>
  props.moodId === null ? new Set<string>() : tuneTargetIds(store.libraryEntries, props.moodId),
);
/// A song that drops out of the top while flagged (or hand-set) to sit high:
/// shown in red.
function droppedHigh(entryId: string): boolean {
  if (!changes.value.dropped.has(entryId)) return false;
  return reviewTargetIsHigh(currentFlag(entryId), currentLevel(entryId));
}
const rises = computed(() => (review.value ? overlapRises(review.value.overlap) : []));
const songNames = (songs: TuneSong[]): string => songs.map((s) => s.name).join(', ');

const fmt = (v: number): string => v.toFixed(2);

/// Where a song's flag (or hand-set level) stands now, for the review's flag rows.
function flagNow(entryId: string): string {
  return reviewTargetLabel(currentFlag(entryId), currentLevel(entryId));
}

// --- Accept ---

const accepting = ref(false);
const acceptError = ref<string | null>(null);
const confirmAccept = ref(false);
const changedSinceTune = computed(() => acceptError.value !== null && isChangedSinceTune(acceptError.value));

function onAccept(): void {
  if (!review.value || !hasProposal.value || running.value) return;
  if (brokenFresh(review.value) > 0) confirmAccept.value = true;
  else void accept(() => acceptLinesArgs(review.value!), 'accept_tuned_lines');
}

function acceptLinesArgs(r: TuneReview): Record<string, unknown> {
  return {
    moodId: props.moodId,
    linesTunedFrom: result.value!.linesTunedFrom,
    lines: r.lines,
    holdingEntryIds: holdingEntryIds(r),
  };
}

/// Runs an accept command; on success the store takes the new state and the
/// dialog closes, on failure the error stays on screen (with "Run Tune
/// again" when the lines changed meanwhile).
async function accept(
  args: () => Record<string, unknown>,
  cmd: string,
  newMoodId: string | null = null,
): Promise<void> {
  accepting.value = true;
  acceptError.value = null;
  try {
    const state = await invoke<AppState>(cmd, args());
    store.hydrate(state);
    open.value = false;
    emit('accepted', newMoodId);
  } catch (e) {
    acceptError.value = String(e);
  } finally {
    accepting.value = false;
  }
}

// --- Split ---

const askSplitName = ref(false);
const confirmSplit = ref(false);

/// The split's own accept: like a proposal's, it asks once when fresh flags
/// leave their band (over both halves), then asks for the new mood's name.
function onAcceptSplit(): void {
  const split = result.value?.split;
  if (!split || running.value) return;
  if (splitBrokenFresh(split) > 0) confirmSplit.value = true;
  else askSplitName.value = true;
}
const splitNameInitial = computed(() => (result.value?.split ? splitName(result.value.split.new.names) : ''));

function acceptSplit(newName: string): void {
  const r = result.value;
  const split = r?.split;
  if (!r || !split) return;
  const taken = new Set(store.moodVocabulary.moods.map((m) => m.id));
  let newId = newMoodId();
  while (taken.has(newId)) newId = newMoodId();
  void accept(
    () => ({
      moodId: props.moodId,
      linesTunedFrom: r.linesTunedFrom,
      keepLines: split.keep.lines,
      newId,
      newName,
      newLines: split.new.lines,
      moveEntryIds: split.newSongs.map((s) => s.entryId),
      keepHoldingEntryIds: holdingEntryIds(split.keep),
      newHoldingEntryIds: holdingEntryIds(split.new),
    }),
    'accept_tuned_split',
    newId,
  );
}

// Each opening starts clean: no result, the Flags list open when asked for.
watch(open, async (isOpen) => {
  if (!isOpen) return;
  runToken++;
  running.value = false;
  result.value = null;
  tuneError.value = null;
  acceptError.value = null;
  selected.value = 0;
  touched.clear();
  focusRequest.value = props.focusEntryId ?? null;
  mode.value = initialTuneMode(props.focusEntryId);
  versions.value = [mood.value?.prompts.join('\n') ?? ''];
  excludeText.value = props.moodId === null ? '' : (excludeByMood.get(props.moodId) ?? '');
  flagsOpen.value = !!props.showFlags;
  if (props.showFlags) {
    await nextTick();
    flagsSection.value?.scrollIntoView({ block: 'start' });
  }
}, { immediate: true });
</script>

<template>
  <VDialog v-model="open" max-width="980" scrollable>
    <VCard :title="mood ? `Tune ${mood.name}` : 'Tune'" prepend-icon="mdi-tune-vertical">
      <VCardText v-if="mood">
        <!-- 0. Flags list -->
        <section ref="flagsSection" class="mb-4">
          <VBtn
            variant="text"
            size="small"
            class="px-1"
            :prepend-icon="flagsOpen ? 'mdi-chevron-down' : 'mdi-chevron-right'"
            @click="flagsOpen = !flagsOpen"
          >
            Flags on {{ mood.name }} ({{ flags.length }})
          </VBtn>
          <div v-if="flagsOpen" class="mt-1">
            <div v-if="flags.length === 0" class="text-caption text-medium-emphasis">
              No flags yet. Flag a bar that surprises you from a song's mood profile.
            </div>
            <div v-for="f in flags" :key="f.entry.id" class="flag-row">
              <VIcon size="small" color="warning" :class="{ 'flag-stale': f.flag.stale }">mdi-flag</VIcon>
              <span class="text-truncate">
                {{ f.entry.name }}
                <span class="text-caption text-medium-emphasis">· {{ albumOf(f.entry.filePath) }}</span>
              </span>
              <span class="text-caption" :class="{ 'text-medium-emphasis': f.flag.stale }">{{ flagLabel(f.flag) }}</span>
              <VMenu location="bottom end">
                <template #activator="{ props: menuProps }">
                  <VBtn v-bind="menuProps" size="x-small" variant="text" :aria-label="`Change the flag on ${f.entry.name}`">
                    Change
                  </VBtn>
                </template>
                <VList density="compact">
                  <VListItem
                    v-for="b in FLAG_BANDS"
                    :key="b.band"
                    :title="b.label"
                    @click="setFlag(f.entry.id, 'band', b.band)"
                  />
                  <VDivider />
                  <VListItem title="Higher" @click="setFlag(f.entry.id, 'higher', null)" />
                  <VListItem title="Lower" @click="setFlag(f.entry.id, 'lower', null)" />
                </VList>
              </VMenu>
              <VBtn size="x-small" variant="text" @click="clearFlag(f.entry.id)">Remove</VBtn>
            </div>
          </div>
        </section>

        <!-- 1. Before you run -->
        <section class="mb-4">
          <div class="text-body-2 mb-1">
            {{ flags.length }} flag{{ flags.length === 1 ? '' : 's' }} on {{ mood.name }}
            <template v-if="handSet > 0">
              and {{ handSet }} hand-set level{{ handSet === 1 ? '' : 's' }} (they count as flags)
            </template>
            <span v-if="shakyNote(targets)" class="text-warning"> · {{ shakyNote(targets) }}</span>
          </div>
          <VBtnToggle
            v-model="mode"
            mandatory
            density="compact"
            variant="outlined"
            divided
            color="primary"
            class="mb-2"
            :disabled="running"
          >
            <VBtn v-for="m in TUNE_MODES" :key="m.mode" :value="m.mode" size="small">{{ m.label }}</VBtn>
          </VBtnToggle>
          <div v-if="mode === 'compare'">
            <div class="text-caption text-medium-emphasis mb-1">
              Write versions of the lines, one prompt per line, and see how each does on your flags.
            </div>
            <div class="versions">
              <div v-for="(_, i) in versions" :key="i" class="version">
                <VTextarea
                  v-model="versions[i]"
                  :label="`Version ${i + 1}`"
                  variant="outlined"
                  density="compact"
                  rows="3"
                  auto-grow
                  hide-details
                  :disabled="running"
                />
                <VBtn
                  v-if="versions.length > 1"
                  size="x-small"
                  variant="text"
                  :disabled="running"
                  @click="removeVersion(i)"
                >
                  Remove
                </VBtn>
              </div>
            </div>
            <VBtn
              v-if="canAddVersion(versions.length)"
              size="small"
              variant="text"
              prepend-icon="mdi-plus"
              class="mt-1"
              :disabled="running"
              @click="addVersion"
            >
              Add version
            </VBtn>
          </div>
          <div class="d-flex align-center ga-3 mt-1">
            <VTextarea
              v-if="mode === 'arrange'"
              v-model="typedText"
              label="Your words and lines (one per line; commas join words in a line)"
              variant="outlined"
              density="compact"
              rows="2"
              auto-grow
              hide-details
              :disabled="running"
            />
            <VTextField
              v-if="mode === 'suggest'"
              v-model="extraText"
              label="Extra words to try (comma-separated, optional)"
              variant="outlined"
              density="compact"
              hide-details
              :disabled="running"
            />
            <VTextField
              v-if="mode !== 'compare'"
              v-model="excludeText"
              label="Words to leave out (comma-separated)"
              variant="outlined"
              density="compact"
              hide-details
              :disabled="running"
            />
            <VSpacer v-if="mode === 'compare'" />
            <VBtn
              color="primary"
              variant="tonal"
              prepend-icon="mdi-tune-vertical"
              :loading="running"
              :disabled="targets === 0 || (mode === 'compare' && (versionLines.length === 0 || versionError !== null))"
              @click="runTune"
            >
              {{ runLabel }}
            </VBtn>
          </div>
          <div v-if="versionError" class="text-caption text-error mt-1">{{ versionError }}</div>
          <div v-if="targets === 0" class="text-caption text-medium-emphasis mt-1">
            Flag a few songs on this mood first
          </div>
          <div v-if="running" class="mt-2">
            <VProgressLinear indeterminate color="primary" />
            <div class="text-caption text-medium-emphasis mt-1">
              {{ mode === 'compare' ? 'Reviewing your versions.' : 'Searching word lines; this can take a minute.' }}
            </div>
          </div>
          <VAlert v-if="tuneError" type="error" variant="tonal" density="compact" class="mt-2">{{ tuneError }}</VAlert>
          <div v-if="result && focusLine" class="d-flex align-center ga-1 mt-2 text-body-2">
            <VIcon size="small" color="primary">mdi-target</VIcon>
            {{ focusLine }}
            <VBtn
              size="x-small"
              variant="text"
              icon
              :disabled="running"
              aria-label="Run again without a focus"
              @click="clearFocus"
            >
              <VIcon size="small">mdi-close</VIcon>
              <VTooltip activator="parent" location="top">Run again without a focus</VTooltip>
            </VBtn>
            <span v-if="focusHint(result.mode)" class="text-caption text-medium-emphasis">
              · {{ focusHint(result.mode) }}
            </span>
          </div>
        </section>

        <template v-if="result && review">
          <VDivider class="mb-3" />

          <!-- 2. Proposed lines beside the current lines -->
          <section class="mb-4">
            <VAlert v-if="!hasProposal" type="info" variant="tonal" density="compact" class="mb-2">
              Nothing beat the current lines. Below is how the current lines do on your flags.
            </VAlert>
            <VBtnToggle
              v-if="result.proposals.length > 1"
              v-model="selected"
              mandatory
              density="compact"
              variant="outlined"
              divided
              class="mb-2"
            >
              <VBtn v-for="(_, i) in result.proposals" :key="i" :value="i" size="small">
                {{ proposalLabel(result.mode, i) }}
              </VBtn>
            </VBtnToggle>
            <div class="two-col">
              <div>
                <div class="text-subtitle-2">Current lines</div>
                <pre class="lines">{{ result.linesTunedFrom.join('\n') }}</pre>
              </div>
              <div v-if="hasProposal">
                <div class="text-subtitle-2">
                  {{ result.mode === 'compare' ? proposalLabel('compare', selected) : 'Proposed lines' }}
                </div>
                <div class="lines">
                  <div v-for="(line, i) in review.lines" :key="i" class="proposed-line">
                    {{ line }}
                    <div v-if="sourceCaption(review.lineSources[i])" class="text-caption text-medium-emphasis">
                      {{ sourceCaption(review.lineSources[i]) }}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </section>

          <!-- 3. Flagged songs before → after against their band -->
          <section class="mb-4">
            <div class="text-subtitle-2 mb-1">Your flagged songs</div>
            <VAlert
              v-if="hasProposal && brokenFresh(review) > 0"
              type="error"
              variant="tonal"
              density="compact"
              class="mb-2"
            >
              {{ leaveBand(brokenFresh(review)) }} with these lines.
            </VAlert>
            <div
              v-for="o in shownFlags"
              :key="o.song.entryId"
              class="outcome-row"
              :class="{ 'is-broken': !o.holds && o.fresh, 'is-broken-stale': !o.holds && !o.fresh }"
            >
              <VIcon size="small" :color="o.holds ? 'success' : o.fresh ? 'error' : 'warning'">
                {{ o.holds ? 'mdi-check' : 'mdi-alert' }}
              </VIcon>
              <span class="text-truncate">
                {{ o.song.name }}
                <span class="text-caption text-medium-emphasis">· {{ o.song.album }}</span>
                <span v-if="o.focus" class="tag text-primary">focus</span>
              </span>
              <span class="text-caption">{{ flagNow(o.song.entryId) }}</span>
              <span class="bars">{{ fmt(o.before) }} → {{ fmt(o.after) }}</span>
              <span class="text-caption text-no-wrap">
                <template v-if="o.holds">holds</template>
                <template v-else>leaves its band</template>
              </span>
              <div
                v-if="!o.holds && !o.fresh && touched.has(o.song.entryId)"
                class="stale-actions text-caption text-medium-emphasis"
              >
                changed: run Tune again to include
              </div>
              <div v-else-if="!o.holds && !o.fresh" class="d-flex ga-1 stale-actions">
                <VBtn size="x-small" variant="tonal" @click="keepFlag(o.song.entryId)">Keep</VBtn>
                <VMenu location="bottom end">
                  <template #activator="{ props: menuProps }">
                    <VBtn v-bind="menuProps" size="x-small" variant="tonal">Update</VBtn>
                  </template>
                  <VList density="compact">
                    <VListItem
                      v-for="b in FLAG_BANDS"
                      :key="b.band"
                      :title="b.label"
                      @click="setFlag(o.song.entryId, 'band', b.band)"
                    />
                    <VDivider />
                    <VListItem title="Higher" @click="setFlag(o.song.entryId, 'higher', null)" />
                    <VListItem title="Lower" @click="setFlag(o.song.entryId, 'lower', null)" />
                  </VList>
                </VMenu>
                <VBtn size="x-small" variant="tonal" @click="clearFlag(o.song.entryId)">Delete</VBtn>
              </div>
            </div>
            <div v-if="shownFlags.length === 0" class="text-caption text-medium-emphasis">
              No flagged song could be graded.
            </div>
          </section>

          <template v-if="hasProposal">
            <!-- 4. Old top 15 beside the new one -->
            <section class="mb-4">
              <div class="two-col">
                <div>
                  <div class="text-subtitle-2">Top {{ review.topBefore.length }} now</div>
                  <ol class="top-list text-caption">
                    <li
                      v-for="t in review.topBefore"
                      :key="t.song.entryId"
                      :class="{ dropped: changes.dropped.has(t.song.entryId), 'dropped-high': droppedHigh(t.song.entryId) }"
                    >
                      <span class="bars">{{ fmt(t.bar) }}</span>
                      {{ t.song.name }}
                      <VIcon v-if="flaggedIds.has(t.song.entryId)" size="x-small" color="warning">mdi-flag</VIcon>
                      <span v-if="changes.dropped.has(t.song.entryId)" class="tag">drops out</span>
                    </li>
                  </ol>
                </div>
                <div>
                  <div class="text-subtitle-2">Top {{ review.top.length }} with the proposal</div>
                  <ol class="top-list text-caption">
                    <li v-for="t in review.top" :key="t.song.entryId">
                      <span class="bars">{{ fmt(t.bar) }}</span>
                      {{ t.song.name }}
                      <VIcon v-if="flaggedIds.has(t.song.entryId)" size="x-small" color="warning">mdi-flag</VIcon>
                      <span v-if="rankLabel(ranks.get(t.song.entryId)) !== 'same'" class="tag">
                        {{ rankLabel(ranks.get(t.song.entryId)) }}
                      </span>
                    </li>
                  </ol>
                </div>
              </div>
            </section>

            <!-- 5. Surprises: unflagged movers and fire count -->
            <section class="mb-4">
              <div class="text-body-2 mb-1">
                Fires on (bar 0.5 or more): {{ review.firesBefore }} → {{ review.firesAfter }} songs
                · reshaping of unflagged songs {{ fmt(review.stability) }} (0 is none)
              </div>
              <div class="two-col">
                <div>
                  <div class="text-subtitle-2">Biggest risers</div>
                  <ul class="mover-list text-caption">
                    <li v-for="m in review.risers" :key="m.song.entryId">
                      <span class="bars">{{ fmt(m.before) }} → {{ fmt(m.after) }}</span> {{ m.song.name }}
                    </li>
                    <li v-if="review.risers.length === 0" class="text-medium-emphasis">None</li>
                  </ul>
                </div>
                <div>
                  <div class="text-subtitle-2">Biggest fallers</div>
                  <ul class="mover-list text-caption">
                    <li v-for="m in review.fallers" :key="m.song.entryId">
                      <span class="bars">{{ fmt(m.before) }} → {{ fmt(m.after) }}</span> {{ m.song.name }}
                    </li>
                    <li v-if="review.fallers.length === 0" class="text-medium-emphasis">None</li>
                  </ul>
                </div>
              </div>
            </section>
          </template>

          <!-- 6. Notes, never gates -->
          <section class="mb-2">
            <div class="text-subtitle-2 mb-1">Notes</div>
            <ul class="notes text-body-2">
              <li v-if="result.mode !== 'compare' && looNote(review.loo)">{{ looNote(review.loo) }}</li>
              <li v-if="review.names.length > 0">
                Name ideas: {{ review.names.slice(0, 3).map((n) => `${n.word} (${fmt(n.rho)})`).join(', ') }}
              </li>
              <li v-if="nameNote(mood.name, review.names)">{{ nameNote(mood.name, review.names) }}</li>
              <li v-for="o in rises" :key="o.moodId">
                Overlap with {{ moodName(o.moodId) }} rises from {{ fmt(o.before) }} to {{ fmt(o.after) }}
              </li>
              <li v-if="review.unsatisfied.length > 0">
                These flags couldn't be satisfied: {{ songNames(review.unsatisfied) }}
              </li>
              <li v-if="result.skippedFlags > 0">
                {{ result.skippedFlags }} flag{{ result.skippedFlags === 1 ? ' was' : 's were' }} left out: the song is not
                analyzed yet, or the flag asks nothing of the bar.
              </li>
              <li v-if="result.skipped.length > 0">
                No analysis yet for {{ songNames(result.skipped) }}
              </li>
            </ul>

            <VCard v-if="result.split" variant="outlined" class="mt-2 pa-3">
              <div class="text-subtitle-2 mb-1">Split suggestion: two moods instead of one</div>
              <div class="text-caption text-medium-emphasis mb-2">
                The flags fit two moods better than one. The two rank the library alike at
                {{ fmt(result.split.rho) }} (1 is identical). Low flags go to both.
              </div>
              <div class="two-col">
                <div>
                  <div class="text-subtitle-2">{{ mood.name }} keeps</div>
                  <pre class="lines">{{ result.split.keep.lines.join('\n') }}</pre>
                  <div class="text-caption">Songs: {{ songNames(result.split.keepSongs) || 'none' }}</div>
                  <div v-if="brokenFresh(result.split.keep) > 0" class="text-caption text-error">
                    {{ leaveBand(brokenFresh(result.split.keep)) }}
                  </div>
                </div>
                <div>
                  <div class="text-subtitle-2">New mood ({{ splitNameInitial || 'unnamed' }})</div>
                  <pre class="lines">{{ result.split.new.lines.join('\n') }}</pre>
                  <div class="text-caption">Songs: {{ songNames(result.split.newSongs) || 'none' }}</div>
                  <div v-if="result.split.new.names.length > 0" class="text-caption">
                    Name ideas: {{ result.split.new.names.slice(0, 3).map((n) => n.word).join(', ') }}
                  </div>
                  <div v-if="brokenFresh(result.split.new) > 0" class="text-caption text-error">
                    {{ leaveBand(brokenFresh(result.split.new)) }}
                  </div>
                </div>
              </div>
              <div class="d-flex mt-2">
                <VSpacer />
                <VBtn
                  size="small"
                  variant="tonal"
                  :color="splitBrokenFresh(result.split) > 0 ? 'error' : undefined"
                  :text="splitAcceptLabel(result.split)"
                  :loading="accepting"
                  :disabled="running"
                  @click="onAcceptSplit"
                />
              </div>
            </VCard>
          </section>

        </template>
      </VCardText>
      <VCardText v-else class="text-medium-emphasis">This mood is no longer in the list.</VCardText>

      <!-- 7. Accept / Reject -->
      <VDivider />
      <VAlert v-if="acceptError" type="error" variant="tonal" density="compact" class="mx-4 mt-2">
        {{ acceptError }}
        <template v-if="changedSinceTune" #append>
          <VBtn size="small" variant="tonal" :loading="running" @click="runTune">Run Tune again</VBtn>
        </template>
      </VAlert>
      <VCardActions>
        <VSpacer />
        <VBtn :text="result ? 'Reject' : 'Close'" variant="plain" @click="open = false" />
        <VBtn
          v-if="result && review && hasProposal"
          :text="acceptLabel(review)"
          :color="brokenFresh(review) > 0 ? 'error' : 'primary'"
          variant="tonal"
          :loading="accepting"
          :disabled="running"
          @click="onAccept"
        />
      </VCardActions>
    </VCard>

    <ConfirmDialog
      v-model="confirmAccept"
      title="Accept these lines?"
      :message="review ? `${acceptLabel(review)}. Their flags stay and will read as made under older wording.` : ''"
      confirm-label="Accept anyway"
      confirm-color="error"
      @confirm="accept(() => acceptLinesArgs(review!), 'accept_tuned_lines')"
    />
    <ConfirmDialog
      v-model="confirmSplit"
      title="Accept the split?"
      :message="result?.split ? `${splitAcceptLabel(result.split)}. Their flags stay and will read as made under older wording.` : ''"
      confirm-label="Accept anyway"
      confirm-color="error"
      @confirm="askSplitName = true"
    />
    <PromptDialog
      v-model="askSplitName"
      title="Accept the split"
      label="New mood's name"
      :initial="splitNameInitial"
      confirm-label="Accept split"
      @submit="acceptSplit"
    />
  </VDialog>
</template>

<style scoped>
.flag-row,
.outcome-row {
  display: grid;
  grid-template-columns: 20px minmax(0, 1fr) auto auto auto;
  align-items: center;
  gap: 8px;
  padding: 2px 4px;
  border-radius: 4px;
}

.outcome-row {
  grid-template-columns: 20px minmax(0, 1fr) auto auto 8.5em;
}

.outcome-row.is-broken {
  background: rgba(var(--v-theme-error), 0.14);
  font-weight: 600;
}

.outcome-row.is-broken-stale {
  background: rgba(var(--v-theme-warning), 0.08);
}

.stale-actions {
  grid-column: 2 / -1;
  align-items: center;
}

.flag-stale {
  opacity: 0.5;
}

.two-col {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
}

.lines {
  font-family: inherit;
  font-size: 0.8125rem;
  white-space: pre-wrap;
  padding: 6px 8px;
  margin: 2px 0 0;
  border-radius: 4px;
  background: rgba(var(--v-theme-on-surface), 0.05);
}

.versions {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 8px;
}

.proposed-line + .proposed-line {
  margin-top: 4px;
}

.proposed-line .text-caption {
  line-height: 1.2;
}

.bars {
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.top-list,
.mover-list,
.notes {
  padding-left: 22px;
  margin: 0;
}

.top-list .bars,
.mover-list .bars {
  display: inline-block;
  min-width: 3em;
  opacity: 0.7;
}

.mover-list .bars {
  min-width: 6.5em;
}

.top-list li.dropped {
  opacity: 0.6;
}

.top-list li.dropped-high {
  opacity: 1;
  color: rgb(var(--v-theme-error));
  font-weight: 600;
}

.tag {
  margin-left: 4px;
  font-size: 0.6875rem;
  opacity: 0.8;
  text-transform: uppercase;
  letter-spacing: 0.03em;
}
</style>
