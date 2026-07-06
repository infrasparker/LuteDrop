use std::sync::Arc;

use crate::audio::util::is_audio_path;
use crate::database;
use crate::database::{configurations, hierarchy, library, DbError};
use crate::models::{AppState, BulkImportProgress, LibraryEntryUsage, Track, TrackUpdates};
use crate::state::AppStateContainer;
use rusqlite::params;

// --- Helpers ---

fn with_db<F, T>(container: &AppStateContainer, f: F) -> Result<T, String>
where
    F: FnOnce(&rusqlite::Connection) -> Result<T, String>,
{
    let conn = container.db.lock().map_err(|e| e.to_string())?;
    f(&conn)
}

/// Loads full state from DB and enriches it with the missing-entry-ids snapshot.
fn load_state(container: &AppStateContainer) -> Result<AppState, String> {
    let mut state = with_db(container, |conn| database::load_full_state(conn))?;
    state.missing_entry_ids = container.missing_snapshot();
    container.mark_state_dirty();
    Ok(state)
}

/// The dominant action shape: run one DB mutation closure, then reload and
/// return the full state. Collapses ~25 identical `with_db(...)?; load_state(...)`
/// pairs.
fn mutate_and_reload<F>(container: &AppStateContainer, f: F) -> Result<AppState, String>
where
    F: FnOnce(&rusqlite::Connection) -> Result<(), String>,
{
    with_db(container, f)?;
    load_state(container)
}

pub fn find_track_ref(container: &AppStateContainer, track_id: &str) -> Result<Track, String> {
    with_db(container, |conn| database::load_track_by_id(conn, track_id))
}

pub fn find_comp_tracks(container: &AppStateContainer, comp_id: &str) -> Result<Vec<Track>, String> {
    with_db(container, |conn| database::load_tracks_for_comp(conn, comp_id))
}

/// Volumes from any caller (IPC or remote) are clamped to [0, 1] before they
/// reach the engine or the database. Non-finite values are rejected outright.
fn sanitize_volume(volume: f64) -> Result<f64, String> {
    if volume.is_finite() {
        Ok(volume.clamp(0.0, 1.0))
    } else {
        Err("volume must be a finite number".into())
    }
}

// --- Data mutation actions (return updated AppState) ---

pub fn load_data(container: &AppStateContainer) -> Result<AppState, String> {
    load_state(container)
}

pub fn add_group(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        let config_id = database::get_active_configuration_id(conn)?;
        hierarchy::insert_group(conn, &name, &config_id)?;
        Ok(())
    })
}

pub fn delete_group(container: &AppStateContainer, group_id: &str) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::delete_group(conn, group_id)?;
        Ok(())
    })
}

pub fn rename_group(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::rename_group(conn, group_id, &name)?;
        Ok(())
    })
}

pub fn add_comp(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::insert_comp(conn, group_id, &name)?;
        Ok(())
    })
}

pub fn delete_comp(container: &AppStateContainer, comp_id: &str) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::delete_comp(conn, comp_id)?;
        Ok(())
    })
}

pub fn rename_comp(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::rename_comp(conn, comp_id, &name)?;
        Ok(())
    })
}

pub fn add_track(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::insert_track(conn, comp_id, &name)?;
        Ok(())
    })
}

pub fn delete_track(container: &AppStateContainer, track_id: &str) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::delete_track(conn, track_id)?;
        Ok(())
    })
}

