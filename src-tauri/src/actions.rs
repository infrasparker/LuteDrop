use std::collections::HashMap;
use std::sync::Arc;

use crate::audio::util::{is_audio_path, is_file_usable};
use crate::database;
use crate::database::{configurations, hierarchy, library, DbError};
use crate::autotag::client::{AnalyzerClient, HttpAnalyzer};
use crate::autotag::scry;
use crate::database::autotag_db;
use crate::database::settings::{self as broadcast_settings, BroadcastSettings};
use crate::database::settings as autotag_settings;
use crate::models::{
    AppState, AutoTagReport, AutoTagSettings, BroadcastStatus, BulkImportProgress, EntryTagScores, EntryType,
    FixMissingReport, LibraryEntry, LibraryEntryUsage, MissingEntryCandidates, MoodResetImpact, MoodVocabulary, Role, SceneConfig, Track, TrackUpdates, ScryHit,
    ScryPoint, EnergyPreview};
use crate::relink;
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

    let mut new_entry_id = None;
    let state = mutate_and_reload(container, |conn| {
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

        new_entry_id = Some(entry_id);
        Ok(())
    })?;

    if let Some(id) = new_entry_id {
        if is_music(&entry_type) {
            container.autotag.queue_entries(vec![id]);
        }
    }
    Ok(state)
}

/// Saves an entry's full tag list. `seen_auto_tags` is the set of auto tags
/// the editor showed; see `library::set_entry_tags`.
pub fn update_library_entry_tags(
    container: &AppStateContainer,
    entry_id: &str,
    tags: Vec<String>,
    seen_auto_tags: Option<Vec<String>>,
) -> Result<AppState, String> {
    let state = mutate_and_reload(container, |conn| {
        library::set_entry_tags(conn, entry_id, &tags, seen_auto_tags.as_deref())
    })?;
    request_rescore_if_enabled(container);
    Ok(state)
}

pub fn bulk_edit_library_entry_tags(
    container: &AppStateContainer,
    entry_ids: Vec<String>,
    add: Vec<String>,
    remove: Vec<String>,
) -> Result<AppState, String> {
    let state = mutate_and_reload(container, |conn| {
        library::bulk_edit_entry_tags(conn, &entry_ids, &add, &remove)
    })?;
    request_rescore_if_enabled(container);
    Ok(state)
}

pub fn keep_auto_tags(
    container: &AppStateContainer,
    entry_ids: Vec<String>,
    tags: Option<Vec<String>>,
) -> Result<AppState, String> {
    let state = mutate_and_reload(container, |conn| {
        library::keep_auto_tags(conn, &entry_ids, tags.as_deref())
    })?;
    request_rescore_if_enabled(container);
    Ok(state)
}

pub fn reject_auto_tags(
    container: &AppStateContainer,
    entry_ids: Vec<String>,
    tags: Option<Vec<String>>,
) -> Result<AppState, String> {
    let state = mutate_and_reload(container, |conn| {
        library::reject_auto_tags(conn, &entry_ids, tags.as_deref())
    })?;
    request_rescore_if_enabled(container);
    Ok(state)
}

/// Clears the user's yes or no vote on the named tags of the given entries,
/// so the analyzer decides them again.
pub fn reset_entry_tag_votes(
    container: &AppStateContainer,
    entry_ids: Vec<String>,
    tags: Vec<String>,
) -> Result<AppState, String> {
    let state = mutate_and_reload(container, |conn| {
        library::reset_entry_tag_votes(conn, &entry_ids, &tags)
    })?;
    request_rescore_if_enabled(container);
    Ok(state)
}

/// Sets the user's role on the given entries. `None` clears it ("use auto"),
/// so the analyzer's role shows.
pub fn set_entry_role(
    container: &AppStateContainer,
    entry_ids: Vec<String>,
    role: Option<Role>,
) -> Result<AppState, String> {
    let state = mutate_and_reload(container, |conn| {
        library::set_user_role(conn, &entry_ids, role)
    })?;
    request_rescore_if_enabled(container);
    Ok(state)
}

/// Sets the user's level of one mood on the given music entries (`None` goes
/// back to the analyzer's suggestion) and marks each one reviewed now. The
/// level must be 0, 0.25, 0.5, 0.75 or 1. No re-score: levels do not train
/// anything yet (learned mood heads are a later release).
pub fn set_entry_mood_level(
    container: &AppStateContainer,
    entry_ids: Vec<String>,
    mood: String,
    level: Option<f64>,
) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| library::set_user_mood_level(conn, &entry_ids, &mood, level))
}

