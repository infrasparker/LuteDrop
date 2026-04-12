# LuteDrop Tauri + Rust Migration Design

## Motivation

LuteDrop is a D&D audio command center — multi-track playback from a desktop, controlled via a Surface tablet over the local network. The current Electron + Vue 3 prototype works, but the Web Audio API can't stream audio and maintain sample-accurate loop points simultaneously. `AudioBufferSourceNode` decodes entire files into memory (causes OOM with large/many files). `HTMLAudioElement` streams but loses loop precision. There's no middle ground in the browser.

Tauri + Rust solves this by moving the audio engine to native code where game audio libraries handle streaming + loop points natively.

## Architecture

Single Rust binary via Tauri with three responsibilities (Phase 1):

1. **Window host** — Tauri manages the OS webview running the Vue 3 frontend
2. **Audio engine** — `kira` crate handles streaming playback, loop regions, crossfades, volume/pan, mixing
3. **Persistence** — `serde` serializes/deserializes AppState to JSON

Phase 2 adds a 4th:

4. **Remote control server** — `axum` + WebSocket server running on the same `tokio` runtime, in the same binary

The Vue frontend communicates with Rust through Tauri's `invoke()` IPC system. No preload script, no `window.api` bridge.

## Project Structure

```
LuteDrop/                        (tauri-rewrite branch)
├── src-tauri/                   (Rust backend — Tauri convention)
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs              (Tauri app entry point)
│   │   ├── commands.rs          (IPC command handlers)
│   │   ├── audio/
│   │   │   ├── mod.rs
│   │   │   └── engine.rs        (kira-based audio engine)
│   │   ├── persistence.rs       (JSON file load/save)
│   │   └── models.rs            (Rust structs + ts-rs derives)
│   ├── icons/
│   └── tauri.conf.json
├── src/                         (Vue 3 frontend)
│   ├── App.vue
│   ├── main.ts
│   ├── assets/
│   ├── components/
│   │   ├── CompCard.vue
│   │   ├── CompDetails.vue
│   │   ├── TrackControl.vue
│   │   ├── ClipRow.vue
│   │   ├── GroupTab.vue
│   │   ├── WaveformEditor.vue   (new — canvas waveform with draggable markers)
│   │   └── Versions.vue
│   ├── composables/
│   │   └── useAudio.ts          (new — wraps invoke() calls to Rust audio engine)
│   ├── plugins/
│   │   └── vuetify.ts
│   ├── stores/
│   │   └── appStore.ts
│   └── views/
│       └── PlaybackView.vue
├── generated/                   (ts-rs auto-generated TypeScript types)
│   └── types.ts
├── index.html
├── vite.config.ts
├── package.json
└── tsconfig.json
```

## Data Model

Rust is the source of truth. Types defined in `src-tauri/src/models.rs` with `#[derive(Serialize, Deserialize, TS)]`. TypeScript types auto-generated to `generated/types.ts` via `ts-rs`.

The data model carries over from the Electron version unchanged:

- **Clip:** id, name, filePath, volume, trimStart, trimEnd
- **Track:** id, name, clips[], volume, mode, fadeIn, fadeOut, pan
- **TrackMode:** seamless (loopPoint) | sequence (loop, transition) | shuffle (loop, transition)
- **TrackTransition:** gapless | gap (min, max) | crossfade (duration)
- **Comp:** id, name, tracks[]
- **Group:** id, name, comps[]
- **AppState:** groups[], lastSavedAt

Playback state types:

- **TrackPlaybackState:** trackId, isPlaying, isPaused, currentClipId, currentTime
- **CompStatus:** active | partial | inactive (derived in frontend from track states)

## IPC: Frontend to Rust Commands

Commands the Rust backend exposes via `#[tauri::command]`:

| Command | Args | Returns | Purpose |
|---|---|---|---|
| `pick_audio_files` | — | `Vec<String>` | Opens native file dialog filtered to audio types |
| `load_data` | — | `AppState` | Reads JSON from app data dir |
| `save_data` | `state: AppState` | — | Writes JSON to app data dir (debounced 500ms) |
| `play_track` | `track_id: String` | — | Start/resume playing a track |
| `pause_track` | `track_id: String` | — | Pause a track |
| `stop_track` | `track_id: String` | — | Stop a track (with fade out if configured) |
| `set_track_volume` | `track_id: String, volume: f64` | — | Real-time track volume adjustment |
| `set_clip_volume` | `clip_id: String, volume: f64` | — | Real-time clip volume adjustment |
| `set_master_volume` | `volume: f64` | — | Real-time master volume adjustment |
| `activate_comp` | `comp_id: String` | — | Play all tracks in a comp |
| `stop_comp` | `comp_id: String` | — | Stop all tracks in a comp |
| `stop_all` | — | — | Stop all playback |
| `get_waveform_data` | `file_path: String, width: u32` | `Vec<f32>` | Downsample audio file to peaks for waveform display |

## IPC: Rust to Frontend Events

Tauri events pushed from Rust to the frontend. The frontend listens with `listen()` from `@tauri-apps/api/event`.

