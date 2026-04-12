use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use tokio::sync::broadcast;
use crate::audio::AudioEngine;
use crate::models::AppState;
use crate::waveform::PeakCache;

/// Managed state shared across all Tauri commands and the WebSocket server.
/// Access in commands via `state: tauri::State<'_, Arc<AppStateContainer>>`.
pub struct AppStateContainer {
    pub data: RwLock<AppState>,
    pub audio: Mutex<AudioEngine>,
    pub save_path: PathBuf,
    pub broadcast_tx: broadcast::Sender<String>,
    pub peak_cache: PeakCache,
}

impl AppStateContainer {
    pub fn new(initial: AppState, save_path: PathBuf, audio: AudioEngine) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            data: RwLock::new(initial),
            audio: Mutex::new(audio),
            save_path,
            broadcast_tx,
            peak_cache: PeakCache::new(),
        }
    }
}
