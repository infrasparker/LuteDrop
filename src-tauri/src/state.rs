use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use crate::audio::AudioEngine;
use crate::models::AppState;

/// Managed state shared across all Tauri commands.
/// Access in commands via `state: tauri::State<'_, AppStateContainer>`.
pub struct AppStateContainer {
    pub data: RwLock<AppState>,
    pub audio: Mutex<AudioEngine>,
    pub save_path: PathBuf,
}

impl AppStateContainer {
    pub fn new(initial: AppState, save_path: PathBuf, audio: AudioEngine) -> Self {
        Self {
            data: RwLock::new(initial),
            audio: Mutex::new(audio),
            save_path,
        }
    }
}