pub fn update_track(container: &AppStateContainer, track_id: &str, updates: TrackUpdates) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        // NOTE: no outer transaction here — `hierarchy::set_clip_order` below
        // opens its own `unchecked_transaction` on `conn`, and SQLite rejects a
        // nested BEGIN. The other updates are independent single-statement
        // UPDATEs; the clip loop's atomicity is provided by `set_clip_order`.
        if let Some(name) = &updates.name {
            hierarchy::rename_track(conn, track_id, name)?;
        }
        if let Some(volume) = updates.volume {
            hierarchy::update_track_volume(conn, track_id, volume)?;
        }
        if updates.fade_in.is_some() || updates.fade_out.is_some() {
            // update_track_fades takes both; read current values for the absent one.
            let (cur_in, cur_out): (f64, f64) = conn
                .query_row(
                    "SELECT fade_in, fade_out FROM tracks WHERE id = ?1",
                    params![track_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| format!("Failed to read current fades: {e}"))?;
            let fade_in = updates.fade_in.unwrap_or(cur_in);
            let fade_out = updates.fade_out.unwrap_or(cur_out);
            hierarchy::update_track_fades(conn, track_id, fade_in, fade_out)?;
        }
        if let Some(mode) = &updates.mode {
            let transition_type = mode.transition.as_ref().and_then(|t| t.r#type.as_deref());
            let gap_min = mode.transition.as_ref().and_then(|t| t.min);
            let gap_max = mode.transition.as_ref().and_then(|t| t.max);
            // Normalise inverted gap ranges — the engine tolerates them
            // (transitions::gap_delay) but they shouldn't reach the DB.
            let (gap_min, gap_max) = match (gap_min, gap_max) {
                (Some(lo), Some(hi)) if lo > hi => (Some(hi), Some(lo)),
                other => other,
            };
            hierarchy::update_track_mode(
                conn, track_id, &mode.r#type, mode.repeat, transition_type, gap_min, gap_max,
            )?;
        }
        if let Some(clips) = &updates.clips {
            let ordered_ids: Vec<String> = clips.iter().map(|c| c.id.clone()).collect();
            hierarchy::set_clip_order(conn, track_id, &ordered_ids)?;
        }
        Ok(())
    })
}

pub fn add_clip(container: &AppStateContainer, track_id: &str, library_entry_id: &str) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::insert_clip(conn, track_id, library_entry_id)?;
        Ok(())
    })
}

pub fn add_clips(container: &AppStateContainer, track_id: &str, library_entry_ids: &[String]) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Failed to begin add_clips transaction: {e}"))?;
        for entry_id in library_entry_ids {
            hierarchy::insert_clip(conn, track_id, entry_id)?;
        }
        tx.commit().map_err(|e| format!("Failed to commit add_clips: {e}"))?;
        Ok(())
    })
}

pub fn delete_clip(container: &AppStateContainer, _track_id: &str, clip_id: &str) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        hierarchy::delete_clip(conn, clip_id)?;
        Ok(())
    })
}

// --- Configuration actions (return updated AppState) ---

pub fn create_configuration(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        let id = configurations::insert_configuration(conn, &name)?;
        configurations::set_active_configuration(conn, &id)?;
        Ok(())
    })
}

pub fn duplicate_configuration(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        let active_id = database::get_active_configuration_id(conn)?;
        let new_id = configurations::duplicate_configuration(conn, &active_id, &name)?;
        configurations::set_active_configuration(conn, &new_id)?;
        Ok(())
    })
}

pub fn load_configuration(container: &AppStateContainer, config_id: String) -> Result<AppState, String> {
    // Stop all audio before switching
    {
        let mut audio = container.lock_audio();
        audio.stop_all();
    }
    with_db(container, |conn| {
        configurations::set_active_configuration(conn, &config_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn delete_configuration(container: &AppStateContainer, config_id: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM configurations", [], |row| row.get(0))
            .map_err(|e| format!("Failed to count configurations: {e}"))?;
        if count <= 1 {
            return Err("Cannot delete the only configuration".to_string());
        }

        let active_id = database::get_active_configuration_id(conn)?;
        if active_id == config_id {
            let next_id: String = conn
                .query_row(
                    "SELECT id FROM configurations WHERE id != ?1 ORDER BY sort_order ASC LIMIT 1",
                    params![config_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("Failed to find next configuration: {e}"))?;
            configurations::set_active_configuration(conn, &next_id)?;
        }

        configurations::delete_configuration(conn, &config_id)?;
        Ok(())
    })
}

pub fn rename_configuration(container: &AppStateContainer, config_id: String, name: String) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        configurations::rename_configuration(conn, &config_id, &name)?;
        Ok(())
    })
}

