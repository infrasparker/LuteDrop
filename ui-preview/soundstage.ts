// The browser harness exercises the real UI against an in-memory Soundstage.
import type { AppState, SceneTrack, SceneReference, Track, TrackPlaybackState } from '@generated/index';
import { useAppStore } from '../src/stores/appStore';
type Args = Record<string, unknown>;
const id = () => crypto.randomUUID();
export function soundstageCommand(state: AppState, command: string, args: Args): unknown {
  const findTrack = (key: unknown) => state.tracks.find((track) => track.id === key);
  const board = state.boards.find((item) => item.id === args.boardId);
  const sceneBoard = state.boards.find((item) => item.scenes.some((scene) => scene.id === args.sceneId));
  const scene = sceneBoard?.scenes.find((item) => item.id === args.sceneId);
  const track = findTrack(args.trackId);
  function mix(trackId: string) {
    let current = state.mix.find((item) => item.trackId === trackId);
    if (!current) { current = { trackId, volume: findTrack(trackId)?.volume ?? 1, selectedClipId: null }; state.mix.push(current); }
    return current;
  }
  function play(item: Track, clipId?: string | null) {
    const store = useAppStore();
    const clip = clipId ?? (item.mode.type === 'shuffle' ? item.clips[Math.floor(Math.random() * item.clips.length)]?.id : item.mode.type === 'sequence' ? item.clips[0]?.id : mix(item.id).selectedClipId ?? item.clips[0]?.id);
    if (!clip) throw new Error('Add clips to this track first');
    const before = store.playbackStates.get(item.id);
    const playback: TrackPlaybackState = { trackId: item.id, isPlaying: true, isPaused: false, currentClipId: clip,
      currentTime: before?.isPlaying && before.currentClipId === clip ? before.currentTime : 0, hasSound: true };
    store.setTrackPlaybackState(item.id, playback);
  }
  function stop(trackId: string) {
    useAppStore().setTrackPlaybackState(trackId, { trackId, isPlaying: false, isPaused: false, currentClipId: null, currentTime: 0, hasSound: false });
  }
  function references(trackId: unknown, clipId?: unknown): SceneReference[] {
    return state.boards.flatMap((b) => b.scenes.flatMap((s) => s.tracks
      .filter((reference) => reference.trackId === trackId && (!clipId || reference.selectedClipId === clipId))
      .map((reference) => ({ sceneId: s.id, sceneName: s.name, boardName: b.name,
        configurationName: state.configurations.find((c) => c.id === state.activeConfigurationId)?.name ?? '', selectedClipId: reference.selectedClipId }))));
  }
  switch (command) {
    case 'get_track_references': return references(args.trackId, args.clipId);
    case 'add_board': state.boards.push({ id: id(), name: args.name as string, scenes: [], trackIds: [] }); break;
    case 'rename_board': if (board) board.name = args.name as string; break;
    case 'delete_board': state.boards = state.boards.filter((item) => item.id !== args.boardId); break;
    case 'add_track': {
      const created: Track = { id: id(), name: args.name as string, volume: 1, fadeIn: 0, fadeOut: 0,
        mode: args.mode === 'shuffle' || args.mode === 'sequence' ? { type: args.mode, repeat: true, transition: { type: 'gapless' } }
          : args.mode === 'oneShot' ? { type: 'oneShot' } : { type: 'seamless', loopStart: 0, loopEnd: 0 }, clips: [] };
      created.clips = ((args.libraryEntryIds ?? []) as string[]).map((entryId) => {
        const entry = state.libraryEntries.find((item) => item.id === entryId)!;
        return { id: id(), name: entry.name, libraryEntryId: entry.id, filePath: entry.filePath, volume: 1,
          libraryVolume: entry.volume, loudnessFactor: entry.loudnessFactor, trimStart: entry.trimStart, trimEnd: entry.trimEnd };
      });
      state.tracks.push(created); board?.trackIds.push(created.id);
      if (scene && sceneBoard) { if (!sceneBoard.trackIds.includes(created.id)) sceneBoard.trackIds.push(created.id); scene.tracks.push({trackId:created.id,enabled:false,volume:1,selectedClipId:null}); }
      break;
    }
    case 'attach_track': if (board && track && !board.trackIds.includes(track.id)) board.trackIds.push(track.id); break;
    case 'detach_track':
      if (args.removeFromScenes && board) for (const item of board.scenes) item.tracks = item.tracks.filter((ref) => ref.trackId !== args.trackId);
      if (board?.scenes.some((s) => s.tracks.some((t) => t.trackId === args.trackId))) throw new Error('Remove this track from the saved scenes on this board first.');
      if (board) board.trackIds = board.trackIds.filter((trackId) => trackId !== args.trackId); break;
    case 'attach_scene_track': if (scene && sceneBoard && track) {
      if (!sceneBoard.trackIds.includes(track.id)) sceneBoard.trackIds.push(track.id);
      if (!scene.tracks.some((ref) => ref.trackId === track.id)) scene.tracks.push({trackId:track.id,enabled:false,volume:mix(track.id).volume,selectedClipId:track.mode.type === 'seamless' || track.mode.type === 'oneShot' ? mix(track.id).selectedClipId : null});
    } break;
    case 'detach_scene_track': if (scene) { scene.tracks = scene.tracks.filter((ref) => ref.trackId !== args.trackId); if (state.activeSceneId === scene.id) stop(args.trackId as string); } break;
    case 'reorder_scene_track': if (scene) { const index=scene.tracks.findIndex((ref) => ref.trackId === args.trackId); const [ref]=scene.tracks.splice(index,1); scene.tracks.splice(args.newIndex as number,0,ref); } break;
    case 'make_scene_track_independent': if (scene && sceneBoard && track) {
      const copy=structuredClone(track); copy.id=id(); copy.name+=' (copy)';
      const selected=scene.tracks.find((ref) => ref.trackId === track.id)!;
      const clipIndex=copy.clips.findIndex((clip) => clip.id === selected.selectedClipId);
      for (const clip of copy.clips) clip.id=id();
      selected.trackId=copy.id; selected.selectedClipId=copy.clips[clipIndex]?.id ?? null;
      sceneBoard.trackIds.push(copy.id); state.tracks.push(copy); if (state.activeSceneId === scene.id) state.activeSceneId=null;
    } break;
    case 'duplicate_track': if (track) {
      const copied = structuredClone(track); copied.id = id(); copied.name = args.name as string;
      for (const clip of copied.clips) clip.id = id(); state.tracks.push(copied);
    } break;
    case 'delete_track':
      if (!args.removeFromScenes && references(args.trackId).length) throw new Error('This shared track is used by scenes');
      state.tracks = state.tracks.filter((item) => item.id !== args.trackId);
      for (const b of state.boards) { b.trackIds = b.trackIds.filter((trackId) => trackId !== args.trackId); for (const s of b.scenes) s.tracks = s.tracks.filter((t) => t.trackId !== args.trackId); }
      break;
    case 'update_track': if (track) {
      const updates = args.updates as Partial<Track>;
      if (updates.clips) track.clips = updates.clips.map((clip) => track.clips.find((item) => item.id === clip.id)!);
      const { clips: _clips, ...fields } = updates; Object.assign(track, fields);
    } break;
    case 'add_clips': if (track) for (const entryId of args.libraryEntryIds as string[]) {
      const entry = state.libraryEntries.find((item) => item.id === entryId)!;
      track.clips.push({ id: id(), name: entry.name, libraryEntryId: entry.id, filePath: entry.filePath, volume: 1,
        libraryVolume: entry.volume, loudnessFactor: entry.loudnessFactor, trimStart: entry.trimStart, trimEnd: entry.trimEnd });
    } break;
    case 'delete_clip': if (track) {
      const refs = references(track.id, args.clipId);
      if (refs.length && !args.replacementClipId && !args.disableAffectedScenes) throw new Error('Choose a replacement or disable the track in affected scenes');
      for (const b of state.boards) for (const s of b.scenes) for (const reference of s.tracks) if (reference.trackId === track.id && reference.selectedClipId === args.clipId) {
        reference.selectedClipId = args.replacementClipId as string | null;
        if (!args.replacementClipId) reference.enabled = false;
      }
      track.clips = track.clips.filter((clip) => clip.id !== args.clipId);
      if (mix(track.id).selectedClipId === args.clipId) mix(track.id).selectedClipId = args.replacementClipId as string | null;
    } break;
    case 'add_scene': if (board) { const created = { id: id(), name: args.name as string, tracks: structuredClone(args.tracks as SceneTrack[]) }; board.scenes.push(created); if (args.markActive !== false) state.activeSceneId = created.id; } break;
    case 'save_scene': if (scene) scene.tracks = structuredClone(args.tracks as SceneTrack[]); break;
    case 'rename_scene': if (scene) scene.name = args.name as string; break;
    case 'delete_scene': if (sceneBoard) sceneBoard.scenes = sceneBoard.scenes.filter((item) => item.id !== args.sceneId); break;
    case 'copy_scene': case 'move_scene': {
      const destination = state.boards.find((item) => item.id === args.destinationBoardId);
      if (!scene || !destination) throw new Error('Scene destination not found');
      const result = command === 'copy_scene' ? { ...structuredClone(scene), id: id(), name: args.name as string } : scene;
      if (command === 'move_scene' && sceneBoard) sceneBoard.scenes = sceneBoard.scenes.filter((item) => item.id !== scene.id);
      const position = destination === sceneBoard && command === 'copy_scene' ? destination.scenes.indexOf(scene) + 1 : destination.scenes.length;
      destination.scenes.splice(position, 0, result);
      for (const reference of result.tracks) if (!destination.trackIds.includes(reference.trackId)) destination.trackIds.push(reference.trackId);
    } break;
    case 'reorder_scene': if (scene && sceneBoard) { sceneBoard.scenes.splice(sceneBoard.scenes.indexOf(scene), 1); sceneBoard.scenes.splice(args.newIndex as number, 0, scene); } break;
    case 'reorder_track': if (board && track) { board.trackIds.splice(board.trackIds.indexOf(track.id), 1); board.trackIds.splice(args.newIndex as number, 0, track.id); } break;
    case 'activate_scene': if (scene) {
      if (!args.addToMix) for (const item of state.tracks) if (!scene.tracks.some((t) => t.trackId === item.id && t.enabled)) stop(item.id);
      for (const reference of scene.tracks) {
        if (args.addToMix && !reference.enabled) continue;
        Object.assign(mix(reference.trackId), { volume: reference.volume, selectedClipId: reference.selectedClipId });
        const item = findTrack(reference.trackId); if (reference.enabled && item) play(item, reference.selectedClipId);
      }
      state.activeSceneId = args.addToMix ? null : scene.id;
    } break;
    case 'play_track': if (track) play(track); break;
    case 'play_track_clip': if (track) { mix(track.id).selectedClipId = args.clipId as string; play(track, args.clipId as string); } break;
    case 'stop_track': if (track) stop(track.id); break;
    case 'pause_track': if (track) { const before = useAppStore().playbackStates.get(track.id); if (before) useAppStore().setTrackPlaybackState(track.id, { ...before, isPlaying: false, isPaused: true }); } break;
    case 'stop_all': for (const item of state.tracks) stop(item.id); break;
    case 'set_track_volume': if (track) mix(track.id).volume = args.volume as number; break;
    case 'set_clip_volume': {
      const clip = state.tracks.flatMap((item) => item.clips).find((item) => item.id === args.clipId);
      if (clip) clip.volume = args.volume as number;
    } break;
    default: return undefined;
  }
  return state;
}
