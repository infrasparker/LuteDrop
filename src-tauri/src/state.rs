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
        }
    }
}
