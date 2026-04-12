use serde::Serialize;
use tauri::State;
use uuid::Uuid;
use crate::models::{AppState, Group, Comp, Track, TrackMode, TrackTransition, Clip};
use crate::persistence;
use crate::state::AppStateContainer;
use crate::waveform;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformData {
    pub peaks: Vec<f32>,
    pub duration: f64,
}

/// Helper: persist current state to disk and return a clone.
fn save_and_return(container: &AppStateContainer, data: &AppState) -> Result<AppState, String> {
    persistence::save_to_disk(&container.save_path, data)?;
    Ok(data.clone())
}

// --- Helpers to find items by ID ---

fn find_group_mut<'a>(state: &'a mut AppState, group_id: &str) -> Result<&'a mut Group, String> {
    state
        .groups
        .iter_mut()
        .find(|g| g.id == group_id)
        .ok_or_else(|| format!("Group not found: {}", group_id))
}

fn find_comp_mut<'a>(state: &'a mut AppState, comp_id: &str) -> Result<&'a mut Comp, String> {
    for group in &mut state.groups {
        if let Some(comp) = group.comps.iter_mut().find(|c| c.id == comp_id) {
            return Ok(comp);
        }
    }
    Err(format!("Comp not found: {}", comp_id))
}

fn find_track_mut<'a>(state: &'a mut AppState, track_id: &str) -> Result<&'a mut Track, String> {
    for group in &mut state.groups {
        for comp in &mut group.comps {
            if let Some(track) = comp.tracks.iter_mut().find(|t| t.id == track_id) {
                return Ok(track);
            }
        }
    }
    Err(format!("Track not found: {}", track_id))
}

// --- Commands ---

#[tauri::command]
pub fn load_data(state: State<'_, AppStateContainer>) -> Result<AppState, String> {
    let data = state.data.read().map_err(|e| e.to_string())?;
    Ok(data.clone())
}