// --- Library actions (return updated AppState) ---

/// Streams a file through SHA-256, returning the lowercase hex digest.
/// Shared by single import (hash-if-absent) and bulk import (always hash).
fn hash_file(path: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("Failed to open file for hashing: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) => return Err(format!("Failed to read file for hashing: {e}")),
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn import_library_entry(
    container: &AppStateContainer,
    file_path: String,
    content_hash: Option<String>,
    name: String,
    duration: Option<f64>,
    sample_rate: Option<i64>,
    file_size: Option<i64>,
    entry_type: String,
) -> Result<AppState, String> {
    // Compute SHA-256 hash if not provided
    let hash = match content_hash {
        Some(h) => Some(h),
        None => Some(hash_file(&file_path)?),
    };

    mutate_and_reload(container, |conn| {
        // Check for duplicate by hash
        if let Some(ref h) = hash {
            if let Some((_id, existing_name)) = library::get_library_entry_by_hash(conn, h)? {
                return Err(format!("Already in library as '{}'", existing_name));
            }
        }

        let entry_id = library::insert_library_entry(
            conn,
            &file_path,
            hash.as_deref(),
            &name,
            duration,
            sample_rate,
            file_size,
            &entry_type,
        )
        .map_err(String::from)?;

        // Probe duration from the audio file if not already provided
        if duration.is_none() {
            if let Some(probed_duration) = crate::audio::util::probe_duration(&file_path) {
                let _ = library::update_library_entry_duration(conn, &entry_id, probed_duration, None);
            }
        }

        Ok(())
    })
}

pub fn update_library_entry_tags(
    container: &AppStateContainer,
    entry_id: &str,
    tags: Vec<String>,
) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| library::set_entry_tags(conn, entry_id, &tags))
}

pub fn update_library_entry_type(
    container: &AppStateContainer,
    entry_id: &str,
    entry_type: &str,
) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        library::update_library_entry_type(conn, entry_id, entry_type)?;
        Ok(())
    })
}

pub fn update_library_entry_markers(
    container: &AppStateContainer,
    entry_id: &str,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        library::update_library_entry_markers(conn, entry_id, trim_start, trim_end, loop_start, loop_end)?;
        Ok(())
    })
}

pub fn update_library_entry_volume(
    container: &AppStateContainer,
    entry_id: &str,
    volume: f64,
) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        library::update_library_entry_volume(conn, entry_id, volume)?;
        Ok(())
    })
}

pub fn rename_library_entry(
    container: &AppStateContainer,
    entry_id: &str,
    name: String,
) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| {
        library::update_library_entry_name(conn, entry_id, &name)?;
        Ok(())
    })
}

pub fn delete_library_entry(
    container: &AppStateContainer,
    entry_id: &str,
) -> Result<AppState, String> {
    // Capture the file path before deletion so we can evict its cache files.
    let file_path = with_db(container, |conn| {
        let state = database::load_full_state(conn)?;
        Ok(state
            .library_entries
            .iter()
            .find(|e| e.id == entry_id)
            .map(|e| e.file_path.clone()))
    })?;

    with_db(container, |conn| {
        library::delete_library_entry(conn, entry_id)?;
        Ok(())
    })?;

    if let Some(fp) = file_path {
        let p = std::path::Path::new(&fp);
        container.peak_cache.evict(p);
    }

    container.mark_entry_found(entry_id);
    load_state(container)
}

/// Removes peak cache files no longer referenced by any library entry.
/// Called once at startup to reclaim disk.
pub fn gc_caches(container: &AppStateContainer) {
    let paths = with_db(container, |conn| library::get_all_file_paths(conn)).unwrap_or_default();
    container.peak_cache.gc(&paths);
}

