//! Local transcode cache for formats kira/symphonia can't stream directly.
//!
//! Opus is the motivating case: symphonia has no Opus decoder, so we can't use
//! kira's streaming path on `.opus` files. Instead of holding the whole track
//! decoded in RAM, we transcode each `.opus` to a 16-bit WAV in a local cache
//! directory (once, lazily) and let kira stream that WAV exactly like an mp3.
//! The original `.opus` on disk stays the single source of truth.
//!
//! Cache lifecycle:
//! - **Lazy:** a WAV is produced on first [`resolve`] of a given source.
//! - **Staleness:** the cache filename hashes the source path + size + mtime, so
//!   replacing the source produces a new file and orphans the old one.
//! - **LRU size cap:** after each transcode, [`enforce_size_cap`] deletes the
//!   least-recently-used WAVs until the cache is under [`MAX_CACHE_BYTES`].
//! - **Eviction:** [`evict`] drops a single source's WAV (on entry delete/relocate).
//! - **Startup GC:** [`gc`] deletes any WAV not expected from the current set of
//!   valid source paths (handles deletes, relocations, and stale versions).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::audio::opus_decode::decode_opus;

/// Maximum total size of the WAV cache before least-recently-used eviction.
const MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GB

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
/// Serialises transcode + eviction so two plays of the same new file don't race.
static TRANSCODE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Initialises the cache directory. Called once at startup.
pub fn init(dir: PathBuf) {
    fs::create_dir_all(&dir).ok();
    let _ = CACHE_DIR.set(dir);
    let _ = TRANSCODE_LOCK.set(Mutex::new(()));
}

fn cache_dir() -> Option<&'static PathBuf> {
    CACHE_DIR.get()
}

/// True if the path needs transcoding before kira can stream it.
/// Currently only `.opus` (symphonia can stream everything else we accept).
fn needs_transcode(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("opus"))
        .unwrap_or(false)
}

fn mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Deterministic cache filename for a source: hash of path + size + mtime.
/// Returns `None` if the cache isn't initialised or the source can't be stat'd.
fn cache_file_for(source: &Path) -> Option<PathBuf> {
    let dir = cache_dir()?;
    let meta = fs::metadata(source).ok()?;
    let mut hasher = DefaultHasher::new();
    source.to_string_lossy().hash(&mut hasher);
    meta.len().hash(&mut hasher);
    mtime_secs(&meta).hash(&mut hasher);
    Some(dir.join(format!("{:016x}.wav", hasher.finish())))
}

/// Returns a path kira/symphonia can stream. For `.opus`, returns a cached WAV
/// (transcoding on first call); for any other format, returns `source` as-is.
pub fn resolve(source: &Path) -> Result<PathBuf, String> {
    if !needs_transcode(source) {
        return Ok(source.to_path_buf());
    }

    let cache_path = cache_file_for(source)
        .ok_or_else(|| "audio cache not initialised or source unreadable".to_string())?;

    if cache_path.exists() {
        touch(&cache_path);
        return Ok(cache_path);
    }

    // Serialise the build so concurrent plays don't transcode the same file twice.
    let _guard = TRANSCODE_LOCK.get().map(|m| m.lock());
    if cache_path.exists() {
        touch(&cache_path);
        return Ok(cache_path);
    }

    let decoded = decode_opus(source)?;
    write_wav(&cache_path, &decoded.frames, decoded.sample_rate)?;
    enforce_size_cap(&cache_path);
    Ok(cache_path)
}

/// Writes interleaved stereo frames as a 16-bit PCM WAV, via a temp file +
/// rename so a crash mid-write never leaves a half-written cache entry.
fn write_wav(path: &Path, frames: &[kira::Frame], sample_rate: u32) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let tmp = path.with_extension("wav.tmp");
    {
        let mut writer =
            hound::WavWriter::create(&tmp, spec).map_err(|e| format!("WAV create failed: {e}"))?;
        for f in frames {
            let l = (f.left.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            let r = (f.right.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(l).map_err(|e| format!("WAV write failed: {e}"))?;
            writer.write_sample(r).map_err(|e| format!("WAV write failed: {e}"))?;
        }
        writer.finalize().map_err(|e| format!("WAV finalize failed: {e}"))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("WAV rename failed: {e}"))?;
    Ok(())
}

/// Updates a cache file's mtime so least-recently-used eviction sees it as fresh.
fn touch(path: &Path) {
    let now = filetime_now();
    let _ = set_file_mtime(path, now);
}

/// Removes the cached WAV for a single source, if present. Best-effort.
/// Call when a library entry is deleted or relocated.
pub fn evict(source: &Path) {
    if let Some(cache_path) = cache_file_for(source) {
        let _ = fs::remove_file(cache_path);
    }
}

/// Deletes every cached WAV that isn't expected from `valid_sources`.
/// Run at startup to reclaim space from deleted/relocated/stale entries.
pub fn gc(valid_sources: &[String]) {
    let dir = match cache_dir() {
        Some(d) => d,
        None => return,
    };
    let expected: HashSet<PathBuf> = valid_sources
        .iter()
        .map(PathBuf::from)
        .filter(|p| needs_transcode(p))
        .filter_map(|p| cache_file_for(&p))
        .collect();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_wav = path.extension().and_then(|e| e.to_str()) == Some("wav");
        if is_wav && !expected.contains(&path) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Deletes least-recently-used WAVs until the cache is under the size cap.
/// `just_written` is never evicted (it's the file the caller is about to use).
fn enforce_size_cap(just_written: &Path) {
    let dir = match cache_dir() {
        Some(d) => d,
        None => return,
    };
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    // (path, size, mtime) for every cache file.
    let mut files: Vec<(PathBuf, u64, u64)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wav") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            total += meta.len();
            files.push((path, meta.len(), mtime_secs(&meta)));
        }
    }

    if total <= MAX_CACHE_BYTES {
        return;
    }

    // Oldest first.
    files.sort_by_key(|(_, _, mtime)| *mtime);
    for (path, size, _) in files {
        if total <= MAX_CACHE_BYTES {
            break;
        }
        if path == just_written {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

// --- mtime helpers (std-only, no extra crate) ---

fn filetime_now() -> SystemTime {
    SystemTime::now()
}

/// Sets a file's modified time. Best-effort; failures are ignored by callers.
fn set_file_mtime(path: &Path, time: SystemTime) -> std::io::Result<()> {
    let f = fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(time)
}
