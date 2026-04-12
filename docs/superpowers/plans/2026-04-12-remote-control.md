# Remote Control Server — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a WebSocket-based remote control server so a tablet on the LAN can control audio playback.

**Architecture:** axum HTTP + WebSocket server runs in the same Rust binary on port 3333. Serves a separate Vue + Vuetify remote app as static files. Remote sends JSON commands over WebSocket, server executes them against the shared audio engine and state, broadcasts updated state to all connected remotes. Command logic is extracted into shared `actions.rs` so both Tauri commands and the WebSocket handler use the same code.

**Tech Stack:** axum, tokio-tungstenite (via axum), tower-http (static files), tokio broadcast channel, Vue 3, Vuetify, WebSocket API

---

## File Map

### New Rust files
- `src-tauri/src/actions.rs` — Shared command logic extracted from commands.rs. Functions like `play_track(container, track_id)` that both Tauri commands and WebSocket handler call.
- `src-tauri/src/server.rs` — axum server setup: HTTP static file serving, WebSocket endpoint, broadcast channel, command dispatch.

### Modified Rust files
- `src-tauri/Cargo.toml` — Add axum, tower-http, tokio features
- `src-tauri/src/commands.rs` — Slim down to thin wrappers around actions.rs
- `src-tauri/src/state.rs` — Add broadcast sender to AppStateContainer
- `src-tauri/src/lib.rs` — Start axum server in setup, add get_server_info command, wire broadcast into polling loop

### New frontend files (remote app)
- `src/remote/index.html` — HTML entry point for remote
- `src/remote/main.ts` — Vue app entry, uses remote store
- `src/remote/App.vue` — Remote root: app bar, master volume, stop all
- `src/remote/composables/useRemote.ts` — WebSocket connection + state
- `src/remote/views/RemotePlaybackView.vue` — Groups, comps, tracks layout
- `src/remote/components/RemoteCompCard.vue` — Comp card with play/stop and status
- `src/remote/components/RemoteTrackControl.vue` — Track with play/pause/stop and volume
- `src/remote/components/RemoteClipRow.vue` — Clip name + volume slider

### New config files
- `vite.remote.config.ts` — Vite config for remote app build

### Modified frontend files
- `package.json` — Add build:remote and dev:remote scripts
- `src/App.vue` — Show remote URL in app bar

---

### Task 1: Extract shared actions from commands

**Files:**
- Create: `src-tauri/src/actions.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

The core logic in commands.rs (find items, mutate state, save, play audio) needs to be callable from both Tauri commands and the WebSocket handler. Extract it into `actions.rs` with functions that take `&AppStateContainer` instead of `State<'_, AppStateContainer>`.

- [ ] **Step 1: Create src-tauri/src/actions.rs**

Move the helper functions and core logic out of commands.rs. Each action function takes a reference to `AppStateContainer` and returns the same types.

