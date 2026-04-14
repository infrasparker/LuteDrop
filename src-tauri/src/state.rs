use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use rusqlite::Connection;
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
        }
    }

    /// Mark an entry as missing. Returns true if it was newly added.
    pub fn mark_entry_missing(&self, id: &str) -> bool {
        if let Ok(mut set) = self.missing_entry_ids.lock() {
            set.insert(id.to_string())
        } else {
            false
        }
    }

    /// Mark an entry as found (no longer missing). Returns true if it was removed.
    pub fn mark_entry_found(&self, id: &str) -> bool {
        if let Ok(mut set) = self.missing_entry_ids.lock() {
            set.remove(id)
        } else {
            false
        }
    }

    /// Snapshot the current missing entry IDs as a Vec.
    pub fn missing_snapshot(&self) -> Vec<String> {
        self.missing_entry_ids
            .lock()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }
}