/// Saves the mood vocabulary from Settings → Moods (see
/// `library::save_mood_vocabulary`): removed moods lose their rows, changed
/// prompts wait for Recalibrate.
pub fn set_mood_vocabulary(container: &AppStateContainer, vocabulary: MoodVocabulary) -> Result<AppState, String> {
    mutate_and_reload(container, |conn| library::save_mood_vocabulary(conn, &vocabulary).map(|_| ()))
}

/// What Reset to defaults would delete, for its confirm dialog.
pub fn mood_reset_impact(container: &AppStateContainer) -> Result<MoodResetImpact, String> {
    with_db(container, library::mood_reset_impact)
}

/// Reset to defaults (see `library::reset_mood_vocabulary`): writes the
/// shipped vocabulary even over an unreadable one. Moods whose prompts
/// differ from the calibration then need a recalibration.
pub fn reset_mood_vocabulary(container: &AppStateContainer) -> Result<AppState, String> {
    mutate_and_reload(container, library::reset_mood_vocabulary)
}

/// Recalibrate: clears the mood calibration, every analyzer mood value and
/// the stored scores (user levels stay), then starts an analysis run, which
/// takes a new calibration from the current prompts. Refused while
/// auto-tagging is off, since nothing could rebuild what it clears.
pub fn recalibrate_moods(container: &AppStateContainer) -> Result<AppState, String> {
    if !autotag_enabled(container) {
        return Err("Turn on auto-tagging first: the analyzer rebuilds the calibration".to_string());
    }
    let state = mutate_and_reload(container, library::clear_mood_analysis)?;
    container.autotag.analyze_library();
    Ok(state)
}

/// Approves one entry in Review: its shown role becomes the user's and it is
/// marked reviewed, atomically. Mood suggestions stay suggestions. Re-scores when that gave the entry
/// a user role, since user roles train the Combat gate.
pub fn approve_entry_review(container: &AppStateContainer, entry_id: &str) -> Result<AppState, String> {
    let role_changed = with_db(container, |conn| library::approve_entry_review(conn, entry_id))?;
    if role_changed {
        request_rescore_if_enabled(container);
    }
    load_state(container)
}