```rust
use crate::models::{AppState, Group, Comp, Track, TrackMode, TrackTransition, Clip};
use crate::persistence;
use crate::state::AppStateContainer;
use uuid::Uuid;

// --- Helpers ---

fn save_and_return(container: &AppStateContainer, data: &AppState) -> Result<AppState, String> {
    persistence::save_to_disk(&container.save_path, data)?;
    Ok(data.clone())
}

fn find_group_mut<'a>(state: &'a mut AppState, group_id: &str) -> Result<&'a mut Group, String> {
    state.groups.iter_mut().find(|g| g.id == group_id)
        .ok_or_else(|| format!("Group not found: {}", group_id))
}

fn find_comp_mut<'a>(state: &'a mut AppState, comp_id: &str) -> Result<&'a mut Comp, String> {
    for group in &mut state.groups {
        if let Some(comp) = group.comps.iter_mut().find(|c| c.id == comp_id) {
            return Ok(comp);
        }
    }
    Err(format!("Comp not found: {}", comp_id))
}

fn find_track_mut<'a>(state: &'a mut AppState, track_id: &str) -> Result<&'a mut Track, String> {
    for group in &mut state.groups {
        for comp in &mut group.comps {
            if let Some(track) = comp.tracks.iter_mut().find(|t| t.id == track_id) {
                return Ok(track);
            }
        }
    }
    Err(format!("Track not found: {}", track_id))
}

pub fn find_track_ref<'a>(state: &'a AppState, track_id: &str) -> Result<&'a Track, String> {
    for group in &state.groups {
        for comp in &group.comps {
            if let Some(track) = comp.tracks.iter().find(|t| t.id == track_id) {
                return Ok(track);
            }
        }
    }
    Err(format!("Track not found: {}", track_id))
}

pub fn find_comp_ref<'a>(state: &'a AppState, comp_id: &str) -> Result<&'a Comp, String> {
    for group in &state.groups {
        if let Some(comp) = group.comps.iter().find(|c| c.id == comp_id) {
            return Ok(comp);
        }
    }
    Err(format!("Comp not found: {}", comp_id))
}

fn chrono_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_millis())
}

// --- Data mutation actions (return updated AppState) ---

pub fn load_data(container: &AppStateContainer) -> Result<AppState, String> {
    let data = container.data.read().map_err(|e| e.to_string())?;
    Ok(data.clone())
}

pub fn add_group(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = Group { id: Uuid::new_v4().to_string(), name, comps: Vec::new() };
    data.groups.push(group);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_group(container: &AppStateContainer, group_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    data.groups.retain(|g| g.id != group_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn rename_group(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, group_id)?;
    group.name = name;
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn add_comp(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, group_id)?;
    let comp = Comp { id: Uuid::new_v4().to_string(), name, tracks: Vec::new() };
    group.comps.push(comp);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_comp(container: &AppStateContainer, group_id: &str, comp_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, group_id)?;
    group.comps.retain(|c| c.id != comp_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn add_track(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, comp_id)?;
    let track = Track {
        id: Uuid::new_v4().to_string(),
        name,
        clips: Vec::new(),
        volume: 1.0,
        mode: TrackMode::Sequence { repeat: true, transition: TrackTransition::Gapless },
        fade_in: 0.0,
        fade_out: 0.0,
    };
    comp.tracks.push(track);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_track(container: &AppStateContainer, comp_id: &str, track_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, comp_id)?;
    comp.tracks.retain(|t| t.id != track_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn update_track(container: &AppStateContainer, comp_id: &str, track_id: &str, updates: serde_json::Value) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, comp_id)?;
    let track = comp.tracks.iter_mut().find(|t| t.id == track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let obj = updates.as_object()
        .ok_or_else(|| "updates must be a JSON object".to_string())?;

    if let Some(v) = obj.get("name") {
        track.name = v.as_str().ok_or("name must be a string")?.to_string();
    }
    if let Some(v) = obj.get("volume") {
        track.volume = v.as_f64().ok_or("volume must be a number")?;
    }
    if let Some(v) = obj.get("fadeIn") {
        track.fade_in = v.as_f64().ok_or("fadeIn must be a number")?;
    }
    if let Some(v) = obj.get("fadeOut") {
        track.fade_out = v.as_f64().ok_or("fadeOut must be a number")?;
    }
    if let Some(v) = obj.get("mode") {
        track.mode = serde_json::from_value(v.clone()).map_err(|e| format!("Invalid mode: {}", e))?;
    }
    if let Some(v) = obj.get("clips") {
        track.clips = serde_json::from_value(v.clone()).map_err(|e| format!("Invalid clips: {}", e))?;
    }

    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn add_clip(container: &AppStateContainer, track_id: &str, name: String, file_path: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let track = find_track_mut(&mut data, track_id)?;
    let clip = Clip { id: Uuid::new_v4().to_string(), name, file_path, volume: 1.0 };
    track.clips.push(clip);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_clip(container: &AppStateContainer, track_id: &str, clip_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let track = find_track_mut(&mut data, track_id)?;
    track.clips.retain(|c| c.id != clip_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

// --- Audio actions (no state return) ---

pub fn play_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let track_config = {
        let data = container.data.read().map_err(|e| e.to_string())?;
        find_track_ref(&data, track_id)?.clone()
    };
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track(track_id)
}

pub fn pause_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.pause_track(track_id)
}

pub fn stop_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_track(track_id)
}

pub fn set_master_volume(container: &AppStateContainer, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_master_volume(volume);
    Ok(())
}

pub fn set_track_volume(container: &AppStateContainer, track_id: &str, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_track_volume(track_id, volume)
}

pub fn set_clip_volume(container: &AppStateContainer, clip_id: &str, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_clip_volume(clip_id, volume)
}

pub fn activate_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs: Vec<Track> = {
        let data = container.data.read().map_err(|e| e.to_string())?;
        let comp = find_comp_ref(&data, comp_id)?;
        comp.tracks.clone()
    };
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track in &track_configs {
        if !audio.has_track(&track.id) {
            audio.setup_track(track)?;
        } else {
            audio.update_track_config(track);
        }
        audio.play_track(&track.id)?;
    }
    Ok(())
}

pub fn stop_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_ids: Vec<String> = {
        let data = container.data.read().map_err(|e| e.to_string())?;
        let comp = find_comp_ref(&data, comp_id)?;
        comp.tracks.iter().map(|t| t.id.clone()).collect()
    };
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track_id in &track_ids {
        if audio.has_track(track_id) {
            audio.stop_track(track_id)?;
        }
    }
    Ok(())
}

pub fn stop_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_all();
    Ok(())
}
```

- [ ] **Step 2: Slim down commands.rs to thin wrappers**

Replace commands.rs with thin wrappers that call actions. Remove all helper functions and logic — they now live in actions.rs.