pub fn get_library_entry_usage(
    container: &AppStateContainer,
    entry_id: &str,
) -> Result<Vec<LibraryEntryUsage>, String> {
    with_db(container, |conn| library::get_entry_usage(conn, entry_id))
}

/// Recursively walks `folder_path` and returns every audio file found inside,
/// filtered by `AUDIO_EXTENSIONS`. Unreadable sub-entries are skipped silently;
/// an unreadable root surfaces as an error.
pub fn scan_folder_for_audio(folder_path: &str) -> Result<Vec<String>, String> {
    let root = std::path::Path::new(folder_path);
    if !root.exists() {
        return Err(format!("Folder does not exist: {folder_path}"));
    }
    if !root.is_dir() {
        return Err(format!("Not a directory: {folder_path}"));
    }

    let mut paths = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() && is_audio_path(entry.path()) {
            if let Some(s) = entry.path().to_str() {
                paths.push(s.to_string());
            }
        }
    }
    paths.sort();
    Ok(paths)
}

/// Kicks off a background library import. The command returns immediately;
/// the spawned thread processes each file with its own short transaction and
/// emits `library-import-progress` after each file plus
/// `library-import-complete` when finished. Fails fast with an error if
/// another import is already running.
///
/// Per-file transactions (vs. one batch transaction) mean a bad file in
/// position 47 doesn't roll back the first 46 successful imports.
pub fn start_bulk_import_library_entries(
    container: &Arc<AppStateContainer>,
    paths: Vec<String>,
    tags: Vec<String>,
    entry_type: String,
) -> Result<(), String> {
    if !container.try_claim_import() {
        return Err("Another import is already running".to_string());
    }

    let container = container.clone();
    std::thread::spawn(move || {
        run_bulk_import(container, paths, tags, entry_type);
    });

    Ok(())
}

