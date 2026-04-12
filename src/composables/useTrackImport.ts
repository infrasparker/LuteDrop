// Handles the multi-step process of importing audio files as tracks.
// Opens file dialog, creates a track per file, adds the file as a clip.

import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import type { AppState } from '@generated/index';
import { useAppStore } from '../stores/appStore';

const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];

export function useTrackImport() {
  const store = useAppStore();

  async function importTracks(compId: string): Promise<void> {
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
    });
    if (!paths) return;

    const pathList = Array.isArray(paths) ? paths : [paths];

    for (const filePath of pathList) {
      const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';

      // Add track, get back state to find the new track's ID
      const stateAfterTrack = await invoke<AppState>('add_track', { compId, name: fileName });
      store.hydrate(stateAfterTrack);

      // Find the newly added track (it's the last one in the comp)
      const comp = store.findComp(compId);
      if (comp && comp.tracks.length > 0) {
        const newTrack = comp.tracks[comp.tracks.length - 1];
        await store.command('add_clip', { trackId: newTrack.id, name: fileName, filePath });
      }
    }
  }

  return { importTracks };
}