```rust
use tauri::State;
use crate::actions;
use crate::models::AppState;
use crate::state::AppStateContainer;
use crate::waveform;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformData {
    pub peaks: Vec<f32>,
    pub duration: f64,
}

#[tauri::command]
pub fn load_data(state: State<'_, AppStateContainer>) -> Result<AppState, String> {
    actions::load_data(&state)
}

#[tauri::command]
pub fn add_group(state: State<'_, AppStateContainer>, name: String) -> Result<AppState, String> {
    actions::add_group(&state, name)
}

#[tauri::command]
pub fn delete_group(state: State<'_, AppStateContainer>, group_id: String) -> Result<AppState, String> {
    actions::delete_group(&state, &group_id)
}

#[tauri::command]
pub fn rename_group(state: State<'_, AppStateContainer>, group_id: String, name: String) -> Result<AppState, String> {
    actions::rename_group(&state, &group_id, name)
}

#[tauri::command]
pub fn add_comp(state: State<'_, AppStateContainer>, group_id: String, name: String) -> Result<AppState, String> {
    actions::add_comp(&state, &group_id, name)
}

#[tauri::command]
pub fn delete_comp(state: State<'_, AppStateContainer>, group_id: String, comp_id: String) -> Result<AppState, String> {
    actions::delete_comp(&state, &group_id, &comp_id)
}

#[tauri::command]
pub fn add_track(state: State<'_, AppStateContainer>, comp_id: String, name: String) -> Result<AppState, String> {
    actions::add_track(&state, &comp_id, name)
}

#[tauri::command]
pub fn delete_track(state: State<'_, AppStateContainer>, comp_id: String, track_id: String) -> Result<AppState, String> {
    actions::delete_track(&state, &comp_id, &track_id)
}

#[tauri::command]
pub fn update_track(state: State<'_, AppStateContainer>, comp_id: String, track_id: String, updates: serde_json::Value) -> Result<AppState, String> {
    actions::update_track(&state, &comp_id, &track_id, updates)
}

#[tauri::command]
pub fn add_clip(state: State<'_, AppStateContainer>, track_id: String, name: String, file_path: String) -> Result<AppState, String> {
    actions::add_clip(&state, &track_id, name, file_path)
}

#[tauri::command]
pub fn delete_clip(state: State<'_, AppStateContainer>, track_id: String, clip_id: String) -> Result<AppState, String> {
    actions::delete_clip(&state, &track_id, &clip_id)
}

#[tauri::command]
pub fn play_track(state: State<'_, AppStateContainer>, track_id: String) -> Result<(), String> {
    actions::play_track_audio(&state, &track_id)
}

#[tauri::command]
pub fn pause_track(state: State<'_, AppStateContainer>, track_id: String) -> Result<(), String> {
    actions::pause_track_audio(&state, &track_id)
}

#[tauri::command]
pub fn stop_track(state: State<'_, AppStateContainer>, track_id: String) -> Result<(), String> {
    actions::stop_track_audio(&state, &track_id)
}

#[tauri::command]
pub fn set_master_volume(state: State<'_, AppStateContainer>, volume: f64) -> Result<(), String> {
    actions::set_master_volume(&state, volume)
}

#[tauri::command]
pub fn set_track_volume(state: State<'_, AppStateContainer>, track_id: String, volume: f64) -> Result<(), String> {
    actions::set_track_volume(&state, &track_id, volume)
}

#[tauri::command]
pub fn set_clip_volume(state: State<'_, AppStateContainer>, clip_id: String, volume: f64) -> Result<(), String> {
    actions::set_clip_volume(&state, &clip_id, volume)
}

#[tauri::command]
pub fn activate_comp(state: State<'_, AppStateContainer>, comp_id: String) -> Result<(), String> {
    actions::activate_comp_audio(&state, &comp_id)
}

#[tauri::command]
pub fn stop_comp(state: State<'_, AppStateContainer>, comp_id: String) -> Result<(), String> {
    actions::stop_comp_audio(&state, &comp_id)
}

#[tauri::command]
pub fn stop_all(state: State<'_, AppStateContainer>) -> Result<(), String> {
    actions::stop_all_audio(&state)
}

#[tauri::command]
pub async fn get_waveform_data(file_path: String, width: u32) -> Result<WaveformData, String> {
    tokio::task::spawn_blocking(move || -> Result<WaveformData, String> {
        let (peaks, duration) = waveform::extract_peaks(&file_path, width)?;
        Ok(WaveformData { peaks, duration })
    })
    .await
    .map_err(|e| format!("Waveform task failed: {}", e))?
}
```

- [ ] **Step 3: Add `mod actions;` to lib.rs**

- [ ] **Step 4: Verify compilation**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "refactor: extract shared actions from Tauri commands"
```

---

### Task 2: Axum WebSocket Server

**Files:**
- Create: `src-tauri/src/server.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add dependencies to Cargo.toml**

Add to `[dependencies]`:
```toml
axum = { version = "0.8", features = ["ws"] }
tower-http = { version = "0.6", features = ["fs"] }
```

Also update tokio features to include what axum needs:
```toml
tokio = { version = "1", features = ["rt", "rt-multi-thread", "net", "sync"] }
```

Note: Check the latest axum and tower-http versions at crates.io. Axum 0.8 requires tokio with `rt-multi-thread` and `net`.

- [ ] **Step 2: Add broadcast channel to AppStateContainer**

Update `src-tauri/src/state.rs`:

```rust
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use tokio::sync::broadcast;
use crate::audio::AudioEngine;
use crate::models::AppState;

/// Managed state shared across all Tauri commands and the WebSocket server.
pub struct AppStateContainer {
    pub data: RwLock<AppState>,
    pub audio: Mutex<AudioEngine>,
    pub save_path: PathBuf,
    pub broadcast_tx: broadcast::Sender<String>,
}

impl AppStateContainer {
    pub fn new(initial: AppState, save_path: PathBuf, audio: AudioEngine) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            data: RwLock::new(initial),
            audio: Mutex::new(audio),
            save_path,
            broadcast_tx,
        }
    }
}
```

The broadcast channel carries serialized JSON state strings. 64-message buffer is plenty.

- [ ] **Step 3: Create src-tauri/src/server.rs**

```rust
use std::net::SocketAddr;
use std::sync::Arc;
use axum::{
    Router,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State as AxumState,
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use tower_http::services::ServeDir;

use crate::actions;
use crate::state::AppStateContainer;
use crate::models::TrackPlaybackState;

/// The full state snapshot sent to remotes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateSnapshot {
    r#type: String,
    app_state: crate::models::AppState,
    playback_states: Vec<TrackPlaybackState>,
}

/// A command received from a remote client.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteCommand {
    command: String,
    #[serde(default)]
    track_id: Option<String>,
    #[serde(default)]
    comp_id: Option<String>,
    #[serde(default)]
    clip_id: Option<String>,
    #[serde(default)]
    volume: Option<f64>,
}

type SharedState = Arc<AppStateContainer>;

/// Start the axum server. Call this from lib.rs setup.
pub async fn start_server(container: Arc<AppStateContainer>, remote_dist_path: String) {
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new(&remote_dist_path))
        .with_state(container.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], 3333));
    log::info!("Remote server listening on http://0.0.0.0:3333");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, container: SharedState) {
    // Send initial state snapshot
    if let Ok(snapshot) = build_snapshot(&container) {
        let json = serde_json::to_string(&snapshot).unwrap_or_default();
        let _ = socket.send(Message::Text(json.into())).await;
    }

    // Subscribe to broadcast channel for state updates
    let mut rx = container.broadcast_tx.subscribe();

    loop {
        tokio::select! {
            // Received a message from the remote client
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_remote_command(&container, &text);
                        // Broadcast updated state after command
                        broadcast_state(&container);
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            // Received a broadcast state update (from polling loop or other source)
            update = rx.recv() => {
                if let Ok(json) = update {
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

fn handle_remote_command(container: &AppStateContainer, text: &str) {
    let cmd: RemoteCommand = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Invalid remote command: {}", e);
            return;
        }
    };

    let result = match cmd.command.as_str() {
        "play_track" => {
            let id = cmd.track_id.as_deref().unwrap_or("");
            actions::play_track_audio(container, id)
        }
        "pause_track" => {
            let id = cmd.track_id.as_deref().unwrap_or("");
            actions::pause_track_audio(container, id)
        }
        "stop_track" => {
            let id = cmd.track_id.as_deref().unwrap_or("");
            actions::stop_track_audio(container, id)
        }
        "set_master_volume" => {
            let vol = cmd.volume.unwrap_or(1.0);
            actions::set_master_volume(container, vol)
        }
        "set_track_volume" => {
            let id = cmd.track_id.as_deref().unwrap_or("");
            let vol = cmd.volume.unwrap_or(1.0);
            actions::set_track_volume(container, id, vol)
        }
        "set_clip_volume" => {
            let id = cmd.clip_id.as_deref().unwrap_or("");
            let vol = cmd.volume.unwrap_or(1.0);
            actions::set_clip_volume(container, id, vol)
        }
        "activate_comp" => {
            let id = cmd.comp_id.as_deref().unwrap_or("");
            actions::activate_comp_audio(container, id)
        }
        "stop_comp" => {
            let id = cmd.comp_id.as_deref().unwrap_or("");
            actions::stop_comp_audio(container, id)
        }
        "stop_all" => actions::stop_all_audio(container),
        _ => {
            log::warn!("Unknown remote command: {}", cmd.command);
            Ok(())
        }
    };

    if let Err(e) = result {
        log::warn!("Remote command '{}' failed: {}", cmd.command, e);
    }
}

fn build_snapshot(container: &AppStateContainer) -> Result<StateSnapshot, String> {
    let app_state = {
        let data = container.data.read().map_err(|e| e.to_string())?;
        data.clone()
    };
    let playback_states = {
        let audio = container.audio.lock().map_err(|e| e.to_string())?;
        audio.get_all_playback_states()
    };
    Ok(StateSnapshot {
        r#type: "state".to_string(),
        app_state,
        playback_states,
    })
}

/// Build and broadcast the current state to all connected remotes.
pub fn broadcast_state(container: &AppStateContainer) {
    if let Ok(snapshot) = build_snapshot(container) {
        if let Ok(json) = serde_json::to_string(&snapshot) {
            let _ = container.broadcast_tx.send(json);
        }
    }
}

/// Get the local LAN IP address.
pub fn get_local_ip() -> String {
    use std::net::UdpSocket;
    // Connect to a public address to determine the local IP — doesn't actually send data
    let socket = UdpSocket::bind("0.0.0.0:0").ok();
    let ip = socket
        .and_then(|s| {
            s.connect("8.8.8.8:80").ok()?;
            s.local_addr().ok()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "localhost".to_string());
    ip
}
```