| Event | Payload | Purpose |
|---|---|---|
| `playback-state-update` | `TrackPlaybackState` | Track state changed — clip ended, sequence advanced, shuffle picked, fade completed |

The frontend uses these events to update the Pinia store's `playbackStates` map, which drives comp status indicators and track UI state.

## Audio Engine (Rust + kira)

The `AudioEngine` struct manages:
- A `kira::AudioManager` (top-level mixer)
- A map of track ID to kira mixer sub-track + active sound handles
- Current track configs (modes, transitions, clip order)

### Playback modes

**Seamless:** Load clip as a streaming sound with kira's loop region set to `loopPoint..trimEnd`. kira handles the looping natively.

**Sequence:** Play clips in order. When a sound finishes, advance to the next clip using the configured transition:
- *Gapless:* Start the next clip immediately
- *Gap:* Wait a random duration between min and max, then start next clip
- *Crossfade:* Start the next clip with overlapping fade in/fade out over the configured duration

**Shuffle:** Same as sequence but pick clips randomly instead of in order.

### Volume chain

`master volume * track volume * clip volume` — all adjustable in real time via IPC commands. kira's mixer tracks and sound handles support real-time volume tweening.

### Fade in / fade out

Applied via kira's built-in tween system. Fade in on play start, fade out on stop command. No manual timer management needed.

## Waveform Editor

A new Vue component (`WaveformEditor.vue`) for visualizing audio files and setting loop/trim points.

**Rust side:** `get_waveform_data` command reads the audio file using `symphonia` (supports mp3/wav/flac/ogg), computes amplitude peaks downsampled to the requested pixel width, returns `Vec<f32>`.

**Frontend side:** Canvas element renders the peaks as a waveform. Draggable markers overlay the waveform for:
- Trim start / trim end (all modes)
- Loop point (seamless mode)

Click to place, drag to adjust. Values update the clip/track config in the Pinia store and persist via `save_data`.

## Rust-side State

The Rust backend keeps the full `AppState` in memory (behind a `Mutex` or `RwLock`). This is necessary because:
- The audio engine needs track configs to know how to play (modes, transitions, clips)
- `activate_comp` / `stop_comp` need to look up which tracks belong to a comp
- `get_waveform_data` needs file paths from clip data
- Persistence reads/writes from this in-memory state

When the frontend updates state (adds a clip, changes a volume, renames a group), it calls `save_data` with the full state. Rust updates its in-memory copy and debounce-writes to disk. Audio-relevant changes (volume, mode) also go through their specific commands so the engine reacts immediately without re-reading the full state.

## Persistence

- `serde_json` serializes AppState to/from a JSON file
- File stored in the OS app data directory (Tauri provides this path)
- `save_data` command debounces writes with a 500ms `tokio` timer
- `load_data` on startup returns the saved state to hydrate the Pinia store
- When the frontend updates track/clip config, it calls `save_data` and also informs the audio engine of changes through the relevant command (e.g., `set_track_volume`)

## Frontend Migration

Migrated from the Electron version with these changes:

**Kept as-is:** Vue components (CompCard, CompDetails, TrackControl, ClipRow, GroupTab, Versions), Vuetify dragon's hoard theme, Pinia store structure, PlaybackView layout, vuedraggable for clip reordering.

**Modified:**
- `appStore.ts`: `window.api.saveData()` / `window.api.loadData()` replaced with `invoke('save_data')` / `invoke('load_data')`
- All `window.api.*` calls replaced with `invoke()` from `@tauri-apps/api/core`
- Event listeners use `listen()` from `@tauri-apps/api/event` instead of Electron's `ipcRenderer.on`

**Removed:** `AudioEngine.ts`, `TrackPlayer.ts`, `AudioCache.ts`, `useAudioEngine.ts` — all replaced by Rust audio engine.

**Added:** `useAudio.ts` composable (wraps `invoke()` calls for audio commands), `WaveformEditor.vue` component.

## Shared Type Generation

Rust structs in `models.rs` derive `ts_rs::TS`. Running `cargo test` (ts-rs convention) generates TypeScript interfaces to `generated/types.ts`. The frontend imports from there instead of a hand-maintained `types.ts`.

## Phasing

### Phase 1 — Desktop app with Rust audio
1. Scaffold Tauri project on `tauri-rewrite` branch
2. Define Rust data model in `models.rs` with serde + ts-rs
3. Implement persistence commands (load/save JSON)
4. Build kira audio engine (all playback modes, volume chain, fades, transitions)
5. Implement waveform data extraction
6. Migrate Vue frontend (components, store, theme, composables)
7. Wire up IPC commands and events
8. Build WaveformEditor component

### Phase 2 — Remote control server (deferred)
1. Add `axum` + `tokio-tungstenite` WebSocket server in the same binary
2. Expose the same audio commands over WebSocket
3. Serve a remote control web UI
4. Bridge WebSocket messages to the shared audio engine

## Rust Crate Dependencies (Phase 1)

- `tauri` — app framework
- `kira` — game audio (streaming, loop regions, mixer, tweens)
- `symphonia` — audio file decoding (for waveform extraction)
- `serde` + `serde_json` — serialization
- `ts-rs` — TypeScript type generation
- `tokio` — async runtime (comes with Tauri)