pub fn update_library_entry_type(
    container: &AppStateContainer,
    entry_id: &str,
    entry_type: &str,
) -> Result<AppState, String> {
    let mut became_music = false;
    let state = mutate_and_reload(container, |conn| {
        became_music = library::update_library_entry_type(conn, entry_id, entry_type)?;
        Ok(())
    })?;
    if became_music && autotag_enabled(container) {
        container.autotag.queue_entries(vec![entry_id.to_string()]);
    }
    Ok(state)
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
    // Ids of the entries this import added, handed to the auto-tag worker at the end.
    let mut new_ids: Vec<String> = Vec::new();

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
                new_ids.push(id.clone());
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

    if is_music(&entry_type) {
        container.autotag.queue_entries(new_ids);
    }
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

/// True if an import with this entry type creates music entries.
fn is_music(entry_type: &str) -> bool {
    matches!(EntryType::from_str(entry_type), EntryType::Music)
}

// --- Auto-tagging ---

/// True if auto-tagging is turned on. A settings read error counts as off.
fn autotag_enabled(container: &AppStateContainer) -> bool {
    with_db(container, autotag_settings::load_autotag)
        .map(|s| s.enabled)
        .unwrap_or(false)
}

/// Asks the worker to re-score after a tag change, if auto-tagging is on.
fn request_rescore_if_enabled(container: &AppStateContainer) {
    if autotag_enabled(container) {
        container.autotag.request_rescore();
    }
}

/// Queues every entry whose analysis is missing, failed, or stale. Called at
/// app start; does nothing while auto-tagging is off.
pub fn queue_unanalyzed_on_startup(container: &AppStateContainer) {
    if autotag_enabled(container) {
        container.autotag.analyze_library();
    }
}

pub fn get_autotag_settings(container: &AppStateContainer) -> Result<AutoTagSettings, String> {
    with_db(container, autotag_settings::load_autotag)
}

/// Saves the connection settings. Turning auto-tagging on starts analyzing
/// the library right away.
pub fn save_autotag_settings(container: &AppStateContainer, settings: AutoTagSettings) -> Result<(), String> {
    let was_enabled = autotag_enabled(container);
    with_db(container, |conn| autotag_settings::save_autotag(conn, &settings))?;
    if settings.enabled && !was_enabled {
        container.autotag.analyze_library();
    }
    Ok(())
}

pub fn get_scene_config(container: &AppStateContainer) -> Result<SceneConfig, String> {
    with_db(container, autotag_settings::load_scene_config)
}

/// Saves the scene config and re-scores, since the vocabulary may have
/// changed.
pub fn save_scene_config(container: &AppStateContainer, config: SceneConfig) -> Result<(), String> {
    with_db(container, |conn| autotag_settings::save_scene_config(conn, &config))?;
    request_rescore_if_enabled(container);
    Ok(())
}

/// Calls the analyzer's `/health` with `settings`, which need not be saved.
/// Blocks for up to a few seconds.
pub fn test_autotag_connection(settings: &AutoTagSettings) -> Result<String, String> {
    HttpAnalyzer::new(&settings.analyzer_url, &settings.token).health()
}

/// Starts analyzing every music entry, including those already embedded, so
/// the library recovers if the analyzer lost its cache. Progress arrives as
/// `autotag-progress` events.
pub fn analyze_library(container: &AppStateContainer) -> Result<(), String> {
    if !autotag_enabled(container) {
        return Err("Auto-tagging is turned off".to_string());
    }
    container.autotag.reanalyze_library();
    Ok(())
}

pub fn get_autotag_report(container: &AppStateContainer) -> AutoTagReport {
    container.autotag.last_report()
}

/// Music entries most like the selected ones. Blocks while the analyzer
/// works, so callers run it off the UI thread.
pub fn similar_entries(container: &AppStateContainer, entry_ids: &[String], limit: u32) -> Result<Vec<ScryHit>, String> {
    let client = scry::client_for(&container.db)?;
    scry::similar(&container.db, &client, entry_ids, limit)
}

/// Music entries ranked by how well they match a text description. Blocks
/// while the analyzer works.
pub fn search_scry(container: &AppStateContainer, text: &str, limit: u32) -> Result<Vec<ScryHit>, String> {
    let client = scry::client_for(&container.db)?;
    scry::search(&container.db, &client, text, limit)
}

/// The top music entries for a set of mood prompts, by raw similarity,
/// without saving or recalibrating anything (Settings → Moods, Preview).
/// Blocks while the analyzer works.
pub fn preview_mood_prompts(container: &AppStateContainer, prompts: &[String]) -> Result<Vec<ScryHit>, String> {
    let client = scry::client_for(&container.db)?;
    scry::preview_prompts(&container.db, &client, prompts)
}

pub fn preview_energy_prompts(
    container: &AppStateContainer,
    positive: &[String],
    negative: &[String],
) -> Result<EnergyPreview, String> {
    let client = scry::client_for(&container.db)?;
    scry::preview_energy(&container.db, &client, positive, negative)
}

/// A 2D position for every analyzed music entry. Blocks while the analyzer
/// works.
pub fn scry_map(container: &AppStateContainer) -> Result<Vec<ScryPoint>, String> {
    let client = scry::client_for(&container.db)?;
    scry::map(&container.db, &client)
}

/// The entry's stored scores, or `None` if it has not been scored.
pub fn get_entry_tag_scores(
    container: &AppStateContainer,
    entry_id: &str,
) -> Result<Option<EntryTagScores>, String> {
    with_db(container, |conn| autotag_db::load_scores(conn, entry_id))
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
        // The path column is UNIQUE, so check for an owner first and give a
        // readable message instead of a raw SQLite constraint error.
        if let Some((owner_id, owner_name)) = library::find_entry_name_by_file_path(conn, &new_file_path)? {
            if owner_id != entry_id {
                return Err(format!("That file is already linked to \"{owner_name}\""));
            }
        }
        library::update_library_entry_file_path(conn, entry_id, &new_file_path)?;
        Ok(())
    })?;

    // The relinked file is often a different encoding of the same recording
    // (e.g. a FLAC replacing an MP3), so the stored hash, size and duration
    // describe the OLD file. Refresh them from the new one. Trim, loop and
    // other markers are untouched. Failures here are only logged: a relink
    // must never fail just because a hash or duration could not be read.
    let usable = is_file_usable(&new_file_path);
    if usable {
        let refreshed = hash_file(&new_file_path).and_then(|hash| {
            let size = std::fs::metadata(&new_file_path)
                .map_err(|e| format!("Failed to read file size: {e}"))?
                .len() as i64;
            // None (undecodable file) leaves the old duration in place.
            let duration = crate::audio::util::probe_duration(&new_file_path);
            with_db(container, |conn| {
                library::update_library_entry_file_info(conn, entry_id, &hash, size, duration)
            })
        });
        if let Err(e) = refreshed {
            log::warn!("Relink: could not refresh file info for {entry_id}: {e}");
        }
    }

    // Evict old peak caches (in-memory + disk .peaks file)
    if old_file_path != new_file_path {
        let old = std::path::Path::new(&old_file_path);
        container.peak_cache.evict(old);
    }

    if usable {
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
        if !is_file_usable(&entry.file_path) {
            container.mark_entry_missing(&entry.id);
        }
    }
}

