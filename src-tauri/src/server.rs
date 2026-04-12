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

    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            if let Err(e) = axum::serve(listener, app).await {
                log::error!("Remote server error: {}", e);
            }
        }
        Err(e) => {
            log::error!("Failed to bind remote server to {}: {}", addr, e);
        }
    }
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
            actions::set_master_volume_action(container, vol)
        }
        "set_track_volume" => {
            let id = cmd.track_id.as_deref().unwrap_or("");
            let vol = cmd.volume.unwrap_or(1.0);
            actions::set_track_volume_action(container, id, vol)
        }
        "set_clip_volume" => {
            let id = cmd.clip_id.as_deref().unwrap_or("");
            let vol = cmd.volume.unwrap_or(1.0);
            actions::set_clip_volume_action(container, id, vol)
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
