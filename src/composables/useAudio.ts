import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useAppStore } from '../stores/appStore';
import type { TrackPlaybackState } from '@generated/index';

let listenerRegistered = false;

function registerListener(): void {
  if (listenerRegistered) return;
  listenerRegistered = true;

  const store = useAppStore();

  listen<TrackPlaybackState>('playback-state-update', (event) => {
    const payload = event.payload;
    store.setTrackPlaybackState(payload.trackId, payload);
  });
}

export function useAudio() {
  registerListener();

  async function playTrack(trackId: string): Promise<void> {
    await invoke('play_track', { trackId });
  }

  async function pauseTrack(trackId: string): Promise<void> {
    await invoke('pause_track', { trackId });
  }

  async function stopTrack(trackId: string): Promise<void> {
    await invoke('stop_track', { trackId });
  }

  async function setMasterVolume(volume: number): Promise<void> {
    await invoke('set_master_volume', { volume });
  }

  async function setTrackVolume(trackId: string, volume: number): Promise<void> {
    await invoke('set_track_volume', { trackId, volume });
  }

  async function setClipVolume(clipId: string, volume: number): Promise<void> {
    await invoke('set_clip_volume', { clipId, volume });
  }

  async function activateComp(compId: string): Promise<void> {
    await invoke('activate_comp', { compId });
  }

  async function stopComp(compId: string): Promise<void> {
    await invoke('stop_comp', { compId });
  }

  async function stopAll(): Promise<void> {
    await invoke('stop_all');
  }

  return {
    playTrack,
    pauseTrack,
    stopTrack,
    setMasterVolume,
    setTrackVolume,
    setClipVolume,
    activateComp,
    stopComp,
    stopAll,
  };
}