fn run_bulk_import(container: Arc<AppStateContainer>, paths: Vec<String>, tags: Vec<String>, entry_type: String) {
    let total = paths.len() as u32;
    let mut imported: u32 = 0;
    let mut skipped: u32 = 0;

    // Announce the task so the frontend can show an indicator before the first file completes.
    container.emit(
        "library-import-progress",
        BulkImportProgress { processed: 0, imported, skipped, total },
    );

    for (i, file_path) in paths.iter().enumerate() {
        let processed = (i as u32) + 1;

        // Hash the file outside the DB lock.
        let hash = match hash_file(file_path) {
            Ok(h) => h,
            Err(e) => {
                log::warn!("Skipping {file_path}: {e}");
                skipped += 1;
                emit_progress(&container, processed, imported, skipped, total);
                continue;
            }
        };

        // Per-file transaction: insert + tag in one unit, then probe duration after commit.
        let file_result: Result<Option<String>, String> = (|| {
            let conn = container
                .db
                .lock()
                .map_err(|e| format!("db lock poisoned: {e}"))?;

            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;

            if library::get_library_entry_by_hash(&conn, &hash)?.is_some() {
                return Ok(None); // skip
            }

            let derived_name = std::path::Path::new(file_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();

            let id = match library::insert_library_entry(
                &conn, file_path, Some(&hash), &derived_name, None, None, None, &entry_type,
            ) {
                Ok(id) => id,
                Err(DbError::Constraint) => return Ok(None), // file_path collision = skip
                Err(e) => return Err(e.to_string()),
            };

            for tag_name in &tags {
                library::add_tag_to_entry(&conn, &id, tag_name)?;
            }

            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(Some(id))
        })();

        match file_result {
            Ok(Some(id)) => {
                imported += 1;
                // Probe duration after the insert tx has committed so the UI sees
                // the row immediately and the duration fills in shortly after.
                if let Some(dur) = crate::audio::util::probe_duration(file_path) {
                    if let Ok(conn) = container.db.lock() {
                        let _ = library::update_library_entry_duration(&conn, &id, dur, None);
                    }
                }
                // Per-file dirty mark: connected remotes see the library grow
                // as the import proceeds rather than only at the end.
                container.mark_state_dirty();
            }
            Ok(None) => {
                skipped += 1;
            }
            Err(e) => {
                log::warn!("Skipping {file_path}: {e}");
                skipped += 1;
            }
        }

        emit_progress(&container, processed, imported, skipped, total);
    }

    container.emit(
        "library-import-complete",
        BulkImportProgress { processed: total, imported, skipped, total },
    );
    // Belt-and-braces: ensure remotes get a final structural snapshot after
    // the import finishes (cheap atomic store; redundant marks are harmless).
    container.mark_state_dirty();
    container.release_import();
}

fn emit_progress(
    container: &AppStateContainer,
    processed: u32,
    imported: u32,
    skipped: u32,
    total: u32,
) {
    container.emit(
        "library-import-progress",
        BulkImportProgress { processed, imported, skipped, total },
    );
}

pub fn relocate_library_entry(
    container: &AppStateContainer,
    entry_id: &str,
    new_file_path: String,
) -> Result<AppState, String> {
    // Get the old file path so we can evict its peak cache
    let old_file_path = with_db(container, |conn| {
        let state = database::load_full_state(conn)?;
        state.library_entries.iter()
            .find(|e| e.id == entry_id)
            .map(|e| e.file_path.clone())
            .ok_or_else(|| format!("Entry not found: {}", entry_id))
    })?;

    with_db(container, |conn| {
        library::update_library_entry_file_path(conn, entry_id, &new_file_path)?;
        Ok(())
    })?;

    // Evict old peak caches (in-memory + disk .peaks file)
    if old_file_path != new_file_path {
        let old = std::path::Path::new(&old_file_path);
        container.peak_cache.evict(old);
    }

    if std::path::Path::new(&new_file_path).exists() {
        container.mark_entry_found(entry_id);
    }
    load_state(container)
}

/// Scans all library entries for missing files and populates the backend set.
/// Called once at launch before any frontend state is sent.
pub fn scan_missing_files(container: &AppStateContainer) {
    let entries = match with_db(container, |conn| database::load_full_state(conn)) {
        Ok(state) => state.library_entries,
        Err(_) => return,
    };
    for entry in &entries {
        if !std::path::Path::new(&entry.file_path).exists() {
            container.mark_entry_missing(&entry.id);
        }
    }
}

// --- Audio actions (no state return) ---

/// Check file existence for all clips in a track and update missing state.
fn update_clip_file_state(container: &AppStateContainer, track: &Track) {
    for clip in &track.clips {
        if !clip.file_path.is_empty() {
            if std::path::Path::new(&clip.file_path).exists() {
                container.mark_entry_found(&clip.library_entry_id);
            } else {
                container.mark_entry_missing(&clip.library_entry_id);
            }
        }
    }
}

/// Setup the track in the engine if absent, else push its latest config.
/// Shared preamble of the play actions.
fn ensure_track_ready(audio: &mut crate::audio::AudioEngine, track: &Track) -> Result<(), String> {
    if !audio.has_track(&track.id) {
        audio.setup_track(track)?;
    } else {
        audio.update_track_config(track);
    }
    Ok(())
}

pub fn play_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    update_clip_file_state(container, &track_config);
    let mut audio = container.lock_audio();
    ensure_track_ready(&mut audio, &track_config)?;
    audio.play_track(track_id)
}

pub fn play_track_clip(container: &AppStateContainer, track_id: &str, clip_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    update_clip_file_state(container, &track_config);
    let mut audio = container.lock_audio();
    ensure_track_ready(&mut audio, &track_config)?;
    audio.play_track_clip(track_id, clip_id)
}