#[tauri::command]
pub fn add_group(
    state: State<'_, AppStateContainer>,
    name: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let group = Group {
        id: Uuid::new_v4().to_string(),
        name,
        comps: Vec::new(),
    };
    data.groups.push(group);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn delete_group(
    state: State<'_, AppStateContainer>,
    group_id: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    data.groups.retain(|g| g.id != group_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn rename_group(
    state: State<'_, AppStateContainer>,
    group_id: String,
    name: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, &group_id)?;
    group.name = name;
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn add_comp(
    state: State<'_, AppStateContainer>,
    group_id: String,
    name: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, &group_id)?;
    let comp = Comp {
        id: Uuid::new_v4().to_string(),
        name,
        tracks: Vec::new(),
    };
    group.comps.push(comp);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn delete_comp(
    state: State<'_, AppStateContainer>,
    group_id: String,
    comp_id: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, &group_id)?;
    group.comps.retain(|c| c.id != comp_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn add_track(
    state: State<'_, AppStateContainer>,
    comp_id: String,
    name: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, &comp_id)?;
    let track = Track {
        id: Uuid::new_v4().to_string(),
        name,
        clips: Vec::new(),
        volume: 1.0,
        mode: TrackMode::Sequence {
            repeat: true,
            transition: TrackTransition::Gapless,
        },
        fade_in: 0.0,
        fade_out: 0.0,
    };
    comp.tracks.push(track);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn delete_track(
    state: State<'_, AppStateContainer>,
    comp_id: String,
    track_id: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, &comp_id)?;
    comp.tracks.retain(|t| t.id != track_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn update_track(
    state: State<'_, AppStateContainer>,
    comp_id: String,
    track_id: String,
    updates: serde_json::Value,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;

    // First find the comp, then the track within it
    let comp = find_comp_mut(&mut data, &comp_id)?;
    let track = comp
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let obj = updates
        .as_object()
        .ok_or_else(|| "updates must be a JSON object".to_string())?;

    if let Some(v) = obj.get("name") {
        track.name = v
            .as_str()
            .ok_or_else(|| "name must be a string".to_string())?
            .to_string();
    }

    if let Some(v) = obj.get("volume") {
        track.volume = v
            .as_f64()
            .ok_or_else(|| "volume must be a number".to_string())?;
    }

    if let Some(v) = obj.get("fadeIn") {
        track.fade_in = v
            .as_f64()
            .ok_or_else(|| "fadeIn must be a number".to_string())?;
    }

    if let Some(v) = obj.get("fadeOut") {
        track.fade_out = v
            .as_f64()
            .ok_or_else(|| "fadeOut must be a number".to_string())?;
    }

    if let Some(v) = obj.get("mode") {
        track.mode = serde_json::from_value(v.clone())
            .map_err(|e| format!("Invalid mode: {}", e))?;
    }

    if let Some(v) = obj.get("clips") {
        track.clips = serde_json::from_value(v.clone())
            .map_err(|e| format!("Invalid clips: {}", e))?;
    }

    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn add_clip(
    state: State<'_, AppStateContainer>,
    track_id: String,
    name: String,
    file_path: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let track = find_track_mut(&mut data, &track_id)?;
    let clip = Clip {
        id: Uuid::new_v4().to_string(),
        name,
        file_path,
        volume: 1.0,
    };
    track.clips.push(clip);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

#[tauri::command]
pub fn delete_clip(
    state: State<'_, AppStateContainer>,
    track_id: String,
    clip_id: String,
) -> Result<AppState, String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;
    let track = find_track_mut(&mut data, &track_id)?;
    track.clips.retain(|c| c.id != clip_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(&state, &data)
}

// --- Audio commands ---

#[tauri::command]
pub fn play_track(
    state: State<'_, AppStateContainer>,
    track_id: String,
) -> Result<(), String> {
    // Read the track config from app state first
    let track_config = {
        let data = state.data.read().map_err(|e| e.to_string())?;
        find_track_ref(&data, &track_id)?.clone()
    };

    // Then lock audio engine
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;

    // Ensure the track is set up in the engine
    if !audio.has_track(&track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }

    audio.play_track(&track_id)
}

#[tauri::command]
pub fn pause_track(
    state: State<'_, AppStateContainer>,
    track_id: String,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.pause_track(&track_id)
}

#[tauri::command]
pub fn stop_track(
    state: State<'_, AppStateContainer>,
    track_id: String,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_track(&track_id)
}

#[tauri::command]
pub fn set_master_volume(
    state: State<'_, AppStateContainer>,
    volume: f64,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.set_master_volume(volume);
    Ok(())
}

#[tauri::command]
pub fn set_track_volume(
    state: State<'_, AppStateContainer>,
    track_id: String,
    volume: f64,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.set_track_volume(&track_id, volume)
}

#[tauri::command]
pub fn set_clip_volume(
    state: State<'_, AppStateContainer>,
    clip_id: String,
    volume: f64,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.set_clip_volume(&clip_id, volume)
}

#[tauri::command]
pub fn activate_comp(
    state: State<'_, AppStateContainer>,
    comp_id: String,
) -> Result<(), String> {
    // Read all track configs from the comp
    let track_configs: Vec<Track> = {
        let data = state.data.read().map_err(|e| e.to_string())?;
        let comp = find_comp_ref(&data, &comp_id)?;
        comp.tracks.clone()
    };

    // Lock audio and play each track
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
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

#[tauri::command]
pub fn stop_comp(
    state: State<'_, AppStateContainer>,
    comp_id: String,
) -> Result<(), String> {
    // Read track IDs from the comp
    let track_ids: Vec<String> = {
        let data = state.data.read().map_err(|e| e.to_string())?;
        let comp = find_comp_ref(&data, &comp_id)?;
        comp.tracks.iter().map(|t| t.id.clone()).collect()
    };

    // Lock audio and stop each track
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    for track_id in &track_ids {
        if audio.has_track(track_id) {
            audio.stop_track(track_id)?;
        }
    }

    Ok(())
}

#[tauri::command]
pub fn stop_all(state: State<'_, AppStateContainer>) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_all();
    Ok(())
}

// --- Waveform command ---

#[tauri::command]
pub async fn get_waveform_data(file_path: String, width: u32) -> Result<WaveformData, String> {
    // Run the CPU-intensive decode on a blocking thread so it doesn't freeze the UI
    tokio::task::spawn_blocking(move || -> Result<WaveformData, String> {
        let (peaks, duration) = waveform::extract_peaks(&file_path, width)?;
        Ok(WaveformData { peaks, duration })
    })
    .await
    .map_err(|e| format!("Waveform task failed: {}", e))?
}

// --- Immutable lookup helpers for audio commands ---

fn find_track_ref<'a>(state: &'a AppState, track_id: &str) -> Result<&'a Track, String> {
    for group in &state.groups {
        for comp in &group.comps {
            if let Some(track) = comp.tracks.iter().find(|t| t.id == track_id) {
                return Ok(track);
            }
        }
    }
    Err(format!("Track not found: {}", track_id))
}

fn find_comp_ref<'a>(state: &'a AppState, comp_id: &str) -> Result<&'a Comp, String> {
    for group in &state.groups {
        if let Some(comp) = group.comps.iter().find(|c| c.id == comp_id) {
            return Ok(comp);
        }
    }
    Err(format!("Comp not found: {}", comp_id))
}

/// Returns the current time as an ISO-8601-ish timestamp string (epoch millis).
fn chrono_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let millis = duration.as_millis();
    format!("{}", millis)
}
