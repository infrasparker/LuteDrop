use crate::models::{AppState, Group, Comp, Track, TrackMode, TrackTransition, Clip};
use crate::persistence;
use crate::state::AppStateContainer;
use uuid::Uuid;

// --- Helpers ---

fn save_and_return(container: &AppStateContainer, data: &AppState) -> Result<AppState, String> {
    persistence::save_to_disk(&container.save_path, data)?;
    Ok(data.clone())
}

fn find_group_mut<'a>(state: &'a mut AppState, group_id: &str) -> Result<&'a mut Group, String> {
    state.groups.iter_mut().find(|g| g.id == group_id)
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

pub fn find_track_ref<'a>(state: &'a AppState, track_id: &str) -> Result<&'a Track, String> {
    for group in &state.groups {
        for comp in &group.comps {
            if let Some(track) = comp.tracks.iter().find(|t| t.id == track_id) {
                return Ok(track);
            }
        }
    }
    Err(format!("Track not found: {}", track_id))
}

pub fn find_comp_ref<'a>(state: &'a AppState, comp_id: &str) -> Result<&'a Comp, String> {
    for group in &state.groups {
        if let Some(comp) = group.comps.iter().find(|c| c.id == comp_id) {
            return Ok(comp);
        }
    }
    Err(format!("Comp not found: {}", comp_id))
}

fn chrono_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_millis())
}

// --- Data mutation actions (return updated AppState) ---

pub fn load_data(container: &AppStateContainer) -> Result<AppState, String> {
    let data = container.data.read().map_err(|e| e.to_string())?;
    Ok(data.clone())
}

pub fn add_group(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = Group { id: Uuid::new_v4().to_string(), name, comps: Vec::new() };
    data.groups.push(group);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_group(container: &AppStateContainer, group_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    data.groups.retain(|g| g.id != group_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn rename_group(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, group_id)?;
    group.name = name;
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn add_comp(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, group_id)?;
    let comp = Comp { id: Uuid::new_v4().to_string(), name, tracks: Vec::new() };
    group.comps.push(comp);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_comp(container: &AppStateContainer, group_id: &str, comp_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let group = find_group_mut(&mut data, group_id)?;
    group.comps.retain(|c| c.id != comp_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn add_track(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, comp_id)?;
    let track = Track {
        id: Uuid::new_v4().to_string(),
        name,
        clips: Vec::new(),
        volume: 1.0,
        mode: TrackMode::Sequence { repeat: true, transition: TrackTransition::Gapless },
        fade_in: 0.0,
        fade_out: 0.0,
    };
    comp.tracks.push(track);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_track(container: &AppStateContainer, comp_id: &str, track_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, comp_id)?;
    comp.tracks.retain(|t| t.id != track_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn update_track(container: &AppStateContainer, comp_id: &str, track_id: &str, updates: serde_json::Value) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let comp = find_comp_mut(&mut data, comp_id)?;
    let track = comp.tracks.iter_mut().find(|t| t.id == track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    let obj = updates.as_object()
        .ok_or_else(|| "updates must be a JSON object".to_string())?;

    if let Some(v) = obj.get("name") {
        track.name = v.as_str()
            .ok_or_else(|| "name must be a string".to_string())?
            .to_string();
    }
    if let Some(v) = obj.get("volume") {
        track.volume = v.as_f64()
            .ok_or_else(|| "volume must be a number".to_string())?;
    }
    if let Some(v) = obj.get("fadeIn") {
        track.fade_in = v.as_f64()
            .ok_or_else(|| "fadeIn must be a number".to_string())?;
    }
    if let Some(v) = obj.get("fadeOut") {
        track.fade_out = v.as_f64()
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
    save_and_return(container, &data)
}

pub fn add_clip(container: &AppStateContainer, track_id: &str, name: String, file_path: String) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let track = find_track_mut(&mut data, track_id)?;
    let clip = Clip { id: Uuid::new_v4().to_string(), name, file_path, volume: 1.0 };
    track.clips.push(clip);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

pub fn delete_clip(container: &AppStateContainer, track_id: &str, clip_id: &str) -> Result<AppState, String> {
    let mut data = container.data.write().map_err(|e| e.to_string())?;
    let track = find_track_mut(&mut data, track_id)?;
    track.clips.retain(|c| c.id != clip_id);
    data.last_saved_at = Some(chrono_now());
    save_and_return(container, &data)
}

// --- Audio actions (no state return) ---

pub fn play_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let track_config = {
        let data = container.data.read().map_err(|e| e.to_string())?;
        find_track_ref(&data, track_id)?.clone()
    };
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track(track_id)
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
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_track_volume(track_id, volume)
}

pub fn set_clip_volume_action(container: &AppStateContainer, clip_id: &str, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_clip_volume(clip_id, volume)
}

pub fn activate_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs: Vec<Track> = {
        let data = container.data.read().map_err(|e| e.to_string())?;
        let comp = find_comp_ref(&data, comp_id)?;
        comp.tracks.clone()
    };
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
    let track_ids: Vec<String> = {
        let data = container.data.read().map_err(|e| e.to_string())?;
        let comp = find_comp_ref(&data, comp_id)?;
        comp.tracks.iter().map(|t| t.id.clone()).collect()
    };
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track_id in &track_ids {
        if audio.has_track(track_id) {
            audio.stop_track(track_id)?;
        }
    }
    Ok(())
}

pub fn stop_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_all();
    Ok(())
}
