import type { Board, SceneTrack, Track, TrackMix, TrackMode, TrackPlaybackState } from '@generated/index';

export function playbackLabel(mode: TrackMode): string {
  if (mode.type === 'seamless') return 'Seamless';
  if (mode.type === 'oneShot') return 'One shot';
  const label = mode.type === 'shuffle' ? 'Shuffle' : 'Sequence';
  const transition = mode.transition;
  const detail = transition.type === 'gap' ? `${transition.min}–${transition.max}s gaps`
    : transition.type === 'crossfade' ? 'Crossfade' : 'Gapless';
  return `${label} · ${detail}${mode.repeat ? ' · Repeat' : ''}`;
}

export function sceneMatches(saved: SceneTrack[], live: SceneTrack[]): boolean {
  const liveMap = new Map(live.map((state) => [state.trackId, state]));
  for (const state of saved) {
    const current = liveMap.get(state.trackId);
    if (!current || state.enabled !== current.enabled || Math.abs(state.volume - current.volume) > .005
      || state.selectedClipId !== current.selectedClipId) return false;
    liveMap.delete(state.trackId);
  }
  return [...liveMap.values()].every((state) => !state.enabled);
}

/** All is a workspace; a scene includes its optional, initially disabled cards too. */
export function viewTrackIds(board: Board | undefined, sceneId: string | null): string[] {
  if (!board) return [];
  return sceneId === null ? board.trackIds : (board.scenes.find((scene) => scene.id === sceneId)?.tracks.map((track) => track.trackId) ?? []);
}

/** Context is scoped to the boards loaded for the current configuration. */
export function trackLocations(trackId: string, boards: Board[]): string[] {
  return boards.flatMap((board) => {
    const scenes = board.scenes.filter((scene) => scene.tracks.some((state) => state.trackId === trackId));
    return scenes.length ? scenes.map((scene) => `${board.name} / ${scene.name}`)
      : board.trackIds.includes(trackId) ? [`${board.name} / All tracks`] : [];
  });
}

export function trackClipSummary(track: Track): string {
  return track.clips.length ? `${track.clips.slice(0, 2).map((clip) => clip.name).join(' · ')}${track.clips.length > 2 ? ` · +${track.clips.length - 2} more` : ''}` : 'No clips yet';
}

export function trackSearchText(track: Track, boards: Board[]): string {
  return [track.name, ...track.clips.map((clip) => clip.name), ...trackLocations(track.id, boards)]
    .join(' ').normalize('NFKC').toLocaleLowerCase();
}

/** Saving a scene preserves its layout; saving All captures only active layers. */
export function captureSceneTracks(board: Board | undefined, sceneId: string | null,
  tracks: Track[], mix: TrackMix[], playback: Map<string, TrackPlaybackState>): SceneTrack[] {
  const scene = sceneId ? board?.scenes.find((item) => item.id === sceneId) : undefined;
  return viewTrackIds(board, sceneId).flatMap((id) => {
    const track = tracks.find((item) => item.id === id);
    if (!track) return [];
    const state = playback.get(id);
    const enabled = !!(state?.isPlaying || state?.isPaused);
    if (!scene && !enabled) return [];
    const live = mix.find((item) => item.trackId === id);
    const manual = track.mode.type === 'seamless' || track.mode.type === 'oneShot';
    return [{ trackId: id, enabled, volume: live?.volume ?? track.volume,
      selectedClipId: manual ? state?.currentClipId ?? live?.selectedClipId ?? track.clips[0]?.id ?? null
        : scene?.tracks.find((item) => item.trackId === id)?.selectedClipId ?? null }];
  });
}

/** Keep existing memberships visible so missing destinations are never ambiguous. */
export function trackDestinations(boards: Board[], trackId: string, currentBoardId?: string, kind: 'scenes' | 'boards' = 'scenes') {
  const ordered = [...boards].sort((a, b) => Number(b.id === currentBoardId) - Number(a.id === currentBoardId));
  return ordered.flatMap((board) => kind === 'boards'
    ? [{ id: board.id, name: board.name, added: board.trackIds.includes(trackId), props: { disabled: board.trackIds.includes(trackId) } }]
    : board.scenes.map((scene) => {
      const added = scene.tracks.some((state) => state.trackId === trackId);
      return { id: scene.id, name: `${board.name} / ${scene.name}`, added, props: { disabled: added } };
    }));
}