pub fn play_track_from_position(container: &AppStateContainer, track_id: &str, position: f64) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    update_clip_file_state(container, &track_config);
    let mut audio = container.lock_audio();
    ensure_track_ready(&mut audio, &track_config)?;
    audio.play_track_from(track_id, position)
}

pub fn pause_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.lock_audio();
    audio.pause_track(track_id)
}

pub fn stop_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.lock_audio();
    audio.stop_track(track_id)
}

pub fn set_master_volume(container: &AppStateContainer, volume: f64) -> Result<(), String> {
    let volume = sanitize_volume(volume)?;
    let mut audio = container.lock_audio();
    audio.set_master_volume(volume);
    Ok(())
}

pub fn set_track_volume(container: &AppStateContainer, track_id: &str, volume: f64) -> Result<(), String> {
    let volume = sanitize_volume(volume)?;
    // Persist to DB (cheap, no full state reload)
    {
        let conn = container.db.lock().map_err(|e| e.to_string())?;
        hierarchy::update_track_volume(&conn, track_id, volume)?;
    }
    container.mark_state_dirty();
    // Update audio engine only if track is currently active
    let mut audio = container.lock_audio();
    if audio.has_track(track_id) {
        audio.set_track_volume(track_id, volume)?;
    }
    Ok(())
}

pub fn set_clip_volume(container: &AppStateContainer, clip_id: &str, volume: f64) -> Result<(), String> {
    let volume = sanitize_volume(volume)?;
    // Persist to DB (cheap, no full state reload)
    {
        let conn = container.db.lock().map_err(|e| e.to_string())?;
        hierarchy::update_clip_volume(&conn, clip_id, volume)?;
    }
    container.mark_state_dirty();
    // Update audio engine — set_clip_volume is a no-op if clip isn't in any active track
    let mut audio = container.lock_audio();
    audio.set_clip_volume(clip_id, volume)
}

pub fn set_library_volume(container: &AppStateContainer, volume: f64) -> Result<(), String> {
    let volume = sanitize_volume(volume)?;
    let mut audio = container.lock_audio();
    audio.set_library_volume(volume);
    Ok(())
}

pub fn activate_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    for track in &track_configs {
        update_clip_file_state(container, track);
    }
    let mut audio = container.lock_audio();
    for track in &track_configs {
        ensure_track_ready(&mut audio, track)?;
        audio.play_track(&track.id)?;
    }
    Ok(())
}

pub fn stop_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    let mut audio = container.lock_audio();
    for track in &track_configs {
        if audio.has_track(&track.id) {
            audio.stop_track(&track.id)?;
        }
    }
    Ok(())
}

pub fn pause_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    let mut audio = container.lock_audio();
    for track in &track_configs {
        if audio.has_track(&track.id) {
            audio.pause_track(&track.id)?;
        }
    }
    Ok(())
}

pub fn resume_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    let mut audio = container.lock_audio();
    for track in &track_configs {
        if audio.has_track(&track.id) {
            audio.resume_track(&track.id)?;
        }
    }
    Ok(())
}

pub fn stop_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.lock_audio();
    audio.stop_all();
    Ok(())
}

pub fn pause_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.lock_audio();
    audio.pause_all();
    Ok(())
}

pub fn resume_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.lock_audio();
    audio.resume_all();
    Ok(())
}

pub fn play_library_sound(
    container: &AppStateContainer,
    entry_id: &str,
    file_path: &str,
    position: f64,
    volume: f64,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    // Check file exists before attempting playback
    if !std::path::Path::new(file_path).exists() {
        container.mark_entry_missing(entry_id);
        return Err(format!("Audio file not found: {}", file_path));
    }
    // File exists — clear stale missing state if any
    container.mark_entry_found(entry_id);
    let mut audio = container.lock_audio();
    // Volume is linear now; master is applied by the engine node automatically.
    let vol = sanitize_volume(volume)? as f32;
    let loop_region = match (loop_start, loop_end) {
        (Some(ls), Some(le)) if le > ls => Some((ls, le)),
        (None, Some(le)) if le > 0.0 => Some((0.0, le)),
        _ => None,
    };
    audio.play_library_sound(crate::audio::sound::PlayParams {
        file_path: file_path.to_string(),
        start_position: position,
        volume: vol,
        trim_start,
        trim_end,
        loop_region,
        fade_in: None,
        fade_out: None,
    })
}

