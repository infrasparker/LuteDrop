import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Group, Comp, Track, AppState, TrackPlaybackState, CompStatus, LibraryEntry, Configuration } from '@generated/index';
import { useToast } from '../composables/useToast';

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
  }

  // --- Command helper: invoke a Rust command and hydrate ---

  async function command(cmd: string, args?: Record<string, unknown>): Promise<void> {
    try {
      const state = await invoke<AppState>(cmd, args);
      hydrate(state);
    } catch (e) {
      const toast = useToast();
      toast.showError(String(e));
    }
  }

  // --- Playback state ---

  function setTrackPlaybackState(trackId: string, state: TrackPlaybackState): void {
    playbackStates.value.set(trackId, state);
  }

  function getCompStatus(compId: string): CompStatus {
    const comp = findComp(compId);
    if (!comp || comp.tracks.length === 0) return 'inactive';

    const playing = comp.tracks.filter((t) => {
      const state = playbackStates.value.get(t.id);
      return state?.isPlaying;
    });
    const paused = comp.tracks.filter((t) => {
      const state = playbackStates.value.get(t.id);
      return state?.isPaused;
    });

    if (playing.length === comp.tracks.length) return 'active';
    if (playing.length > 0) return 'partial';
    if (paused.length > 0) return 'paused';
    return 'inactive';
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
