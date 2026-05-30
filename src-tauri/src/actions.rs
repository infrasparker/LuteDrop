use std::sync::Arc;

use crate::audio::util::is_audio_path;
use crate::database;
use crate::database::{configurations, hierarchy, library};
use crate::models::{AppState, BulkImportProgress, LibraryEntryUsage, Track};
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
    Ok(state)
}

pub fn find_track_ref(container: &AppStateContainer, track_id: &str) -> Result<Track, String> {
    let state = with_db(container, |conn| database::load_full_state(conn))?;
    for group in &state.groups {
        for comp in &group.comps {
            if let Some(track) = comp.tracks.iter().find(|t| t.id == track_id) {
                return Ok(track.clone());
            }
        }
    }
    Err(format!("Track not found: {}", track_id))
}

pub fn find_comp_tracks(container: &AppStateContainer, comp_id: &str) -> Result<Vec<Track>, String> {
    let state = with_db(container, |conn| database::load_full_state(conn))?;
    for group in &state.groups {
        if let Some(comp) = group.comps.iter().find(|c| c.id == comp_id) {
            return Ok(comp.tracks.clone());
        }
    }
    Err(format!("Comp not found: {}", comp_id))
}

// --- Data mutation actions (return updated AppState) ---

pub fn load_data(container: &AppStateContainer) -> Result<AppState, String> {
    load_state(container)
}

