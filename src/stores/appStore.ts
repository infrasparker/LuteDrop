import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type {
  Group,
  Comp,
  Track,
  AppState,
  TrackPlaybackState,
  CompStatus,
  LibraryEntry,
  Configuration,
  BulkImportProgress,
  MoodCalibrationInfo,
  MoodVocabulary,
  TuneReview,
  TuneSplitReview,
} from '@generated/index';
import { useToast } from '../composables/useToast';
import { compStatus, LIBRARY_PLAYBACK_ID } from '../shared/playback';
import { setMoodVocabulary } from '../utils/moods';

/// One version column on the Moods page: its lines as typed, the review
/// they were last evaluated to (and the text it was made from), and its
/// run state.
export interface MoodDraft {
  key: number;
  name: string;
  /// typed / arranged / suggested / before (the lines Use replaced), or a
  /// split's column.
  source: 'typed' | 'arranged' | 'suggested' | 'before' | 'split';
  text: string;
  review: TuneReview | null;
  /// The lines (joined by new lines) `review` was made from.
  reviewedLines: string | null;
  /// The Current lines `review` was made against.
  reviewedAgainst: string | null;
  /// The mood's flag signature when `review` was made.
  reviewedFlags: string | null;
  evaluating: boolean;
  /// Arrange / Suggest still searching for this column.
  running: 'arrange' | 'suggest' | null;
  error: string | null;
  /// A suggested column's split offer, or the split column's halves.
  split: TuneSplitReview | null;
}

