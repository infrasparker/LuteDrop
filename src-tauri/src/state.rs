use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::sync::MutexGuard;
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
    /// Random per-session pairing token for the phone remote (see server.rs).
    pub remote_token: String,
    pub missing_entry_ids: Mutex<HashSet<String>>,
    /// True while a background library import is running. Used to block
    /// overlapping imports. Set via `try_claim_import` / `release_import`.
    pub import_in_progress: AtomicBool,
    /// Set by any action that changes persisted app state or the missing-entries snapshot; the broadcast tick
    /// consumes it to decide when remotes need a fresh structural snapshot.
    pub state_dirty: AtomicBool,
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
            remote_token: crate::server::generate_remote_token(),
            missing_entry_ids: Mutex::new(HashSet::new()),
            import_in_progress: AtomicBool::new(false),
            state_dirty: AtomicBool::new(false),
            app_handle: Mutex::new(None),
        }
    }

    /// Atomically claims the import slot. Returns `true` if acquired, `false`
    /// if another import is already running. Pair with `release_import`.
    pub fn try_claim_import(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.import_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn release_import(&self) {
        use std::sync::atomic::Ordering;
        self.import_in_progress.store(false, Ordering::Release);
    }

    pub fn mark_state_dirty(&self) {
        use std::sync::atomic::Ordering;
        self.state_dirty.store(true, Ordering::Release);
    }

    /// Returns true at most once per mark — the broadcast tick's consume side.
    pub fn take_state_dirty(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.state_dirty.swap(false, Ordering::AcqRel)
    }

    /// Locks the audio engine, recovering from mutex poisoning. A panic on
    /// the poll thread must not permanently kill audio: engine state stays
    /// structurally valid across a panic (single-field mutations only), so
    /// recovering the guard is safe. The poison is logged so a crashing poll
    /// tick is visible instead of silent.
    pub fn lock_audio(&self) -> MutexGuard<'_, AudioEngine> {
        self.audio.lock().unwrap_or_else(|poisoned| {
            log::error!("Audio mutex poisoned (a thread panicked while holding it); recovering");
            poisoned.into_inner()
        })
    }

    /// Emits a Tauri event to the frontend if the app handle is set.
    pub fn emit<P: serde::Serialize + Clone>(&self, event: &str, payload: P) {
        if let Ok(handle) = self.app_handle.lock() {
            if let Some(ref h) = *handle {
                let _ = h.emit(event, payload);
            }
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
            self.mark_state_dirty();
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
            self.mark_state_dirty();
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