pub fn stop_library_sound(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.lock_audio();
    audio.stop_library_sound();
    Ok(())
}

// --- Mute actions ---

pub fn toggle_master_mute(container: &AppStateContainer) -> Result<bool, String> {
    let mut audio = container.lock_audio();
    Ok(audio.toggle_master_mute())
}

pub fn toggle_track_mute(container: &AppStateContainer, track_id: &str) -> Result<bool, String> {
    let mut audio = container.lock_audio();
    Ok(audio.toggle_track_mute(track_id))
}

pub fn toggle_clip_mute(container: &AppStateContainer, clip_id: &str) -> Result<bool, String> {
    let mut audio = container.lock_audio();
    Ok(audio.toggle_clip_mute(clip_id))
}

pub fn toggle_comp_mute(container: &AppStateContainer, comp_id: &str) -> Result<bool, String> {
    let track_ids = {
        let state = with_db(container, |conn| database::load_full_state(conn))?;
        state.groups.iter()
            .flat_map(|g| g.comps.iter())
            .find(|c| c.id == comp_id)
            .map(|c| c.tracks.iter().map(|t| t.id.clone()).collect::<Vec<_>>())
            .ok_or_else(|| format!("Comp not found: {}", comp_id))?
    };
    let mut audio = container.lock_audio();
    Ok(audio.toggle_comp_mute(&track_ids))
}

pub fn get_mute_state(container: &AppStateContainer) -> Result<serde_json::Value, String> {
    let audio = container.lock_audio();
    let (master, tracks, clips) = audio.get_mute_snapshot();
    Ok(serde_json::json!({
        "master": master,
        "tracks": tracks,
        "clips": clips,
    }))
}

pub fn preload_peaks(container: &AppStateContainer) -> Result<usize, String> {
    let paths = {
        let conn = container.db.lock().map_err(|e| e.to_string())?;
        library::get_all_file_paths(&conn)?
    };
    let mut loaded = 0;
    for path in paths.iter() {
        match crate::waveform::extract_peaks(path, 1200, None, None, &container.peak_cache) {
            Ok(_) => loaded += 1,
            Err(e) => log::warn!("Failed to preload peaks for {}: {}", path, e),
        }
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::{hash_file, sanitize_volume};

    #[test]
    fn sanitize_volume_clamps_and_rejects_non_finite() {
        assert_eq!(sanitize_volume(0.5), Ok(0.5));
        assert_eq!(sanitize_volume(-1e300), Ok(0.0));
        assert_eq!(sanitize_volume(2.0), Ok(1.0));
        assert!(sanitize_volume(f64::NAN).is_err());
        assert!(sanitize_volume(f64::INFINITY).is_err());
    }

    #[test]
    fn hash_file_matches_known_sha256_vector() {
        // NIST test vector: SHA-256("abc")
        let dir = std::env::temp_dir();
        let path = dir.join(format!("lutedrop_hash_file_test_{}.txt", std::process::id()));
        std::fs::write(&path, b"abc").expect("write temp file");

        let result = hash_file(path.to_str().unwrap());

        let _ = std::fs::remove_file(&path);

        assert_eq!(
            result,
            Ok("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string())
        );
    }

    #[test]
    fn hash_file_missing_path_errs() {
        let result = hash_file("Z:\\this\\path\\does\\not\\exist\\lutedrop_missing.bin");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to open file for hashing"));
    }
}