- [ ] **Step 4: Wire the server into lib.rs**

Update `src-tauri/src/lib.rs` to:
- Add `mod server;`
- Wrap AppStateContainer in `Arc` so it can be shared with axum
- Start the axum server in a tokio task during setup
- Add a `get_server_info` Tauri command
- Wire the broadcast channel into the polling loop

```rust
mod actions;
mod audio;
mod commands;
mod models;
mod persistence;
mod server;
mod state;
mod waveform;

use std::sync::Arc;
use std::time::Duration;

use tauri::Emitter;
use tauri::Manager;
use state::AppStateContainer;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = persistence::data_file_path(&app.handle());
            let initial_state = persistence::load_from_disk(&path);
            let audio_engine = audio::AudioEngine::new()
                .expect("Failed to initialize audio engine");

            let container = Arc::new(AppStateContainer::new(initial_state, path, audio_engine));

            // Share the Arc with Tauri's managed state
            app.manage(container.clone());

            // Start the axum WebSocket server
            let server_container = container.clone();
            let remote_dist = app.path().resource_dir()
                .unwrap_or_default()
                .join("dist-remote")
                .to_string_lossy()
                .to_string();
            tokio::spawn(async move {
                server::start_server(server_container, remote_dist).await;
            });

            // Start the playback polling loop with broadcast
            let poll_container = container.clone();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                playback_polling_loop(app_handle, poll_container);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::load_data,
            commands::add_group,
            commands::delete_group,
            commands::rename_group,
            commands::add_comp,
            commands::delete_comp,
            commands::add_track,
            commands::delete_track,
            commands::update_track,
            commands::add_clip,
            commands::delete_clip,
            commands::play_track,
            commands::pause_track,
            commands::stop_track,
            commands::set_master_volume,
            commands::set_track_volume,
            commands::set_clip_volume,
            commands::activate_comp,
            commands::stop_comp,
            commands::stop_all,
            commands::get_waveform_data,
            commands::get_server_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}

fn playback_polling_loop(app_handle: tauri::AppHandle, container: Arc<AppStateContainer>) {
    loop {
        std::thread::sleep(Duration::from_millis(50));

        let mut audio = match container.audio.lock() {
            Ok(a) => a,
            Err(_) => continue,
        };

        let finished = audio.check_finished_sounds();
        for (track_id, next_clip_index) in finished {
            if let Err(e) = audio.advance_track(&track_id, next_clip_index) {
                log::warn!("Failed to advance track {}: {}", track_id, e);
            }
        }

        let states = audio.get_all_playback_states();
        drop(audio);

        // Emit to desktop frontend
        for state in &states {
            let _ = app_handle.emit("playback-state-update", state);
        }

        // Broadcast to remote clients
        server::broadcast_state(&container);
    }
}
```

Note: The `tokio::spawn` for the axum server requires that the Tauri app runs on a tokio runtime. Tauri 2 uses tokio internally, so this should work. If it doesn't, the server can be spawned on a dedicated tokio runtime via `std::thread::spawn` + `tokio::runtime::Runtime::new()`.

- [ ] **Step 5: Add get_server_info command to commands.rs**

Append to commands.rs:

```rust
#[tauri::command]
pub fn get_server_info() -> Result<serde_json::Value, String> {
    let ip = crate::server::get_local_ip();
    Ok(serde_json::json!({
        "ip": ip,
        "port": 3333
    }))
}
```

- [ ] **Step 6: Verify compilation**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build
```

Fix any dependency version issues by checking crates.io for the latest compatible versions. The axum WebSocket API may differ — check docs.rs/axum for the correct types.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: axum WebSocket server with remote command dispatch"
```

---

### Task 3: Desktop UI — Show Remote URL

**Files:**
- Modify: `src/App.vue`

- [ ] **Step 1: Update App.vue to show the remote URL**

```vue
<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import type { AppState } from '@generated/index';
import PlaybackView from './views/PlaybackView.vue';

const store = useAppStore();
const audio = useAudio();
const masterVolume = ref(1);
const serverUrl = ref('');

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);

  try {
    const info = await invoke<{ ip: string; port: number }>('get_server_info');
    serverUrl.value = `http://${info.ip}:${info.port}`;
  } catch (e) {
    console.warn('Could not get server info:', e);
  }
});
</script>

<template>
  <VApp>
    <VAppBar density="compact" color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
        <span v-if="serverUrl" class="text-caption text-medium-emphasis ml-3">
          Remote: {{ serverUrl }}
        </span>
      </VAppBarTitle>

      <template #append>
        <VIcon class="mr-1">mdi-volume-high</VIcon>
        <VSlider
          :model-value="masterVolume"
          @update:model-value="(v: number) => { masterVolume = v; audio.setMasterVolume(v); }"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 120px"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="audio.stopAll()"
        />
      </template>
    </VAppBar>

    <VMain>
      <PlaybackView />
    </VMain>
  </VApp>
