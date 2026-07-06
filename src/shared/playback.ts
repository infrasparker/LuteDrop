import type { Comp, CompStatus, Group, TrackPlaybackState } from '@generated/index';

/// The synthetic track id the audio engine uses for the single library-preview
/// voice. Mirrors LIBRARY_TRACK_ID in src-tauri/src/audio/mod.rs.
export const LIBRARY_PLAYBACK_ID = '__library';

/// Aggregate playback status of a comp from a map of per-track states.
/// Pure; shared by the desktop store and the remote composable.
export function compStatus(
  comp: Comp | undefined,
  states: Map<string, TrackPlaybackState>,
): CompStatus {
  if (!comp || comp.tracks.length === 0) return 'inactive';
  const playing = comp.tracks.filter((t) => states.get(t.id)?.isPlaying).length;
  const paused = comp.tracks.filter((t) => states.get(t.id)?.isPaused).length;
  if (playing === comp.tracks.length) return 'active';
  if (playing > 0) return 'partial';
  if (paused > 0) return 'paused';
  return 'inactive';
}

/// True if any track in the group is playing or paused.
export function isGroupActive(group: Group, states: Map<string, TrackPlaybackState>): boolean {
  return group.comps.some((comp) =>
    comp.tracks.some((track) => {
      const s = states.get(track.id);
      return s?.isPlaying || s?.isPaused;
    }),
  );
}
