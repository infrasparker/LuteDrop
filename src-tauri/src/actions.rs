use crate::database;
use crate::database::{hierarchy, library};
use crate::models::{AppState, Track};
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
    with_db(container, |conn| database::load_full_state(conn))
}

pub fn add_group(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_group(conn, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_group(container: &AppStateContainer, group_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_group(conn, group_id)?;
        database::load_full_state(conn)
    })
}

pub fn rename_group(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_group(conn, group_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn add_comp(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_comp(conn, group_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_comp(container: &AppStateContainer, _group_id: &str, comp_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_comp(conn, comp_id)?;
        database::load_full_state(conn)
    })
}

pub fn rename_comp(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_comp(conn, comp_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn add_track(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_track(conn, comp_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_track(container: &AppStateContainer, _comp_id: &str, track_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_track(conn, track_id)?;
        database::load_full_state(conn)
    })
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

        database::load_full_state(conn)
    })
}

pub fn add_clip(container: &AppStateContainer, track_id: &str, library_entry_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_clip(conn, track_id, library_entry_id)?;
        database::load_full_state(conn)
    })
}

pub fn delete_clip(container: &AppStateContainer, _track_id: &str, clip_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_clip(conn, clip_id)?;
        database::load_full_state(conn)
    })
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
        )?;

        // Probe duration from the audio file if not already provided
        if duration.is_none() {
            if let Some(probed_duration) = crate::audio::util::probe_duration(&file_path) {
                let _ = library::update_library_entry_duration(conn, &entry_id, probed_duration, None);
            }
        }

        database::load_full_state(conn)
    })
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

        database::load_full_state(conn)
    })
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
        database::load_full_state(conn)
    })
}

pub fn update_library_entry_volume_action(
    container: &AppStateContainer,
    entry_id: &str,
    volume: f64,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_volume(conn, entry_id, volume)?;
        database::load_full_state(conn)
    })
}

pub fn rename_library_entry_action(
    container: &AppStateContainer,
    entry_id: &str,
    name: String,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_name(conn, entry_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_library_entry_action(
    container: &AppStateContainer,
    entry_id: &str,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::delete_library_entry(conn, entry_id)?;
        database::load_full_state(conn)
    })
}

pub fn relocate_library_entry(
    container: &AppStateContainer,
    entry_id: &str,
    new_file_path: String,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_file_path(conn, entry_id, &new_file_path)?;
        database::load_full_state(conn)
    })
}

pub fn check_missing_files(container: &AppStateContainer) -> Result<Vec<String>, String> {
    with_db(container, |conn| {
        let state = database::load_full_state(conn)?;
        let missing: Vec<String> = state
            .library_entries
            .iter()
            .filter(|entry| !std::path::Path::new(&entry.file_path).exists())
            .map(|entry| entry.id.clone())
            .collect();
        Ok(missing)
    })
}

// --- Audio actions (no state return) ---

pub fn play_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
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

pub fn play_preview_audio(
    container: &AppStateContainer,
    file_path: &str,
    position: f64,
    volume: f64,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.play_preview(file_path, position, volume, trim_start, trim_end, loop_start, loop_end)
}

pub fn stop_preview_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_preview();
    Ok(())
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
