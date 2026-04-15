import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';
import type { TrackPlaybackState } from '@generated/index';

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

  async function playTrackFrom(trackId: string, position: number): Promise<void> {
    try {
      await invoke('play_track_from', { trackId, position });
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
  ): Promise<void> {
    try {
      await invoke('play_library_sound', {
        entryId,
        filePath, position, volume,
        trimStart: trimStart ?? null,
        trimEnd: trimEnd ?? null,
        loopStart: loopStart ?? null,
        loopEnd: loopEnd ?? null,
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
      appStore.playbackStates.delete('__library');
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