pub fn add_group(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        let config_id = database::get_active_configuration_id(conn)?;
        hierarchy::insert_group(conn, &name, &config_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn delete_group(container: &AppStateContainer, group_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_group(conn, group_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn rename_group(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_group(conn, group_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn add_comp(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_comp(conn, group_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn delete_comp(container: &AppStateContainer, _group_id: &str, comp_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_comp(conn, comp_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn rename_comp(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_comp(conn, comp_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn add_track(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_track(conn, comp_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn delete_track(container: &AppStateContainer, _comp_id: &str, track_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_track(conn, track_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn update_track(container: &AppStateContainer, _comp_id: &str, track_id: &str, updates: serde_json::Value) -> Result<AppState, String> {
    with_db(container, |conn| {
        let obj = updates.as_object()
            .ok_or_else(|| "updates must be a JSON object".to_string())?;

        if let Some(v) = obj.get("name") {
            let name = v.as_str()
                .ok_or_else(|| "name must be a string".to_string())?;
            hierarchy::rename_track(conn, track_id, name)?;
        }

        if let Some(v) = obj.get("volume") {
            let volume = v.as_f64()
                .ok_or_else(|| "volume must be a number".to_string())?;
            hierarchy::update_track_volume(conn, track_id, volume)?;
        }

        // Handle fadeIn/fadeOut — need to read current values since update_track_fades takes both
        if obj.contains_key("fadeIn") || obj.contains_key("fadeOut") {
            // Read current fade values from DB
            let (current_fade_in, current_fade_out): (f64, f64) = conn
                .query_row(
                    "SELECT fade_in, fade_out FROM tracks WHERE id = ?1",
                    params![track_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|e| format!("Failed to read current fades: {e}"))?;

            let fade_in = if let Some(v) = obj.get("fadeIn") {
                v.as_f64().ok_or_else(|| "fadeIn must be a number".to_string())?
            } else {
                current_fade_in
            };

            let fade_out = if let Some(v) = obj.get("fadeOut") {
                v.as_f64().ok_or_else(|| "fadeOut must be a number".to_string())?
            } else {
                current_fade_out
            };

            hierarchy::update_track_fades(conn, track_id, fade_in, fade_out)?;
        }

        if let Some(v) = obj.get("mode") {
            let mode_obj = v.as_object()
                .ok_or_else(|| "mode must be a JSON object".to_string())?;

            let mode_type = mode_obj.get("type")
                .and_then(|t| t.as_str())
                .ok_or_else(|| "mode.type must be a string".to_string())?;

            let repeat = mode_obj.get("repeat").and_then(|r| r.as_bool());

            let transition = mode_obj.get("transition");
            let transition_type = transition
                .and_then(|t| t.get("type"))
                .and_then(|t| t.as_str());
            let gap_min = transition
                .and_then(|t| t.get("min"))
                .and_then(|t| t.as_f64());
            let gap_max = transition
                .and_then(|t| t.get("max"))
                .and_then(|t| t.as_f64());

            hierarchy::update_track_mode(
                conn,
                track_id,
                mode_type,
                repeat,
                transition_type,
                gap_min,
                gap_max,
            )?;
        }

        if let Some(v) = obj.get("clips") {
            let clips_arr = v.as_array()
                .ok_or_else(|| "clips must be an array".to_string())?;

            // Reorder clips based on the array order
            for (index, clip_val) in clips_arr.iter().enumerate() {
                let clip_id = clip_val.get("id")
                    .and_then(|id| id.as_str())
                    .ok_or_else(|| "each clip must have an 'id' string field".to_string())?;

                hierarchy::reorder_clip(conn, track_id, clip_id, index)?;
            }
        }

        Ok(())
    })?;
    load_state(container)
}

pub fn add_clip(container: &AppStateContainer, track_id: &str, library_entry_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_clip(conn, track_id, library_entry_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn delete_clip(container: &AppStateContainer, _track_id: &str, clip_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_clip(conn, clip_id)?;
        Ok(())
    })?;
    load_state(container)
}

// --- Configuration actions (return updated AppState) ---

pub fn create_configuration(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        let id = configurations::insert_configuration(conn, &name)?;
        configurations::set_active_configuration(conn, &id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn duplicate_configuration(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        let active_id = database::get_active_configuration_id(conn)?;
        let new_id = configurations::duplicate_configuration(conn, &active_id, &name)?;
        configurations::set_active_configuration(conn, &new_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn load_configuration(container: &AppStateContainer, config_id: String) -> Result<AppState, String> {
    // Stop all audio before switching
    {
        let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
        audio.stop_all();
    }
    with_db(container, |conn| {
        configurations::set_active_configuration(conn, &config_id)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn delete_configuration(container: &AppStateContainer, config_id: String) -> Result<AppState, String> {
    with_db(container, |conn| {
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
    })?;
    load_state(container)
}

pub fn rename_configuration(container: &AppStateContainer, config_id: String, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        configurations::rename_configuration(conn, &config_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}

// --- Library actions (return updated AppState) ---

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
    use sha2::{Digest, Sha256};
    use std::io::Read;

    // Compute SHA-256 hash if not provided
    let hash = if let Some(h) = content_hash {
        Some(h)
    } else {
        let mut file = std::fs::File::open(&file_path)
            .map_err(|e| format!("Failed to open file for hashing: {e}"))?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(e) => return Err(format!("Failed to read file for hashing: {e}")),
            }
        }
        Some(format!("{:x}", hasher.finalize()))
    };

    with_db(container, |conn| {
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
        )?;

        // Probe duration from the audio file if not already provided
        if duration.is_none() {
            if let Some(probed_duration) = crate::audio::util::probe_duration(&file_path) {
                let _ = library::update_library_entry_duration(conn, &entry_id, probed_duration, None);
            }
        }

        Ok(())
    })?;
    load_state(container)
}

pub fn update_library_entry_tags(
    container: &AppStateContainer,
    entry_id: &str,
    tags: Vec<String>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        // Remove all existing tags for this entry
        let existing_tags = library::get_tags_for_entry(conn, entry_id)?;
        for tag_name in &existing_tags {
            library::remove_tag_from_entry(conn, entry_id, tag_name)?;
        }

        // Add new tags
        for tag_name in &tags {
            library::add_tag_to_entry(conn, entry_id, tag_name)?;
        }

        Ok(())
    })?;
    load_state(container)
}

pub fn update_library_entry_type_action(
    container: &AppStateContainer,
    entry_id: &str,
    entry_type: &str,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_type(conn, entry_id, entry_type)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn update_library_entry_markers_action(
    container: &AppStateContainer,
    entry_id: &str,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_markers(conn, entry_id, trim_start, trim_end, loop_start, loop_end)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn update_library_entry_volume_action(
    container: &AppStateContainer,
    entry_id: &str,
    volume: f64,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_volume(conn, entry_id, volume)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn rename_library_entry_action(
    container: &AppStateContainer,
    entry_id: &str,
    name: String,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_name(conn, entry_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}

pub fn delete_library_entry_action(
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
        crate::audio::cache::evict(p);
        container.peak_cache.evict(p);
    }

    container.mark_entry_found(entry_id);
    load_state(container)
}

/// Removes cache files (transcode WAVs + peak data) no longer referenced by any
/// library entry. Called once at startup to reclaim disk.
pub fn gc_caches(container: &AppStateContainer) {
    let paths = with_db(container, |conn| library::get_all_file_paths(conn)).unwrap_or_default();
    crate::audio::cache::gc(&paths);
    container.peak_cache.gc(&paths);
}

pub fn get_library_entry_usage_action(
    container: &AppStateContainer,
    entry_id: &str,
) -> Result<Vec<LibraryEntryUsage>, String> {
    with_db(container, |conn| library::get_entry_usage(conn, entry_id))
}

/// Recursively walks `folder_path` and returns every audio file found inside,
/// filtered by `AUDIO_EXTENSIONS`. Unreadable sub-entries are skipped silently;
/// an unreadable root surfaces as an error.
pub fn scan_folder_for_audio_action(folder_path: &str) -> Result<Vec<String>, String> {
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
pub fn start_bulk_import_library_entries_action(
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
    use sha2::{Digest, Sha256};
    use std::io::Read;

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
        let hash = match std::fs::File::open(file_path) {
            Ok(mut file) => {
                let mut hasher = Sha256::new();
                let mut buf = [0u8; 65536];
                let mut read_err: Option<String> = None;
                loop {
                    match file.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => hasher.update(&buf[..n]),
                        Err(e) => {
                            read_err = Some(e.to_string());
                            break;
                        }
                    }
                }
                if let Some(e) = read_err {
                    log::warn!("Skipping {file_path} (read error: {e})");
                    skipped += 1;
                    emit_progress(&container, processed, imported, skipped, total);
                    continue;
                }
                format!("{:x}", hasher.finalize())
            }
            Err(e) => {
                log::warn!("Skipping {file_path} (open error: {e})");
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
                &conn,
                file_path,
                Some(&hash),
                &derived_name,
                None,
                None,
                None,
                &entry_type,
            ) {
                Ok(id) => id,
                Err(e) => {
                    if e.contains("UNIQUE constraint") {
                        return Ok(None); // file_path collision = skip
                    }
                    return Err(e);
                }
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

    // Evict old caches (in-memory + disk .peaks file, and any transcode WAV)
    if old_file_path != new_file_path {
        let old = std::path::Path::new(&old_file_path);
        container.peak_cache.evict(old);
        crate::audio::cache::evict(old);
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

pub fn play_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    update_clip_file_state(container, &track_config);
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track(track_id)
}

pub fn play_track_clip(container: &AppStateContainer, track_id: &str, clip_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    update_clip_file_state(container, &track_config);
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track_clip(track_id, clip_id)
}

pub fn play_track_from_position(container: &AppStateContainer, track_id: &str, position: f64) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    update_clip_file_state(container, &track_config);
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track_from(track_id, position)
}

pub fn pause_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.pause_track(track_id)
}

pub fn stop_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_track(track_id)
}

pub fn set_master_volume_action(container: &AppStateContainer, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_master_volume(volume);
    Ok(())
}

pub fn set_track_volume_action(container: &AppStateContainer, track_id: &str, volume: f64) -> Result<(), String> {
    // Persist to DB (cheap, no full state reload)
    {
        let conn = container.db.lock().map_err(|e| e.to_string())?;
        hierarchy::update_track_volume(&conn, track_id, volume)?;
    }
    // Update audio engine only if track is currently active
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if audio.has_track(track_id) {
        audio.set_track_volume(track_id, volume)?;
    }
    Ok(())
}

pub fn set_clip_volume_action(container: &AppStateContainer, clip_id: &str, volume: f64) -> Result<(), String> {
    // Persist to DB (cheap, no full state reload)
    {
        let conn = container.db.lock().map_err(|e| e.to_string())?;
        hierarchy::update_clip_volume(&conn, clip_id, volume)?;
    }
    // Update audio engine — set_clip_volume is a no-op if clip isn't in any active track
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_clip_volume(clip_id, volume)
}

pub fn activate_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    for track in &track_configs {
        update_clip_file_state(container, track);
    }
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track in &track_configs {
        if !audio.has_track(&track.id) {
            audio.setup_track(track)?;
        } else {
            audio.update_track_config(track);
        }
        audio.play_track(&track.id)?;
    }
    Ok(())
}

pub fn stop_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track in &track_configs {
        if audio.has_track(&track.id) {
            audio.stop_track(&track.id)?;
        }
    }
    Ok(())
}

pub fn pause_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track in &track_configs {
        if audio.has_track(&track.id) {
            audio.pause_track(&track.id)?;
        }
    }
    Ok(())
}

pub fn resume_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_tracks(container, comp_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track in &track_configs {
        if audio.has_track(&track.id) {
            audio.resume_track(&track.id)?;
        }
    }
    Ok(())
}

pub fn stop_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_all();
    Ok(())
}

pub fn pause_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.pause_all();
    Ok(())
}

pub fn resume_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
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
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    let vol = crate::audio::util::amplitude_to_decibels(audio.get_master_volume() * volume);
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
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_library_sound();
    Ok(())
}

// --- Mute actions ---

pub fn toggle_master_mute(container: &AppStateContainer) -> Result<bool, String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    Ok(audio.toggle_master_mute())
}

pub fn toggle_track_mute(container: &AppStateContainer, track_id: &str) -> Result<bool, String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    Ok(audio.toggle_track_mute(track_id))
}

pub fn toggle_clip_mute(container: &AppStateContainer, clip_id: &str) -> Result<bool, String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
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
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    Ok(audio.toggle_comp_mute(&track_ids))
}

pub fn get_mute_state(container: &AppStateContainer) -> Result<serde_json::Value, String> {
    let audio = container.audio.lock().map_err(|e| e.to_string())?;
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
