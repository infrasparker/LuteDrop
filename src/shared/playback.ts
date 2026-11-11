import type { Scene, SceneStatus, Board, TrackPlaybackState } from '@generated/index';

/// The synthetic track id the audio engine uses for the single library-preview
/// voice. Mirrors LIBRARY_TRACK_ID in src-tauri/src/audio/mod.rs.
export const LIBRARY_PLAYBACK_ID = '__library';

/// Aggregate playback status of a scene from a map of per-track states.
/// Pure; shared by the desktop store and the remote composable.
export function sceneStatus(
  scene: Scene | undefined,
  states: Map<string, TrackPlaybackState>,
): SceneStatus {
  const tracks = scene?.tracks.filter((t) => t.enabled) ?? [];
  if (tracks.length === 0) return 'inactive';
  const playing = tracks.filter((t) => states.get(t.trackId)?.isPlaying).length;
  const paused = tracks.filter((t) => states.get(t.trackId)?.isPaused).length;
  if (playing === tracks.length) return 'active';
  if (playing > 0) return 'partial';
  if (paused > 0) return 'paused';
  return 'inactive';
}

/// True if any track in the board is playing or paused.
export function isBoardActive(board: Board, states: Map<string, TrackPlaybackState>): boolean {
  return board.trackIds.some((id) => {
    const state = states.get(id);
    return state?.isPlaying || state?.isPaused;
  });
}
