use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use rusqlite::Connection;
use tauri::Emitter;
use tokio::sync::broadcast;
use crate::audio::AudioEngine;
use crate::waveform::PeakCache;

pub struct AppStateContainer {
    pub db: Mutex<Connection>,
    pub audio: Mutex<AudioEngine>,
    pub broadcast_tx: broadcast::Sender<String>,
    pub peak_cache: PeakCache,
    pub db_path: PathBuf,
    pub missing_entry_ids: Mutex<HashSet<String>>,
    app_handle: Mutex<Option<tauri::AppHandle>>,
}

impl AppStateContainer {
    pub fn new(db: Connection, audio: AudioEngine, peak_cache_dir: PathBuf, db_path: PathBuf) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            db: Mutex::new(db),
            audio: Mutex::new(audio),
            broadcast_tx,
            peak_cache: PeakCache::new(peak_cache_dir),
            db_path,
            missing_entry_ids: Mutex::new(HashSet::new()),
            app_handle: Mutex::new(None),
        }
    }

    /// Set the app handle after Tauri setup. Enables event emission from mutation methods.
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        if let Ok(mut h) = self.app_handle.lock() {
            *h = Some(handle);
        }
    }

    /// Mark an entry as missing. Returns true if it was newly added.
    /// Emits a `missing-entries-changed` event to the frontend if the set changed.
    pub fn mark_entry_missing(&self, id: &str) -> bool {
        let newly_added = if let Ok(mut set) = self.missing_entry_ids.lock() {
            set.insert(id.to_string())
        } else {
            false
        };
        if newly_added {
            self.emit_missing_changed(id);
        }
        newly_added
    }

    /// Mark an entry as found (no longer missing). Returns true if it was removed.
    /// Emits a `missing-entry-found` event to the frontend if the set changed.
    pub fn mark_entry_found(&self, id: &str) -> bool {
        let was_removed = if let Ok(mut set) = self.missing_entry_ids.lock() {
            set.remove(id)
        } else {
            false
        };
        if was_removed {
            self.emit_found(id);
        }
        was_removed
    }

    /// Snapshot the current missing entry IDs as a Vec.
    pub fn missing_snapshot(&self) -> Vec<String> {
        self.missing_entry_ids
            .lock()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn emit_missing_changed(&self, id: &str) {
        if let Ok(handle) = self.app_handle.lock() {
            if let Some(ref h) = *handle {
                let _ = h.emit("missing-entries-changed", id.to_string());
            }
        }
    }

    fn emit_found(&self, id: &str) {
        if let Ok(handle) = self.app_handle.lock() {
            if let Some(ref h) = *handle {
                let _ = h.emit("missing-entry-found", id.to_string());
            }
        }
    }
}