/// Builds the data for the "Fix missing files" dialog in one call, so the
/// expensive part (walking the search folder, possibly over a network share)
/// happens exactly once. Steps, in order: (1) load every entry; (2) re-check
/// each file and update the missing flags; (3) pick the search folder: the one
/// passed in (and remember it), else the remembered one, else a guess from the
/// entries whose files are still there; (4) if there is no usable folder,
/// return the missing entries with no candidates; (5) otherwise scan the
/// folder once; (6) match each missing entry against that scan, dropping empty
/// (zero-byte) files from its candidates.
pub fn find_missing_file_candidates(
    container: &AppStateContainer,
    folder: &str,
) -> Result<FixMissingReport, String> {
    // (1) Entries come back ordered by name, which is the order we report in.
    let entries = with_db(container, |conn| database::load_full_state(conn))?.library_entries;

    // (2) Refresh the missing flags so the main window agrees with this report.
    let mut present_paths: Vec<String> = Vec::new();
    let mut missing: Vec<&LibraryEntry> = Vec::new();
    for entry in &entries {
        if is_file_usable(&entry.file_path) {
            container.mark_entry_found(&entry.id);
            present_paths.push(entry.file_path.clone());
        } else {
            container.mark_entry_missing(&entry.id);
            missing.push(entry);
        }
    }

    // (3) Choose the search folder. Only a folder the user typed is saved;
    // a guessed one is not, so it keeps following the library around.
    let given = folder.trim();
    let search_root = if !given.is_empty() {
        with_db(container, |conn| broadcast_settings::save_fix_missing_root(conn, given))?;
        given.to_string()
    } else if let Some(saved) = with_db(container, |conn| broadcast_settings::load_fix_missing_root(conn))? {
        saved
    } else {
        relink::common_root(&present_paths)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    };

    // (4) No folder, or it is not reachable right now: not an error, the
    // dialog just shows the missing entries and lets the user pick a folder.
    let root_path = std::path::Path::new(&search_root);
    if search_root.is_empty() || !root_path.is_dir() {
        return Ok(FixMissingReport {
            search_root,
            search_root_exists: false,
            files_scanned: 0,
            entries: missing
                .into_iter()
                .map(|entry| MissingEntryCandidates { entry: entry.clone(), candidates: Vec::new() })
                .collect(),
        });
    }

    // (5) The one folder walk.
    let scanned = relink::index_files(root_path, &scan_folder_for_audio(&search_root)?);
    // Which file belongs to which entry, so used files can be flagged.
    let used: HashMap<String, String> = entries
        .iter()
        .map(|e| (e.file_path.clone(), e.name.clone()))
        .collect();

    // (6) Match each missing entry.
    let report_entries = missing
        .into_iter()
        .map(|entry| {
            let needles = relink::needles_for(&entry.file_path, &entry.name);
            let mut candidates = relink::match_candidates(&needles, &scanned, &used);
            // Only the few kept candidates are checked on disk, not every scanned file.
            candidates.retain(|c| is_file_usable(&c.file_path));
            MissingEntryCandidates { entry: entry.clone(), candidates }
        })
        .collect();

    Ok(FixMissingReport {
        search_root,
        search_root_exists: true,
        files_scanned: scanned.len() as u32,
        entries: report_entries,
    })
}

// --- Audio actions (no state return) ---

/// Check file existence for all clips in a track and update missing state.
fn update_clip_file_state(container: &AppStateContainer, track: &Track) {
    for clip in &track.clips {
        if !clip.file_path.is_empty() {
            if is_file_usable(&clip.file_path) {
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

/// `paused = true` seeks without un-pausing: the track ends up paused at
/// `position` (one audio lock, so nothing plays in between).
pub fn play_track_from_position(container: &AppStateContainer, track_id: &str, clip_id: Option<&str>, position: f64, paused: bool) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    update_clip_file_state(container, &track_config);
    let mut audio = container.lock_audio();
    ensure_track_ready(&mut audio, &track_config)?;
    audio.play_track_from(track_id, clip_id, position, paused)
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
    paused: bool,
) -> Result<(), String> {
    // Check the file is usable (exists and is not empty) before attempting playback
    if !is_file_usable(file_path) {
        container.mark_entry_missing(entry_id);
        return Err(format!("Audio file missing or empty: {}", file_path));
    }
    // File is usable — clear stale missing state if any
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
    }, paused)
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

// --- Broadcast (live stream) ---
//
// Every function here follows the lock order: settings are read or saved
// under the db lock, the stream is started/stopped under the audio lock, and
// the two are never held at once. Starting or stopping spawns/joins only our
// own broadcast threads, which never take either lock.

/// Build the public listener link from the user's `base` address: `None` if
/// `base` is blank; otherwise surrounding spaces and trailing slashes are
/// removed, `https://` is added when no scheme was given, and
/// `/listen/{key}` is appended.
pub fn public_url(base: &str, key: &str) -> Option<String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return None;
    }
    let with_scheme = if base.contains("://") {
        base.to_string()
    } else {
        format!("https://{base}")
    };
    Some(format!("{with_scheme}/listen/{key}"))
}

