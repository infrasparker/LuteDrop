import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Group, Comp, Track, AppState, TrackPlaybackState, CompStatus, LibraryEntry } from '@generated/index';
import { useToast } from '../composables/useToast';

export const useAppStore = defineStore('app', () => {
  const groups = ref<Group[]>([]);
  const playbackStates = ref<Map<string, TrackPlaybackState>>(new Map());
  const libraryEntries = ref<LibraryEntry[]>([]);
  const tags = ref<string[]>([]);
  const missingEntryIds = ref<Set<string>>(new Set());
  const previewEntryId = ref<string | null>(null);

  // --- Hydrate from Rust state ---

  function hydrate(state: AppState): void {
    groups.value = state.groups;
    libraryEntries.value = state.libraryEntries;
    tags.value = state.tags;
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

  function clearTrackPlaybackState(trackId: string): void {
    playbackStates.value.delete(trackId);
  }

  function getCompStatus(compId: string): CompStatus {
    const comp = findComp(compId);
    if (!comp || comp.tracks.length === 0) return 'inactive';

    const playing = comp.tracks.filter((t) => {
      const state = playbackStates.value.get(t.id);
      return state?.isPlaying;
    });

    if (playing.length === 0) return 'inactive';
    if (playing.length === comp.tracks.length) return 'active';
    return 'partial';
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

  async function checkMissingFiles(): Promise<void> {
    try {
      const missing = await invoke<string[]>('check_missing_files');
      missingEntryIds.value = new Set(missing);
    } catch (e) {
      const toast = useToast();
      toast.showError(String(e));
    }
  }

  return {
    groups,
    playbackStates,
    libraryEntries,
    tags,
    missingEntryIds,
    previewEntryId,
    hydrate,
    command,
    setTrackPlaybackState,
    clearTrackPlaybackState,
    getCompStatus,
    findComp,
    findTrack,
    findLibraryEntry,
    isEntryMissing,
    checkMissingFiles,
  };
});
