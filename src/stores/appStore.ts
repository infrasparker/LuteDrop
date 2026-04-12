import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Group, Comp, Track, AppState, TrackPlaybackState, CompStatus } from '@generated/index';

export const useAppStore = defineStore('app', () => {
  const groups = ref<Group[]>([]);
  const playbackStates = ref<Map<string, TrackPlaybackState>>(new Map());

  // --- Hydrate from Rust state ---

  function hydrate(state: AppState): void {
    groups.value = state.groups;
  }

  // --- Command helper: invoke a Rust command and hydrate ---

  async function command(cmd: string, args?: Record<string, unknown>): Promise<void> {
    const state = await invoke<AppState>(cmd, args);
    hydrate(state);
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

  return {
    groups,
    playbackStates,
    hydrate,
    command,
    setTrackPlaybackState,
    clearTrackPlaybackState,
    getCompStatus,
    findComp,
    findTrack,
  };
});