</template>
```

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "feat: show remote URL in desktop app bar"
```

---

### Task 4: Remote Vue App — Build Config & Entry

**Files:**
- Create: `vite.remote.config.ts`
- Create: `src/remote/index.html`
- Create: `src/remote/main.ts`
- Modify: `package.json`

- [ ] **Step 1: Create vite.remote.config.ts**

```typescript
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import vuetify from 'vite-plugin-vuetify';
import { resolve } from 'path';

export default defineConfig({
  plugins: [
    vue(),
    vuetify({ autoImport: true }),
  ],
  resolve: {
    alias: {
      '@generated': resolve(__dirname, 'generated'),
    },
  },
  root: 'src/remote',
  build: {
    outDir: '../../dist-remote',
    emptyOutDir: true,
  },
});
```

- [ ] **Step 2: Create src/remote/index.html**

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>LuteDrop Remote</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="./main.ts"></script>
  </body>
</html>
```

- [ ] **Step 3: Create src/remote/main.ts**

```typescript
import { createApp } from 'vue';
import App from './App.vue';
import { vuetify } from '../plugins/vuetify';

const app = createApp(App);
app.use(vuetify);
app.mount('#app');
```

Note: No Pinia — the remote uses its own reactive state from the WebSocket composable.

- [ ] **Step 4: Add scripts to package.json**

Add to the `"scripts"` section:
```json
"dev:remote": "vite --config vite.remote.config.ts",
"build:remote": "vite build --config vite.remote.config.ts"
```

- [ ] **Step 5: Add dist-remote to .gitignore**

Append `dist-remote` to `.gitignore`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: remote app build config and entry point"
```

---

### Task 5: Remote Vue App — WebSocket Composable

**Files:**
- Create: `src/remote/composables/useRemote.ts`

- [ ] **Step 1: Create the WebSocket composable**

This composable connects to the server, receives state updates, and provides functions to send commands.

```typescript
import { ref, reactive } from 'vue';
import type { AppState, TrackPlaybackState } from '@generated/index';

type CompStatus = 'active' | 'partial' | 'inactive';

interface StateSnapshot {
  type: string;
  appState: AppState;
  playbackStates: TrackPlaybackState[];
}

const groups = ref<AppState['groups']>([]);
const playbackStates = reactive(new Map<string, TrackPlaybackState>());
const connected = ref(false);

let ws: WebSocket | null = null;
let initialized = false;

function getWsUrl(): string {
  // Connect to the same host that served the page, on the /ws endpoint
  const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${location.host}/ws`;
}

