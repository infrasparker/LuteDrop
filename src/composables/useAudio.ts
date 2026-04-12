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
    stopAll,
    pauseAll,
    resumeAll,
  };
}
