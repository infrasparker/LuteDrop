# LuteDrop Remote Control — Design Spec

## Motivation

LuteDrop runs on a desktop upstairs, playing audio over Bluetooth to a speaker downstairs. The DM controls playback from a Surface tablet at the table. The remote control server enables this by serving a web UI over the local network that sends commands back to the desktop app in real time.

## Architecture

An axum HTTP + WebSocket server runs inside the same Rust binary, on the same tokio runtime, on port 3333. It serves:

1. Static files for the remote Vue app (built separately to `dist-remote/`)
2. A WebSocket endpoint at `/ws` for real-time bidirectional communication

The desktop app bar displays the remote URL (e.g., `http://192.168.1.50:3333`) so the DM knows where to point the tablet browser.

## Communication Protocol

### Remote → Server (commands)

JSON messages over WebSocket. The server deserializes these and calls the same Rust functions that Tauri commands use.

```json
{ "command": "activate_comp", "compId": "abc" }
{ "command": "stop_comp", "compId": "abc" }
{ "command": "play_track", "trackId": "xyz" }
{ "command": "pause_track", "trackId": "xyz" }
{ "command": "stop_track", "trackId": "xyz" }
{ "command": "set_master_volume", "volume": 0.75 }
{ "command": "set_track_volume", "trackId": "xyz", "volume": 0.5 }
{ "command": "set_clip_volume", "clipId": "xyz", "volume": 0.8 }
{ "command": "stop_all" }
```

### Server → Remote (state broadcasts)

On connect: server immediately sends the full state. On any change (triggered by desktop or remote): server broadcasts updated state to all connected remotes.

```json
{
  "type": "state",
  "appState": { "groups": [...], "lastSavedAt": "..." },
  "playbackStates": [
    { "trackId": "xyz", "isPlaying": true, "isPaused": false, "currentClipId": "abc", "currentTime": 12.5 }
  ]
}
```

The existing playback polling loop (50ms interval) already collects playback states. It will also broadcast to connected remotes.

State changes from Tauri commands (desktop user adds a group, changes volume, etc.) also trigger a broadcast so the remote stays in sync.

## Remote UI

A Vue + Vuetify app sharing the same theme (`vuetify.ts`) as the desktop but with its own components. Playback control only — no editing.

### What the remote shows:
- App bar with "LuteDrop Remote" title, master volume slider, stop all button
- Group tabs
- Comp cards with play/stop buttons and status indicators (active/partial/inactive borders)
- When a comp is selected: track list with play/pause/stop buttons, track volume slider, and clip volume sliders
- Track names, clip names, mode indicator (seamless/sequence/shuffle)

### What the remote does NOT show:
- Add/delete/rename groups, comps, tracks, clips
- Mode/transition settings
- Fade in/out settings
- Waveform editor
- Loop point controls
- Drag reordering

## File Structure

```
src/remote/                        (Remote Vue app — separate entry point)
  index.html                       (HTML entry for remote)
  main.ts                          (creates Vue app with remote store)
  App.vue                          (remote root — app bar, master controls)
  composables/useRemote.ts         (WebSocket connection, state management)
  views/RemotePlaybackView.vue     (groups, comps, tracks layout)
  components/
    RemoteCompCard.vue             (comp card with play/stop, status)
    RemoteTrackControl.vue         (track with play/pause/stop, volume)
    RemoteClipRow.vue              (clip name + volume slider)
src/plugins/vuetify.ts             (shared — imported by both apps)

src-tauri/src/
  server.rs                        (axum server setup, static file serving, WebSocket handler)

vite.remote.config.ts              (builds remote app to dist-remote/)
```

## Rust Server (`src-tauri/src/server.rs`)

### Responsibilities:
- Start axum HTTP server on port 3333 bound to 0.0.0.0
- Serve static files from the embedded `dist-remote/` directory (or a path relative to the binary)
- Handle WebSocket upgrades at `/ws`
- On WebSocket connect: send current AppState + playback states
- On WebSocket message: deserialize the command, execute it against the shared AppStateContainer + AudioEngine (same functions as Tauri commands), broadcast updated state to all remotes
- Maintain a list of connected WebSocket clients for broadcasting

### Integration with existing code:
- The server accesses the same `AppStateContainer` (with `RwLock<AppState>` and `Mutex<AudioEngine>`) that Tauri commands use
- Command execution logic should be extracted from `commands.rs` into shared functions in a new `actions.rs` module. Both Tauri commands and the WebSocket handler call these shared functions. For example, `actions::play_track(state: &AppStateContainer, track_id: &str) -> Result<(), String>` is called by both the Tauri `play_track` command and the WebSocket command handler. Tauri commands become thin wrappers around action functions.
- The playback polling loop in `lib.rs` gets a handle to the broadcast channel to send state updates to remotes

### Broadcasting:
- Use a `tokio::sync::broadcast` channel
- The polling loop sends state updates through the channel every 50ms
- Tauri mutation commands (add_group, update_track, etc.) also send through the channel after mutating state
- Each WebSocket handler subscribes to the channel and forwards updates to its client

## Desktop Integration

The desktop `App.vue` app bar displays the remote URL. This requires:
- A new Tauri command `get_server_info` that returns `{ ip: String, port: u16 }`
- The server determines the LAN IP using the same `networkInterfaces()` approach from the Electron version (iterate network interfaces, find the first non-internal IPv4 address)

## Rust Crate Dependencies

- `axum` — HTTP framework
- `tokio-tungstenite` (via axum's WebSocket support) — WebSocket handling
- `tower-http` — static file serving middleware

These all run on the tokio runtime that Tauri already provides.

## Build & Deploy

The remote app builds separately via `vite.remote.config.ts`:
```bash
npm run build:remote    # vite build --config vite.remote.config.ts
```

Output goes to `dist-remote/`. The axum server serves files from this directory.

For development, the remote can be served by a separate Vite dev server on a different port, with WebSocket pointing to the Rust server on 3333. Or just build it and test against the axum server directly.

The main `package.json` scripts:
```json
{
  "dev": "vite",
  "dev:remote": "vite --config vite.remote.config.ts",
  "build": "vue-tsc --noEmit && vite build",
  "build:remote": "vite build --config vite.remote.config.ts"
}
```