export const useAppStore = defineStore('app', () => {
  const groups = ref<Group[]>([]);
  const playbackStates = ref<Map<string, TrackPlaybackState>>(new Map());
  const libraryEntries = ref<LibraryEntry[]>([]);
  const tags = ref<string[]>([]);
  const missingEntryIds = ref<Set<string>>(new Set());
  const libraryPlayingEntryId = ref<string | null>(null);
  const editEntryId = ref<string | null>(null);
  const configurations = ref<Configuration[]>([]);
  const activeConfigurationId = ref<string>('');

  // --- Moods: the vocabulary and its calibration (the Moods settings) ---
  const moodVocabulary = ref<MoodVocabulary>({ moods: [], energy: { positive: [], negative: [] } });
  const moodCalibration = ref<MoodCalibrationInfo | null>(null);
  /// Some mood's prompts changed (or a mood was added) since the
  /// calibration: the lens row shows a marker until Recalibrate runs.
  const moodsNeedRecalibration = ref(false);
  /// The stored vocabulary is unreadable: the defaults are shown and Save is
  /// refused, so it cannot delete the real moods' levels.
  const moodVocabularyUnreadable = ref(false);
  // --- The Moods page (MoodsView.vue) ---
  /// The mood the page shows (a mood id, or "energy"); null picks the first.
  const moodsSelectedId = ref<string | null>(null);
  /// The flag the page's Arrange / Suggest focus on, pinned by "Open in
  /// Moods" on a song's flag; otherwise they focus the newest flag.
  const moodFocus = ref<{ moodId: string; entryId: string } | null>(null);
  /// Version columns per mood id. Kept here so they survive leaving the
  /// page, but not a restart.
  const moodDrafts = ref<Record<string, MoodDraft[]>>({});
  /// Current's own review per mood id, with the lines it was made from.
  const moodBaselines = ref<Record<string, { lines: string[]; review: TuneReview; focusEntryId: string | null }>>({});

  /// Non-null while a background library import is active. Drives the
  /// bottom-bar progress indicator and disables the Import menu.
  const importProgress = ref<BulkImportProgress | null>(null);

  // --- Mute state (mirrors audio engine, not persisted to DB) ---
  const masterMuted = ref(false);
  const mutedTracks = ref<Set<string>>(new Set());
  const mutedClips = ref<Set<string>>(new Set());

  // --- Hydrate from Rust state ---

  function hydrate(state: AppState): void {
    groups.value = state.groups;
    libraryEntries.value = state.libraryEntries;
    tags.value = state.tags;
    configurations.value = state.configurations;
    activeConfigurationId.value = state.activeConfigurationId;
    missingEntryIds.value = new Set(state.missingEntryIds);
    moodVocabulary.value = state.moodVocabulary;
    moodCalibration.value = state.moodCalibration;
    moodsNeedRecalibration.value = state.moodsNeedRecalibration;
    moodVocabularyUnreadable.value = state.moodVocabularyUnreadable;
    setMoodVocabulary(state.moodVocabulary);

    // Prune playback states for tracks that no longer exist (deleted
    // tracks/comps/configs previously leaked entries forever).
    const validIds = new Set<string>([LIBRARY_PLAYBACK_ID]);
    for (const g of state.groups) {
      for (const c of g.comps) {
        for (const t of c.tracks) validIds.add(t.id);
      }
    }
    for (const key of [...playbackStates.value.keys()]) {
      if (!validIds.has(key)) playbackStates.value.delete(key);
    }
  }

  // --- Command helper: invoke a Rust command and hydrate; resolves false when it failed ---

  async function command(cmd: string, args?: Record<string, unknown>): Promise<boolean> {
    try {
      const state = await invoke<AppState>(cmd, args);
      hydrate(state);
      return true;
    } catch (e) {
      const toast = useToast();
      toast.showError(String(e));
      return false;
    }
  }

  // --- Playback state ---

  function setTrackPlaybackState(trackId: string, state: TrackPlaybackState): void {
    playbackStates.value.set(trackId, state);
  }

  function getCompStatus(compId: string): CompStatus {
    return compStatus(findComp(compId), playbackStates.value);
  }

  // --- Lookups ---

  function findComp(compId: string): Comp | undefined {
    for (const group of groups.value) {
      const comp = group.comps.find((c) => c.id === compId);
      if (comp) return comp;
    }
    return undefined;
  }

  function findTrack(trackId: string): Track | undefined {
    for (const group of groups.value) {
      for (const comp of group.comps) {
        const track = comp.tracks.find((t) => t.id === trackId);
        if (track) return track;
      }
    }
    return undefined;
  }

  function findLibraryEntry(entryId: string): LibraryEntry | undefined {
    return libraryEntries.value.find((e) => e.id === entryId);
  }

  function isEntryMissing(entryId: string): boolean {
    return missingEntryIds.value.has(entryId);
  }

  // --- Mute helpers ---

  function setMasterMuted(muted: boolean): void {
    masterMuted.value = muted;
  }

  function setTrackMuted(trackId: string, muted: boolean): void {
    if (muted) {
      mutedTracks.value.add(trackId);
    } else {
      mutedTracks.value.delete(trackId);
    }
  }

  function setClipMuted(clipId: string, muted: boolean): void {
    if (muted) {
      mutedClips.value.add(clipId);
    } else {
      mutedClips.value.delete(clipId);
    }
  }

  function isTrackMuted(trackId: string): boolean {
    return mutedTracks.value.has(trackId);
  }

  function isClipMuted(clipId: string): boolean {
    return mutedClips.value.has(clipId);
  }

  function isCompMuted(compId: string): boolean {
    const comp = findComp(compId);
    if (!comp || comp.tracks.length === 0) return false;
    return comp.tracks.every((t) => mutedTracks.value.has(t.id));
  }

  return {
    groups,
    playbackStates,
    libraryEntries,
    tags,
    missingEntryIds,
    libraryPlayingEntryId,
    editEntryId,
    configurations,
    activeConfigurationId,
    moodVocabulary,
    moodCalibration,
    moodsNeedRecalibration,
    moodVocabularyUnreadable,
    moodsSelectedId,
    moodFocus,
    moodDrafts,
    moodBaselines,
    importProgress,
    masterMuted,
    mutedTracks,
    mutedClips,
    hydrate,
    command,
    setTrackPlaybackState,
    getCompStatus,
    findComp,
    findTrack,
    findLibraryEntry,
    isEntryMissing,
    setMasterMuted,
    setTrackMuted,
    setClipMuted,
    isTrackMuted,
    isClipMuted,
    isCompMuted,
  };
});
