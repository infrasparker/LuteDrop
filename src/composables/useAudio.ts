import { effectScope, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';
import type { TrackPlaybackState } from '@generated/index';
import { LIBRARY_PLAYBACK_ID } from '../shared/playback';

let listenerRegistered = false;
const masterVolume = ref(1);

function registerListener(): void {
  if (listenerRegistered) return;
  listenerRegistered = true;

  const store = useAppStore();

  listen<TrackPlaybackState>('playback-state-update', (event) => {
    const payload = event.payload;
    store.setTrackPlaybackState(payload.trackId, payload);
  });

  listen<number>('master-volume-update', (event) => {
    masterVolume.value = event.payload;
  });

  listen<string>('missing-entries-changed', (event) => {
    const entryId = event.payload;
    store.missingEntryIds.add(entryId);
    const entry = store.findLibraryEntry(entryId);
    const name = entry?.name ?? 'Unknown';
    const toast = useToast();
    toast.showError(`Audio file not found: ${name}`);
  });

  listen<string>('missing-entry-found', (event) => {
    store.missingEntryIds.delete(event.payload);
  });

  // Clear the "currently previewing" entry id whenever the single library
  // preview voice stops (was duplicated as a per-component watch in three
  // library components). A paused preview keeps its id so the transport
  // scrubber (and the editor playhead) still know which entry it is.
  //
  // Detached scope: registerListener() runs inside the first useAudio()
  // caller's setup, so a bare watch() would be bound to that component's
  // effect scope and disposed on its unmount — never recreated, because the
  // module-level registration flag stays true. effectScope(true) detaches
  // the watcher from any component so it lives for the app's lifetime.
  effectScope(true).run(() => {
    watch(
      () => {
        const s = store.playbackStates.get(LIBRARY_PLAYBACK_ID);
        return !!s && (s.isPlaying || s.isPaused);
      },
      (active) => {
        if (!active) store.libraryPlayingEntryId = null;
      },
    );
  });
}

export function useAudio() {
  registerListener();
  const toast = useToast();

  async function playTrack(trackId: string): Promise<void> {
    try {
      await invoke('play_track', { trackId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  // `clipId` keeps the seek in that clip; omitted, the track's first clip plays.
  // `paused` seeks a paused track and leaves it paused (one backend call).
  async function playTrackFrom(
    trackId: string, position: number, clipId?: string, paused = false,
  ): Promise<void> {
    try {
      await invoke('play_track_from', { trackId, clipId: clipId ?? null, position, paused });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function playTrackClip(trackId: string, clipId: string): Promise<void> {
    try {
      await invoke('play_track_clip', { trackId, clipId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function pauseTrack(trackId: string): Promise<void> {
    try {
      await invoke('pause_track', { trackId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function stopTrack(trackId: string): Promise<void> {
    try {
      await invoke('stop_track', { trackId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function setMasterVolume(volume: number): Promise<void> {
    try {
      await invoke('set_master_volume', { volume });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function setTrackVolume(trackId: string, volume: number): Promise<void> {
    try {
      await invoke('set_track_volume', { trackId, volume });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function setClipVolume(clipId: string, volume: number): Promise<void> {
    try {
      await invoke('set_clip_volume', { clipId, volume });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function setLibraryVolume(volume: number): Promise<void> {
    try {
      await invoke('set_library_volume', { volume });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function activateComp(compId: string): Promise<void> {
    try {
      await invoke('activate_comp', { compId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function stopComp(compId: string): Promise<void> {
    try {
      await invoke('stop_comp', { compId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function pauseComp(compId: string): Promise<void> {
    try {
      await invoke('pause_comp', { compId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function resumeComp(compId: string): Promise<void> {
    try {
      await invoke('resume_comp', { compId });
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function stopAll(): Promise<void> {
    try {
      await invoke('stop_all');
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function pauseAll(): Promise<void> {
    try {
      await invoke('pause_all');
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function resumeAll(): Promise<void> {
    try {
      await invoke('resume_all');
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function playLibrarySound(
    entryId: string,
    filePath: string,
    position: number,
    volume: number,
    trimStart?: number | null,
    trimEnd?: number | null,
    loopStart?: number | null,
    loopEnd?: number | null,
    // Seek a paused preview and leave it paused (one backend call).
    paused = false,
  ): Promise<void> {
    try {
      await invoke('play_library_sound', {
        entryId,
        filePath, position, volume,
        trimStart: trimStart ?? null,
        trimEnd: trimEnd ?? null,
        loopStart: loopStart ?? null,
        loopEnd: loopEnd ?? null,
        paused,
      });
      const appStore = useAppStore();
      appStore.libraryPlayingEntryId = entryId;
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function stopLibrarySound(): Promise<void> {
    try {
      await invoke('stop_library_sound');
      const appStore = useAppStore();
      appStore.playbackStates.delete(LIBRARY_PLAYBACK_ID);
      appStore.libraryPlayingEntryId = null;
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function toggleMasterMute(): Promise<void> {
    try {
      const muted = await invoke<boolean>('toggle_master_mute');
      const appStore = useAppStore();
      appStore.setMasterMuted(muted);
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function toggleTrackMute(trackId: string): Promise<void> {
    try {
      const muted = await invoke<boolean>('toggle_track_mute', { trackId });
      const appStore = useAppStore();
      appStore.setTrackMuted(trackId, muted);
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function toggleClipMute(clipId: string): Promise<void> {
    try {
      const muted = await invoke<boolean>('toggle_clip_mute', { clipId });
      const appStore = useAppStore();
      appStore.setClipMuted(clipId, muted);
    } catch (e) {
      toast.showError(String(e));
    }
  }

  async function toggleCompMute(compId: string): Promise<void> {
    try {
      const muted = await invoke<boolean>('toggle_comp_mute', { compId });
      const appStore = useAppStore();
      // Update all tracks in the comp
      const comp = appStore.findComp(compId);
      if (comp) {
        for (const track of comp.tracks) {
          appStore.setTrackMuted(track.id, muted);
        }
      }
    } catch (e) {
      toast.showError(String(e));
    }
  }

  return {
    masterVolume,
    playTrack,
    playTrackFrom,
    playTrackClip,
    pauseTrack,
    stopTrack,
    setMasterVolume,
    setTrackVolume,
    setClipVolume,
    setLibraryVolume,
    activateComp,
    stopComp,
    pauseComp,
    resumeComp,
    stopAll,
    pauseAll,
    resumeAll,
    playLibrarySound,
    stopLibrarySound,
    toggleMasterMute,
    toggleTrackMute,
    toggleClipMute,
    toggleCompMute,
  };
}
