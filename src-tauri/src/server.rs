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
    master_volume: f64,
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

/// Port for the phone-remote HTTP/WebSocket server. Also reported by the
/// `get_server_info` command — keep the two in sync via this constant.
pub const REMOTE_PORT: u16 = 3333;

/// Random per-session pairing token required on every `/ws` upgrade.
pub fn generate_remote_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Start the axum server. Call this from lib.rs setup.
pub async fn start_server(container: Arc<AppStateContainer>, remote_dist_path: String) {
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback_service(ServeDir::new(&remote_dist_path))
        .with_state(container.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], REMOTE_PORT));
    log::info!("Remote server listening on http://0.0.0.0:{}", REMOTE_PORT);
    log::info!("Remote pairing token: {}", container.remote_token);

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
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    headers: axum::http::HeaderMap,
    AxumState(state): AxumState<SharedState>,
) -> axum::response::Response {
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    let host = headers.get("host").and_then(|v| v.to_str().ok());
    let token = params.get("token").map(String::as_str);
    if !ws_request_allowed(origin, host, token, &state.remote_token) {
        log::warn!("Rejected remote WS connection (origin: {:?})", origin);
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| handle_ws(socket, state)).into_response()
}

async fn handle_ws(mut socket: WebSocket, container: SharedState) {
    // Send initial state snapshot
    if let Some(snapshot) = build_snapshot(&container) {
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
        "play_track_clip" => {
            let track_id = cmd.track_id.as_deref().unwrap_or("");
            let clip_id = cmd.clip_id.as_deref().unwrap_or("");
            actions::play_track_clip(container, track_id, clip_id)
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
        "pause_comp" => {
            let id = cmd.comp_id.as_deref().unwrap_or("");
            actions::pause_comp_audio(container, id)
        }
        "resume_comp" => {
            let id = cmd.comp_id.as_deref().unwrap_or("");
            actions::resume_comp_audio(container, id)
        }
        "stop_all" => actions::stop_all_audio(container),
        "pause_all" => actions::pause_all_audio(container),
        "resume_all" => actions::resume_all_audio(container),
        _ => {
            log::warn!("Unknown remote command: {}", cmd.command);
            Ok(())
        }
    };

    if let Err(e) = result {
        log::warn!("Remote command '{}' failed: {}", cmd.command, e);
    }
}

fn build_snapshot(container: &AppStateContainer) -> Option<StateSnapshot> {
    let mut app_state = {
        let conn = container.db.lock().ok()?;
        crate::database::load_full_state(&conn).ok()?
    };
    app_state.missing_entry_ids = container.missing_snapshot();
    let (playback_states, master_volume) = {
        let audio = container.audio.lock().ok()?;
        (audio.get_all_playback_states(), audio.get_master_volume())
    };
    Some(StateSnapshot {
        r#type: "state".to_string(),
        app_state,
        playback_states,
        master_volume,
    })
}

/// Build and broadcast the current state to all connected remotes.
pub fn broadcast_state(container: &AppStateContainer) {
    if let Some(snapshot) = build_snapshot(container) {
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

/// Extracts the host portion of an authority string ("host", "host:port",
/// "[v6::literal]:port"). IPv6 brackets are kept so two distinct IPv6
/// literals never collapse to the same prefix fragment.
fn host_of(authority: &str) -> &str {
    if authority.starts_with('[') {
        match authority.find(']') {
            Some(end) => &authority[..end + 1], // include both brackets
            None => "", // malformed bracketed literal → rejected by caller
        }
    } else {
        authority.split(':').next().unwrap_or(authority)
    }
}

/// Gate for `/ws` upgrades. The token is the primary check; the Origin check
/// additionally blocks cross-site WebSocket hijacking from browser pages
/// (WebSockets are exempt from CORS). Origin may be absent for non-browser
/// clients — the token still protects those. When present, the Origin's host
/// must equal the host the request was addressed to; the port is ignored so
/// the vite dev remote (port 1421) can reach the server (port 3333).
fn ws_request_allowed(
    origin: Option<&str>,
    request_host: Option<&str>,
    token: Option<&str>,
    expected_token: &str,
) -> bool {
    if token != Some(expected_token) {
        return false;
    }
    let Some(origin) = origin else { return true };
    let origin_host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .map(host_of)
        .unwrap_or("");
    let req_host = request_host.map(host_of).unwrap_or("");
    !origin_host.is_empty() && origin_host == req_host
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_tokens_are_random_and_nonempty() {
        let a = generate_remote_token();
        let b = generate_remote_token();
        assert_eq!(a.len(), 32); // uuid v4 simple form: 32 hex chars
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn ws_rejects_bad_or_missing_token() {
        assert!(!ws_request_allowed(None, Some("192.168.1.5:3333"), None, "tok"));
        assert!(!ws_request_allowed(None, Some("192.168.1.5:3333"), Some("wrong"), "tok"));
    }

    #[test]
    fn ws_accepts_valid_token_without_origin() {
        assert!(ws_request_allowed(None, Some("192.168.1.5:3333"), Some("tok"), "tok"));
    }

    #[test]
    fn ws_accepts_same_host_origin_any_port() {
        // Production: remote page served by axum itself
        assert!(ws_request_allowed(
            Some("http://192.168.1.5:3333"), Some("192.168.1.5:3333"), Some("tok"), "tok"));
        // Dev: vite remote server on 1421 connecting to 3333
        assert!(ws_request_allowed(
            Some("http://192.168.1.5:1421"), Some("192.168.1.5:3333"), Some("tok"), "tok"));
    }

    #[test]
    fn ws_rejects_cross_site_origin_even_with_token() {
        assert!(!ws_request_allowed(
            Some("https://evil.example"), Some("192.168.1.5:3333"), Some("tok"), "tok"));
    }

    #[test]
    fn ws_rejects_different_ipv6_origin_host() {
        assert!(!ws_request_allowed(
            Some("http://[2001:db8:1234:5678::abcd]:1421"),
            Some("[2001:db8:1234:5678::ef01]:3333"),
            Some("tok"), "tok"));
    }

    #[test]
    fn ws_accepts_same_ipv6_origin_host_any_port() {
        assert!(ws_request_allowed(
            Some("http://[2001:db8::1]:1421"),
            Some("[2001:db8::1]:3333"),
            Some("tok"), "tok"));
    }

    #[test]
    fn ws_rejects_malformed_bracketed_origin() {
        assert!(!ws_request_allowed(
            Some("http://[2001:db8::1"),
            Some("[2001:db8::1]:3333"),
            Some("tok"), "tok"));
    }
}