function connect(): void {
  if (ws) return;

  ws = new WebSocket(getWsUrl());

  ws.onopen = () => {
    connected.value = true;
  };

  ws.onmessage = (event) => {
    try {
      const data: StateSnapshot = JSON.parse(event.data);
      if (data.type === 'state') {
        groups.value = data.appState.groups;
        playbackStates.clear();
        for (const ps of data.playbackStates) {
          playbackStates.set(ps.trackId, ps);
        }
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

function getCompStatus(compId: string): CompStatus {
  const comp = groups.value
    .flatMap((g) => g.comps)
    .find((c) => c.id === compId);
  if (!comp || comp.tracks.length === 0) return 'inactive';

  const playing = comp.tracks.filter((t) => {
    const state = playbackStates.get(t.id);
    return state?.isPlaying;
  });

  if (playing.length === 0) return 'inactive';
  if (playing.length === comp.tracks.length) return 'active';
  return 'partial';
}

export function useRemote() {
  if (!initialized) {
    initialized = true;
    connect();
  }

  return {
    groups,
    playbackStates,
    connected,
    getCompStatus,

    activateComp: (compId: string) => send({ command: 'activate_comp', compId }),
    stopComp: (compId: string) => send({ command: 'stop_comp', compId }),
    playTrack: (trackId: string) => send({ command: 'play_track', trackId }),
    pauseTrack: (trackId: string) => send({ command: 'pause_track', trackId }),
    stopTrack: (trackId: string) => send({ command: 'stop_track', trackId }),
    setMasterVolume: (volume: number) => send({ command: 'set_master_volume', volume }),
    setTrackVolume: (trackId: string, volume: number) => send({ command: 'set_track_volume', trackId, volume }),
    setClipVolume: (clipId: string, volume: number) => send({ command: 'set_clip_volume', clipId, volume }),
    stopAll: () => send({ command: 'stop_all' }),
  };
}
```

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "feat: remote WebSocket composable with state sync"
```

---

### Task 6: Remote Vue App — Components & Views

**Files:**
- Create: `src/remote/App.vue`
- Create: `src/remote/views/RemotePlaybackView.vue`
- Create: `src/remote/components/RemoteCompCard.vue`
- Create: `src/remote/components/RemoteTrackControl.vue`
- Create: `src/remote/components/RemoteClipRow.vue`

- [ ] **Step 1: Create src/remote/App.vue**

```vue
<script setup lang="ts">
import { ref } from 'vue';
import { useRemote } from './composables/useRemote';
import RemotePlaybackView from './views/RemotePlaybackView.vue';

const remote = useRemote();
const masterVolume = ref(1);
</script>

<template>
  <VApp>
    <VAppBar density="compact" color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
        <span class="text-caption text-medium-emphasis ml-2">Remote</span>
        <VIcon
          :color="remote.connected.value ? 'success' : 'error'"
          size="x-small"
          class="ml-2"
        >
          {{ remote.connected.value ? 'mdi-wifi' : 'mdi-wifi-off' }}
        </VIcon>
      </VAppBarTitle>

      <template #append>
        <VIcon class="mr-1">mdi-volume-high</VIcon>
        <VSlider
          :model-value="masterVolume"
          @update:model-value="(v: number) => { masterVolume = v; remote.setMasterVolume(v); }"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 120px"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="remote.stopAll()"
        />
      </template>
    </VAppBar>

    <VMain>
      <RemotePlaybackView />
    </VMain>
  </VApp>
</template>
```

- [ ] **Step 2: Create src/remote/views/RemotePlaybackView.vue**

```vue
<script setup lang="ts">
import { ref, computed } from 'vue';
import { useRemote } from '../composables/useRemote';
import RemoteCompCard from '../components/RemoteCompCard.vue';
import RemoteTrackControl from '../components/RemoteTrackControl.vue';

const remote = useRemote();
const activeTab = ref(0);
const selectedCompId = ref<string | null>(null);

const currentGroup = computed(() => remote.groups.value[activeTab.value] ?? null);
const selectedComp = computed(() =>
  currentGroup.value?.comps.find((c) => c.id === selectedCompId.value) ?? null
);
</script>

<template>
  <VContainer fluid>
    <VTabs v-model="activeTab" density="compact" v-if="remote.groups.value.length > 0" class="mb-4">
      <VTab v-for="(group, i) in remote.groups.value" :key="group.id" :value="i">
        {{ group.name }}
      </VTab>
    </VTabs>

    <div v-if="currentGroup" class="d-flex" style="height: calc(100vh - 120px);">
      <!-- Comp grid -->
      <div style="width: 300px; min-width: 300px; overflow-y: auto;" class="pr-3">
        <div class="d-flex flex-wrap ga-2">
          <RemoteCompCard
            v-for="comp in currentGroup.comps"
            :key="comp.id"
            :comp="comp"
            :selected="comp.id === selectedCompId"
            @select="selectedCompId = $event"
          />
        </div>
        <div v-if="currentGroup.comps.length === 0" class="text-medium-emphasis text-center pa-4">
          No comps.
        </div>
      </div>

      <!-- Track details -->
      <VDivider vertical class="mx-2" />
      <div style="flex: 1; overflow-y: auto;" class="pl-3">
        <div v-if="selectedComp">
          <span class="text-h6 font-weight-bold mb-3 d-block">{{ selectedComp.name }}</span>
          <RemoteTrackControl
            v-for="track in selectedComp.tracks"
            :key="track.id"
            :track="track"
          />
          <div v-if="selectedComp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
            No tracks.
          </div>
        </div>
        <div v-else class="text-medium-emphasis text-center pa-8">
          Select a composition.
        </div>
      </div>
    </div>

    <div v-else class="text-medium-emphasis text-center pa-8">
      No groups yet.
    </div>
  </VContainer>
</template>
```

- [ ] **Step 3: Create src/remote/components/RemoteCompCard.vue**

```vue
<script setup lang="ts">
import { computed } from 'vue';
import type { Comp } from '@generated/index';
import { useRemote } from '../composables/useRemote';

const props = defineProps<{
  comp: Comp;
  selected: boolean;
}>();

const emit = defineEmits<{
  select: [compId: string];
}>();

const remote = useRemote();
const status = computed(() => remote.getCompStatus(props.comp.id));

const borderStyle = computed(() => {
  switch (status.value) {
    case 'active': return '2px solid #FF6D00';
    case 'partial': return '2px solid #BF360C';
    default: return '2px solid transparent';
  }
});
</script>

<template>
  <VCard
    :color="selected ? 'primary' : 'surface-light'"
    variant="flat"
    class="pa-2 text-center"
    :style="{ width: '130px', cursor: 'pointer', border: borderStyle }"
    @click="emit('select', comp.id)"
  >
    <div class="d-flex justify-center ga-1 mb-2">
      <VBtn
        :icon="status === 'active' ? 'mdi-pause' : 'mdi-play'"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="status === 'active' ? remote.stopComp(comp.id) : remote.activateComp(comp.id)"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="remote.stopComp(comp.id)"
      />
    </div>
    <div class="text-subtitle-2 font-weight-bold text-truncate">{{ comp.name }}</div>
    <div class="text-caption" style="opacity: 0.6;">{{ comp.tracks.length }} tracks</div>
  </VCard>
</template>
```

- [ ] **Step 4: Create src/remote/components/RemoteTrackControl.vue**

```vue
<script setup lang="ts">
import { computed } from 'vue';
import type { Track } from '@generated/index';
import { useRemote } from '../composables/useRemote';
import RemoteClipRow from './RemoteClipRow.vue';

const props = defineProps<{
  track: Track;
}>();

const remote = useRemote();
const playbackState = computed(() => remote.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);
</script>

<template>
  <VCard class="mb-2" color="surface-light">
    <div class="d-flex align-center ga-2 pa-2">
      <VBtn
        :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
        :color="isPlaying ? 'secondary' : 'primary'"
        variant="tonal"
        size="small"
        rounded="circle"
        @click="isPlaying ? remote.pauseTrack(track.id) : remote.playTrack(track.id)"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="small"
        rounded="circle"
        @click="remote.stopTrack(track.id)"
      />

      <span class="text-subtitle-2 font-weight-bold flex-grow-1">{{ track.name }}</span>

      <span class="text-caption text-medium-emphasis">
        {{ track.clips.length }} clip{{ track.clips.length !== 1 ? 's' : '' }}
        &middot; {{ track.mode.type }}
      </span>

      <VSlider
        :model-value="track.volume"
        @update:model-value="(v: number) => remote.setTrackVolume(track.id, v)"
        :min="0"
        :max="1"
        :step="0.01"
        hide-details
        density="compact"
        style="max-width: 120px"
      />
    </div>

    <!-- Clip volumes -->
    <div v-if="track.clips.length > 0" class="px-3 pb-2">
      <RemoteClipRow
        v-for="clip in track.clips"
        :key="clip.id"
        :clip="clip"
      />
    </div>
  </VCard>
</template>
```

- [ ] **Step 5: Create src/remote/components/RemoteClipRow.vue**

```vue
<script setup lang="ts">
import type { Clip } from '@generated/index';
import { useRemote } from '../composables/useRemote';

const props = defineProps<{
  clip: Clip;
}>();

const remote = useRemote();
</script>

<template>
  <div class="d-flex align-center ga-2 py-1">
    <span class="text-body-2 flex-grow-1 text-truncate" style="max-width: 200px;">
      {{ clip.name }}
    </span>
    <VSlider
      :model-value="clip.volume"
      @update:model-value="(v: number) => remote.setClipVolume(clip.id, v)"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      style="max-width: 100px"
      prepend-icon="mdi-volume-high"
    />
  </div>
</template>
```

- [ ] **Step 6: Build the remote app to verify**

```bash
npm run build:remote
```

Expected: Builds to `dist-remote/` without errors.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: remote Vue app with playback controls"
```

---

### Task 7: Integration & Testing

**Files:**
- Possibly modify: `src-tauri/src/server.rs`, `src-tauri/src/lib.rs`

This task is about making sure everything works end-to-end. The subagent should:

- [ ] **Step 1: Build the remote app**

```bash
npm run build:remote
```

- [ ] **Step 2: Build the Rust backend**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo build
```

Fix any compilation errors.

- [ ] **Step 3: Fix the dist-remote path resolution**

The axum server needs to find `dist-remote/`. During development, it's at the project root. Check that the path in `lib.rs` resolves correctly. If using `app.path().resource_dir()`, the `dist-remote` folder needs to be included in the Tauri bundle. For development, it's simpler to use a path relative to the project root. Update `lib.rs` and `server.rs` if needed.

For dev mode, the dist-remote path can be resolved relative to the current working directory or the Cargo manifest directory. A practical approach:

```rust
// In lib.rs setup
let remote_dist = std::env::current_dir()
    .unwrap_or_default()
    .join("dist-remote")
    .to_string_lossy()
    .to_string();
```

- [ ] **Step 4: Verify the remote serves correctly**

Run the full app (`cargo tauri dev` from the project root). Open a browser to `http://localhost:3333`. The remote UI should load. Open `http://localhost:3333/ws` shouldn't crash the server (it's a WebSocket endpoint, browser will get an upgrade error, that's fine).

- [ ] **Step 5: Fix any issues found during testing**

Common issues to check:
- CORS: axum may need CORS headers if the remote dev server is on a different port. For production (same origin), this isn't an issue.
- WebSocket URL: the remote composable derives the URL from `location.host`, which works when served by axum but not when served by Vite dev server on a different port. For dev, the composable may need a hardcoded fallback.
- Tauri managed state: `Arc<AppStateContainer>` needs to work with `State<'_, Arc<AppStateContainer>>` in commands. If Tauri can't extract it, the commands may need `State<'_, AppStateContainer>` and the Arc wrapping handled differently. Check this compiles.

- [ ] **Step 6: Commit any fixes**

```bash
git add -A
git commit -m "fix: remote server integration fixes"
```

---

## Post-Implementation Checklist

- [ ] Desktop app starts and shows the remote URL in the app bar
- [ ] Remote UI loads at `http://<ip>:3333` in a browser
- [ ] Remote shows groups, comps, tracks matching the desktop
- [ ] Play/pause/stop on remote controls audio on the desktop
- [ ] Volume sliders on remote adjust audio in real time
- [ ] Changes on the desktop (add group, play track) reflect on the remote
- [ ] Multiple remotes can connect simultaneously
- [ ] Disconnecting and reconnecting works (auto-reconnect)
- [ ] Remote shows connection status indicator