/// Read the saved broadcast settings (db lock only).
fn load_broadcast_settings(container: &AppStateContainer) -> Result<BroadcastSettings, String> {
    with_db(container, |conn| broadcast_settings::load_broadcast(conn))
}

/// Save the broadcast settings (db lock only).
fn save_broadcast_settings(container: &AppStateContainer, s: &BroadcastSettings) -> Result<(), String> {
    with_db(container, |conn| broadcast_settings::save_broadcast(conn, s))
}

/// Listener count, or `None` when the stream is off (audio lock only).
fn broadcast_listeners(container: &AppStateContainer) -> Option<usize> {
    container.lock_audio().broadcast_listeners()
}

/// Assemble what the Broadcast dialog shows. `listeners` is the live count
/// (`None` = not running), and it is also what `enabled` reports, so the
/// dialog always shows whether the stream is actually up.
fn broadcast_status(s: &BroadcastSettings, listeners: Option<usize>, error: Option<String>) -> BroadcastStatus {
    BroadcastStatus {
        enabled: listeners.is_some(),
        port: s.port,
        key: s.key.clone(),
        public_base: s.public_base.clone(),
        lan_url: format!("http://{}:{}/listen/{}", crate::server::get_local_ip(), s.port, s.key),
        public_url: public_url(&s.public_base, &s.key),
        listener_count: listeners.unwrap_or(0) as u32,
        error,
    }
}

/// Current broadcast settings, links and listener count. `error` carries why
/// the launch-time restore failed, if it did and the stream has not started
/// successfully since (see [`restore_broadcast_on_startup`]).
pub fn get_broadcast_status(container: &AppStateContainer) -> Result<BroadcastStatus, String> {
    let s = load_broadcast_settings(container)?;
    let (listeners, error) = {
        let audio = container.lock_audio();
        (audio.broadcast_listeners(), audio.last_broadcast_error())
    };
    Ok(broadcast_status(&s, listeners, error))
}

/// Turn the stream on or off and remember the choice.
///
/// If turning it on fails (most often: the port is in use) this still
/// returns `Ok`, with `enabled: false` and the reason in `error`, because
/// that is a state for the dialog to show rather than a broken command. The
/// saved flag is set to off, so the next launch does not retry a start that
/// just failed. `Err` means the database could not be read or written.
pub fn set_broadcast_enabled(container: &AppStateContainer, enabled: bool) -> Result<BroadcastStatus, String> {
    let mut s = load_broadcast_settings(container)?;
    let error = if enabled {
        let started = {
            let mut audio = container.lock_audio();
            if audio.broadcast_listeners().is_some() {
                Ok(()) // already on: nothing to do
            } else {
                audio.start_broadcast(s.port, s.key.clone())
            }
        };
        s.enabled = started.is_ok();
        started.err()
    } else {
        container.lock_audio().stop_broadcast();
        s.enabled = false;
        None
    };
    save_broadcast_settings(container, &s)?;
    Ok(broadcast_status(&s, broadcast_listeners(container), error))
}

/// Change the port and public address. The port cannot change while the
/// stream is running (the listener server is already bound to the old one),
/// so that case is refused; the public address only feeds the shown link
/// and can change any time.
pub fn set_broadcast_settings(
    container: &AppStateContainer,
    port: u16,
    public_base: String,
) -> Result<BroadcastStatus, String> {
    if port == 0 {
        return Err("Port must be between 1 and 65535".into());
    }
    let mut s = load_broadcast_settings(container)?;
    let listeners = broadcast_listeners(container);
    if listeners.is_some() && port != s.port {
        return Err("Turn Broadcast off before changing the port".into());
    }
    s.port = port;
    s.public_base = public_base;
    save_broadcast_settings(container, &s)?;
    Ok(broadcast_status(&s, listeners, None))
}

/// Replace the listener key with a new random one, so every link shared so
/// far stops working. If the stream is running it is restarted with the new
/// key at once, which disconnects anyone still on the old link. If that
/// restart fails the stream stays off, the saved flag is cleared, and the
/// reason is returned in `error` (as for [`set_broadcast_enabled`]).
pub fn regenerate_broadcast_key(container: &AppStateContainer) -> Result<BroadcastStatus, String> {
    let mut s = load_broadcast_settings(container)?;
    s.key = broadcast_settings::generate_listener_key();
    save_broadcast_settings(container, &s)?;

    let restarted = {
        let mut audio = container.lock_audio();
        if audio.broadcast_listeners().is_some() {
            audio.stop_broadcast();
            Some(audio.start_broadcast(s.port, s.key.clone()))
        } else {
            None
        }
    };
    let error = match restarted {
        Some(Err(e)) => {
            s.enabled = false;
            save_broadcast_settings(container, &s)?;
            Some(e)
        }
        _ => None,
    };
    Ok(broadcast_status(&s, broadcast_listeners(container), error))
}

