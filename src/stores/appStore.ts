import { captureSceneTracks } from '../utils/soundstage';
import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type {
  Board,
  Scene,
  Track,
  TrackMix,
  SceneTrack,
  AppState,
  TrackPlaybackState,
  SceneStatus,
  LibraryEntry,
  Configuration,
  BulkImportProgress,
  MoodCalibrationInfo,
  MoodVocabulary,
  TuneReview,
  TuneSplitReview,
} from '@generated/index';
import { useToast } from '../composables/useToast';
import { sceneStatus, LIBRARY_PLAYBACK_ID } from '../shared/playback';
import { setMoodVocabulary } from '../utils/moods';

/// One version column on the Moods page: its lines as typed, the review
/// they were last evaluated to (and the text it was made from), and its
/// run state.
export interface MoodDraft {
  key: number;
  name: string;
  /// typed / arranged / suggested / before (the lines Use replaced), or a
  /// split's column.
  source: 'typed' | 'arranged' | 'suggested' | 'learned' | 'before' | 'split';
  profile?: import('@generated/index').LearnedMoodProfile | null;
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
  running: 'arrange' | 'suggest' | 'learn' | null;
  error: string | null;
  /// A suggested column's split offer, or the split column's halves.
  split: TuneSplitReview | null;
}

export const useAppStore = defineStore('app', () => {
  const boards = ref<Board[]>([]);
  const tracks = ref<Track[]>([]);
  const mix = ref<TrackMix[]>([]);
  const activeSceneId = ref<string | null>(null);
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
    boards.value = state.boards;
    tracks.value = state.tracks;
    mix.value = state.mix;
    activeSceneId.value = state.activeSceneId;
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
    // tracks/scenes/configs previously leaked entries forever).
    const validIds = new Set<string>([LIBRARY_PLAYBACK_ID]);
    for (const track of state.tracks) validIds.add(track.id);
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

  function getSceneStatus(sceneId: string): SceneStatus {
    return sceneStatus(findScene(sceneId), playbackStates.value);
  }

  // --- Lookups ---

  function findScene(sceneId: string): Scene | undefined {
    for (const board of boards.value) {
      const scene = board.scenes.find((c) => c.id === sceneId);
      if (scene) return scene;
    }
    return undefined;
  }

  function findTrack(trackId: string): Track | undefined {
    return tracks.value.find((track) => track.id === trackId);
  }

  function trackMix(trackId: string): TrackMix {
    return mix.value.find((state) => state.trackId === trackId)
      ?? { trackId, volume: findTrack(trackId)?.volume ?? 1, selectedClipId: null };
  }

  function setMix(trackId: string, patch: Partial<TrackMix>): void {
    const current = mix.value.find((state) => state.trackId === trackId);
    if (current) Object.assign(current, patch);
    else mix.value.push({ ...trackMix(trackId), ...patch });
  }

  function setClipVolume(clipId: string, volume: number): void {
    for (const track of tracks.value) {
      const clip = track.clips.find((item) => item.id === clipId);
      if (clip) {
        clip.volume = volume;
        return;
      }
    }
  }

  function captureScene(boardId: string, sceneId: string | null = null): SceneTrack[] {
    return captureSceneTracks(boards.value.find((board) => board.id === boardId), sceneId, tracks.value, mix.value, playbackStates.value);
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

  function isSceneMuted(sceneId: string): boolean {
    const scene = findScene(sceneId);
    if (!scene || scene.tracks.length === 0) return false;
    return scene.tracks.every((t) => mutedTracks.value.has(t.trackId));
  }

  return {
    boards,
    tracks,
    mix,
    activeSceneId,
    trackMix,
    setMix,
    setClipVolume,
    captureScene,
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
    getSceneStatus,
    findScene,
    findTrack,
    findLibraryEntry,
    isEntryMissing,
    setMasterMuted,
    setTrackMuted,
    setClipMuted,
    isTrackMuted,
    isClipMuted,
    isSceneMuted,
  };
});
