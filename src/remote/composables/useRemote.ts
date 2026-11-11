import { ref, reactive } from 'vue';
import type { AppState, TrackPlaybackState, SceneStatus } from '@generated/index';
import { sceneStatus } from '../../shared/playback';

interface StateSnapshot {
  type: string;
  appState: AppState;
  playbackStates: TrackPlaybackState[];
  masterVolume: number;
}

interface PlaybackSnapshot {
  type: string;
  playbackStates: TrackPlaybackState[];
  masterVolume: number;
}

const boards = ref<AppState['boards']>([]);
const tracks = ref<AppState['tracks']>([]);
const mix = ref<AppState['mix']>([]);
const activeSceneId = ref<string | null>(null);
const missingEntryIds = ref<Set<string>>(new Set());
const playbackStates = reactive(new Map<string, TrackPlaybackState>());
const connected = ref(false);
const masterVolume = ref(1);

let ws: WebSocket | null = null;
let initialized = false;

function getWsUrl(): string {
  // In dev mode (Vite on port 1421), connect to the Rust server on port 3333.
  // In production (served by axum), use the same host. The pairing token
  // arrives as ?token= on the page URL (from the link shown in the desktop
  // app) and must be forwarded on the upgrade request.
  const isDev = location.port === '1421';
  const host = isDev ? `${location.hostname}:3333` : location.host;
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const token = new URLSearchParams(location.search).get('token') ?? '';
  return `${protocol}//${host}/ws?token=${encodeURIComponent(token)}`;
}

function applyPlaybackStates(states: TrackPlaybackState[], volume: number | undefined): void {
  const seen = new Set<string>();
  for (const ps of states) {
    playbackStates.set(ps.trackId, ps);
    seen.add(ps.trackId);
  }
  for (const key of [...playbackStates.keys()]) {
    if (!seen.has(key)) playbackStates.delete(key);
  }
  if (volume !== undefined) {
    masterVolume.value = volume;
  }
}

function connect(): void {
  if (ws) return;

  ws = new WebSocket(getWsUrl());

  ws.onopen = () => {
    connected.value = true;
  };

  ws.onmessage = (event) => {
    try {
      const data: StateSnapshot | PlaybackSnapshot = JSON.parse(event.data);
      if (data.type === 'state') {
        const s = data as StateSnapshot;
        boards.value = s.appState.boards;
        tracks.value = s.appState.tracks;
        mix.value = s.appState.mix;
        activeSceneId.value = s.appState.activeSceneId;
        missingEntryIds.value = new Set(s.appState.missingEntryIds);
        applyPlaybackStates(s.playbackStates, s.masterVolume);
      } else if (data.type === 'playback') {
        const p = data as PlaybackSnapshot;
        applyPlaybackStates(p.playbackStates, p.masterVolume);
      }
    } catch (e) {
      console.warn('Failed to parse state update:', e);
    }
  };

  ws.onclose = () => {
    connected.value = false;
    ws = null;
    // Reconnect after a delay
    setTimeout(connect, 2000);
  };

  ws.onerror = () => {
    ws?.close();
  };
}

function send(command: Record<string, unknown>): void {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(command));
  }
}

function getSceneStatus(sceneId: string): SceneStatus {
  const scene = boards.value.flatMap((g) => g.scenes).find((c) => c.id === sceneId);
  return sceneStatus(scene, playbackStates);
}

export function useRemote() {
  if (!initialized) {
    initialized = true;
    connect();
  }

  return {
    boards,
    tracks,
    mix,
    activeSceneId,
    missingEntryIds,
    playbackStates,
    connected,
    masterVolume,
    getSceneStatus,
    isEntryMissing: (entryId: string) => missingEntryIds.value.has(entryId),

    activateScene: (sceneId: string) => send({ command: 'activate_scene', sceneId }),
    stopScene: (sceneId: string) => send({ command: 'stop_scene', sceneId }),
    pauseScene: (sceneId: string) => send({ command: 'pause_scene', sceneId }),
    resumeScene: (sceneId: string) => send({ command: 'resume_scene', sceneId }),
    playTrack: (trackId: string) => send({ command: 'play_track', trackId }),
    pauseTrack: (trackId: string) => send({ command: 'pause_track', trackId }),
    stopTrack: (trackId: string) => send({ command: 'stop_track', trackId }),
    playTrackClip: (trackId: string, clipId: string) => send({ command: 'play_track_clip', trackId, clipId }),
    setMasterVolume: (volume: number) => send({ command: 'set_master_volume', volume }),
    setTrackVolume: (trackId: string, volume: number) => send({ command: 'set_track_volume', trackId, volume }),
    setClipVolume: (clipId: string, volume: number) => send({ command: 'set_clip_volume', clipId, volume }),
    stopAll: () => send({ command: 'stop_all' }),
    pauseAll: () => send({ command: 'pause_all' }),
    resumeAll: () => send({ command: 'resume_all' }),
  };
}