/// At launch, turn the stream back on if it was on when the app closed.
///
/// Unlike a failed switch-on in the dialog, a failure here does NOT clear the
/// saved flag: the user asked for Broadcast to be on, and the cause (often a
/// port briefly held by something else) may be gone by the next launch. The
/// reason is kept on the audio engine instead, so the dialog shows it next to
/// a switch that reads off (`enabled` reflects whether the stream is actually
/// running).
pub fn restore_broadcast_on_startup(container: &AppStateContainer) {
    match load_broadcast_settings(container) {
        Ok(s) if s.enabled => match container.lock_audio().restore_broadcast(s.port, s.key.clone()) {
            Ok(()) => log::info!("Broadcast restored on port {}", s.port),
            Err(e) => log::warn!("Could not restore Broadcast on startup: {e}"),
        },
        Ok(_) => {}
        Err(e) => log::warn!("Could not read Broadcast settings on startup: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        find_missing_file_candidates, get_broadcast_status, hash_file, is_file_usable, public_url,
        relocate_library_entry, get_autotag_settings, save_autotag_settings,
        restore_broadcast_on_startup, sanitize_volume, scan_missing_files, set_broadcast_enabled,
        set_broadcast_settings, with_db,
    };
    use crate::database::{self, library};
    use crate::state::AppStateContainer;

    /// A container with an in-memory database and a real audio engine, for
    /// actions that touch both.
    fn test_container() -> AppStateContainer {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::init_schema(&conn).unwrap();
        // load_state needs an active configuration to exist.
        crate::database::ensure_default_configuration(&conn).unwrap();
        let dir = tempfile::tempdir().unwrap().keep();
        AppStateContainer::new(conn, crate::audio::AudioEngine::new().unwrap(), dir.join("peaks"), dir.join("db"))
    }

    #[test]
    fn enabling_autotag_starts_library_analysis_once() {
        let c = test_container();
        let mut settings = get_autotag_settings(&c).unwrap();
        assert!(!settings.enabled);

        save_autotag_settings(&c, settings.clone()).unwrap();
        assert_eq!(c.autotag.pending_analyze_requests(), 0, "saving while off starts nothing");

        settings.enabled = true;
        save_autotag_settings(&c, settings.clone()).unwrap();
        assert_eq!(c.autotag.pending_analyze_requests(), 1, "turning on starts an analysis");

        save_autotag_settings(&c, settings).unwrap();
        assert_eq!(c.autotag.pending_analyze_requests(), 0, "saving while already on starts nothing");
    }

    #[test]
    fn public_url_normalizes_base() {
        assert_eq!(public_url("", "k"), None);
        assert_eq!(public_url("   ", "k"), None);
        assert_eq!(public_url("https://s.example.org", "k").unwrap(), "https://s.example.org/listen/k");
        assert_eq!(public_url("https://s.example.org/", "k").unwrap(), "https://s.example.org/listen/k");
        assert_eq!(public_url("s.example.org/", "k").unwrap(), "https://s.example.org/listen/k");
        assert_eq!(public_url("http://10.0.0.5:5883", "k").unwrap(), "http://10.0.0.5:5883/listen/k");
    }

    #[test]
    fn set_broadcast_enabled_clears_flag_on_bind_failure() {
        let holder = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let c = test_container();
        set_broadcast_settings(&c, port, String::new()).unwrap();
        let status = set_broadcast_enabled(&c, true).unwrap();
        assert!(!status.enabled);
        assert!(status.error.as_deref().unwrap_or("").contains(&port.to_string()));
        let persisted = with_db(&c, |conn| crate::database::settings::load_broadcast(conn)).unwrap();
        assert!(!persisted.enabled, "a failed start must not be remembered as on");
    }

    #[test]
    fn failed_startup_restore_shows_error_and_keeps_saved_flag() {
        let holder = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let c = test_container();
        set_broadcast_settings(&c, port, String::new()).unwrap();
        with_db(&c, |conn| {
            let mut s = crate::database::settings::load_broadcast(conn)?;
            s.enabled = true;
            crate::database::settings::save_broadcast(conn, &s)
        })
        .unwrap();

        restore_broadcast_on_startup(&c);

        let status = get_broadcast_status(&c).unwrap();
        assert!(!status.enabled, "the stream must show as off when it could not start");
        assert!(
            status.error.as_deref().unwrap_or("").contains(&port.to_string()),
            "the startup failure must be shown, got {:?}",
            status.error
        );
        let persisted = with_db(&c, |conn| crate::database::settings::load_broadcast(conn)).unwrap();
        assert!(persisted.enabled, "a failed startup restore must keep the saved preference");
    }

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

    /// Inserts a "music" entry pointing at `path` and returns its id.
    fn add_entry(c: &AppStateContainer, path: &str, hash: &str, name: &str, size: i64) -> String {
        with_db(c, |conn| {
            Ok(library::insert_library_entry(conn, path, Some(hash), name, None, None, Some(size), "music")?)
        })
        .unwrap()
    }

    #[test]
    fn relocate_refreshes_hash_size_and_keeps_markers() {
        use sha2::{Digest, Sha256};
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old.wav").to_str().unwrap().to_string();
        let new = tmp.path().join("new.wav").to_str().unwrap().to_string();
        let bytes = b"0123456789abcdef";
        std::fs::write(&new, bytes).unwrap();

        let c = test_container();
        let id = add_entry(&c, &old, "stale", "Song", 1);
        with_db(&c, |conn| {
            conn.execute("UPDATE library_entries SET duration = 12.5 WHERE id = ?1", [&id])
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
        with_db(&c, |conn| library::update_library_entry_markers(conn, &id, Some(2.0), None, None, None)).unwrap();

        relocate_library_entry(&c, &id, new.clone()).unwrap();

        let state = with_db(&c, |conn| database::load_full_state(conn)).unwrap();
        let e = state.library_entries.iter().find(|e| e.id == id).unwrap();
        assert_eq!(e.file_path, new);
        assert_eq!(e.file_size, Some(16));
        assert_eq!(e.content_hash.as_deref(), Some(format!("{:x}", Sha256::digest(bytes)).as_str()));
        assert_eq!(e.trim_start, Some(2.0));
        assert_eq!(e.duration, Some(12.5), "an undecodable file must leave the old duration alone");
    }

    #[test]
    fn relocate_to_nonexistent_path_keeps_missing_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old.wav").to_str().unwrap().to_string();
        let nope = tmp.path().join("nope.wav").to_str().unwrap().to_string();
        let c = test_container();
        let id = add_entry(&c, &old, "h", "Song", 1);
        c.mark_entry_missing(&id);

        relocate_library_entry(&c, &id, nope.clone()).unwrap();

        assert!(c.missing_snapshot().contains(&id));
        let state = with_db(&c, |conn| database::load_full_state(conn)).unwrap();
        assert_eq!(state.library_entries.iter().find(|e| e.id == id).unwrap().file_path, nope);
    }

    #[test]
    fn zero_byte_file_is_not_usable_and_launch_scan_marks_it_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty.wav").to_str().unwrap().to_string();
        let ok = tmp.path().join("ok.wav").to_str().unwrap().to_string();
        std::fs::write(&empty, b"").unwrap();
        std::fs::write(&ok, b"data").unwrap();
        assert!(!is_file_usable(&empty));
        assert!(is_file_usable(&ok));

        let c = test_container();
        let empty_id = add_entry(&c, &empty, "h1", "Empty", 0);
        let _ok_id = add_entry(&c, &ok, "h2", "Ok", 4);
        scan_missing_files(&c);

        assert_eq!(c.missing_snapshot(), vec![empty_id]);
    }

    #[test]
    fn relocate_to_path_owned_by_other_entry_errs_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let a_path = tmp.path().join("a.wav").to_str().unwrap().to_string();
        let b_path = tmp.path().join("b.wav").to_str().unwrap().to_string();
        let c = test_container();
        let a = add_entry(&c, &a_path, "ha", "A", 1);
        let _b = add_entry(&c, &b_path, "hb", "B", 1);

        let err = relocate_library_entry(&c, &a, b_path).unwrap_err();

        assert!(err.contains("already linked to"), "got {err}");
        assert!(err.contains("\"B\""), "got {err}");
        let state = with_db(&c, |conn| database::load_full_state(conn)).unwrap();
        assert_eq!(state.library_entries.iter().find(|e| e.id == a).unwrap().file_path, a_path);
    }

    // --- find_missing_file_candidates ---

    use crate::models::MatchKind;

    fn s(p: &std::path::Path) -> String {
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn report_lists_missing_entries_with_candidates_and_skips_present() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("keep.wav"), b"data").unwrap();
        std::fs::write(lib.join("Battle Theme.opus"), b"data").unwrap();
        std::fs::write(lib.join("01 battle theme.opus"), b"data").unwrap();
        let old = tmp.path().join("old").join("battle theme.mp3");

        let c = test_container();
        add_entry(&c, &s(&lib.join("keep.wav")), "h1", "Keep", 4);
        let missing_id = add_entry(&c, &s(&old), "h2", "Battle Theme", 9);

        let report = find_missing_file_candidates(&c, &s(&lib)).unwrap();

        assert_eq!(report.entries.len(), 1);
        let e = &report.entries[0];
        assert_eq!(e.entry.name, "Battle Theme");
        assert_eq!(e.candidates[0].kind, MatchKind::Exact);
        assert_eq!(e.candidates[0].relative_path, "Battle Theme.opus");
        assert_eq!(e.candidates[1].kind, MatchKind::Contains);
        assert_eq!(report.search_root, s(&lib));
        assert!(report.search_root_exists);
        assert_eq!(report.files_scanned, 3);
        assert_eq!(c.missing_snapshot(), vec![missing_id]);
    }

    #[test]
    fn zero_byte_file_counts_as_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty.wav");
        std::fs::write(&empty, b"").unwrap();
        let c = test_container();
        let id = add_entry(&c, &s(&empty), "h", "Empty", 0);

        let report = find_missing_file_candidates(&c, &s(tmp.path())).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].entry.id, id);
        assert!(c.missing_snapshot().contains(&id));
    }

    #[test]
    fn default_root_derives_from_present_entries_and_is_not_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let music = tmp.path().join("music");
        for album in ["A", "B"] {
            std::fs::create_dir_all(music.join(album)).unwrap();
            std::fs::write(music.join(album).join("x.wav"), b"data").unwrap();
        }
        let c = test_container();
        add_entry(&c, &s(&music.join("A").join("x.wav")), "h1", "One", 4);
        add_entry(&c, &s(&music.join("B").join("x.wav")), "h2", "Two", 4);

        let report = find_missing_file_candidates(&c, "").unwrap();

        assert_eq!(report.search_root, s(&music));
        let saved = with_db(&c, |conn| crate::database::settings::load_fix_missing_root(conn)).unwrap();
        assert_eq!(saved, None);
    }

    #[test]
    fn given_root_is_remembered_and_used_next_time() {
        let tmp = tempfile::tempdir().unwrap();
        let x = tmp.path().join("x");
        std::fs::create_dir_all(&x).unwrap();
        let c = test_container();

        find_missing_file_candidates(&c, &s(&x)).unwrap();
        let report = find_missing_file_candidates(&c, "").unwrap();

        assert_eq!(report.search_root, s(&x));
        assert!(report.search_root_exists);
    }

    #[test]
    fn report_for_unreachable_root_is_empty_not_error() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("does-not-exist");
        let c = test_container();
        add_entry(&c, &s(&tmp.path().join("old.mp3")), "h", "Song", 1);

        let report = find_missing_file_candidates(&c, &s(&gone)).unwrap();

        assert!(!report.search_root_exists);
        assert_eq!(report.files_scanned, 0);
        assert_eq!(report.entries.len(), 1);
        assert!(report.entries[0].candidates.is_empty());
    }

    #[test]
    fn candidate_owned_by_other_entry_is_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let song = lib.join("song.opus");
        std::fs::write(&song, b"data").unwrap();
        let c = test_container();
        add_entry(&c, &s(&song), "h1", "Other", 4);
        add_entry(&c, &s(&tmp.path().join("old").join("song.mp3")), "h2", "Mine", 1);

        let report = find_missing_file_candidates(&c, &s(&lib)).unwrap();

        assert_eq!(report.entries.len(), 1);
        let cand = &report.entries[0].candidates[0];
        assert_eq!(cand.used_by_entry_name.as_deref(), Some("Other"));
    }

    #[test]
    fn zero_byte_candidates_are_dropped() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("song.opus"), b"").unwrap();
        std::fs::write(lib.join("song.flac"), b"data").unwrap();
        let c = test_container();
        add_entry(&c, &s(&tmp.path().join("old").join("song.mp3")), "h", "Song", 1);

        let report = find_missing_file_candidates(&c, &s(&lib)).unwrap();

        let rels: Vec<&str> = report.entries[0].candidates.iter().map(|c| c.relative_path.as_str()).collect();
        assert_eq!(rels, vec!["song.flac"]);
    }

    #[test]
    fn entry_with_symbol_only_name_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = tmp.path().join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("song.opus"), b"data").unwrap();
        let c = test_container();
        add_entry(&c, &s(&tmp.path().join("old").join("---.mp3")), "h", "---", 1);

        let report = find_missing_file_candidates(&c, &s(&lib)).unwrap();

        assert_eq!(report.entries.len(), 1);
        assert!(report.entries[0].candidates.is_empty());
    }
}
