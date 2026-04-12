use serde::Serialize;
use tauri::State;
use std::sync::Arc;
use crate::actions;
use crate::models::AppState;
use crate::state::AppStateContainer;
use crate::waveform;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformData {
    pub peaks: Vec<f32>,
    pub duration: f64,
}

#[tauri::command]
pub fn load_data(state: State<'_, Arc<AppStateContainer>>) -> Result<AppState, String> {
    actions::load_data(&state)
}

#[tauri::command]
pub fn add_group(state: State<'_, Arc<AppStateContainer>>, name: String) -> Result<AppState, String> {
    actions::add_group(&state, name)
}

#[tauri::command]
pub fn delete_group(state: State<'_, Arc<AppStateContainer>>, group_id: String) -> Result<AppState, String> {
    actions::delete_group(&state, &group_id)
}

#[tauri::command]
pub fn rename_group(state: State<'_, Arc<AppStateContainer>>, group_id: String, name: String) -> Result<AppState, String> {
    actions::rename_group(&state, &group_id, name)
}

#[tauri::command]
pub fn rename_comp(state: State<'_, Arc<AppStateContainer>>, comp_id: String, name: String) -> Result<AppState, String> {
    actions::rename_comp(&state, &comp_id, name)
}

#[tauri::command]
pub fn add_comp(state: State<'_, Arc<AppStateContainer>>, group_id: String, name: String) -> Result<AppState, String> {
    actions::add_comp(&state, &group_id, name)
}

#[tauri::command]
pub fn delete_comp(state: State<'_, Arc<AppStateContainer>>, group_id: String, comp_id: String) -> Result<AppState, String> {
    actions::delete_comp(&state, &group_id, &comp_id)
}

#[tauri::command]
pub fn add_track(state: State<'_, Arc<AppStateContainer>>, comp_id: String, name: String) -> Result<AppState, String> {
    actions::add_track(&state, &comp_id, name)
}

#[tauri::command]
pub fn delete_track(state: State<'_, Arc<AppStateContainer>>, comp_id: String, track_id: String) -> Result<AppState, String> {
    actions::delete_track(&state, &comp_id, &track_id)
}

#[tauri::command]
pub fn update_track(state: State<'_, Arc<AppStateContainer>>, comp_id: String, track_id: String, updates: serde_json::Value) -> Result<AppState, String> {
    actions::update_track(&state, &comp_id, &track_id, updates)
}

#[tauri::command]
pub fn add_clip(state: State<'_, Arc<AppStateContainer>>, track_id: String, name: String, file_path: String) -> Result<AppState, String> {
    actions::add_clip(&state, &track_id, name, file_path)
}

#[tauri::command]
pub fn delete_clip(state: State<'_, Arc<AppStateContainer>>, track_id: String, clip_id: String) -> Result<AppState, String> {
    actions::delete_clip(&state, &track_id, &clip_id)
}

#[tauri::command]
pub fn play_track(state: State<'_, Arc<AppStateContainer>>, track_id: String) -> Result<(), String> {
    actions::play_track_audio(&state, &track_id)
}

#[tauri::command]
pub fn play_track_from(state: State<'_, Arc<AppStateContainer>>, track_id: String, position: f64) -> Result<(), String> {
    actions::play_track_from_position(&state, &track_id, position)
}

#[tauri::command]
pub fn pause_track(state: State<'_, Arc<AppStateContainer>>, track_id: String) -> Result<(), String> {
    actions::pause_track_audio(&state, &track_id)
}

#[tauri::command]
pub fn stop_track(state: State<'_, Arc<AppStateContainer>>, track_id: String) -> Result<(), String> {
    actions::stop_track_audio(&state, &track_id)
}

#[tauri::command]
pub fn set_master_volume(state: State<'_, Arc<AppStateContainer>>, volume: f64) -> Result<(), String> {
    actions::set_master_volume_action(&state, volume)
}

#[tauri::command]
pub fn set_track_volume(state: State<'_, Arc<AppStateContainer>>, track_id: String, volume: f64) -> Result<(), String> {
    actions::set_track_volume_action(&state, &track_id, volume)
}

#[tauri::command]
pub fn set_clip_volume(state: State<'_, Arc<AppStateContainer>>, clip_id: String, volume: f64) -> Result<(), String> {
    actions::set_clip_volume_action(&state, &clip_id, volume)
}

#[tauri::command]
pub fn activate_comp(state: State<'_, Arc<AppStateContainer>>, comp_id: String) -> Result<(), String> {
    actions::activate_comp_audio(&state, &comp_id)
}

#[tauri::command]
pub fn stop_comp(state: State<'_, Arc<AppStateContainer>>, comp_id: String) -> Result<(), String> {
    actions::stop_comp_audio(&state, &comp_id)
}

#[tauri::command]
pub fn stop_all(state: State<'_, Arc<AppStateContainer>>) -> Result<(), String> {
    actions::stop_all_audio(&state)
}

#[tauri::command]
pub fn pause_all(state: State<'_, Arc<AppStateContainer>>) -> Result<(), String> {
    actions::pause_all_audio(&state)
}

#[tauri::command]
pub fn resume_all(state: State<'_, Arc<AppStateContainer>>) -> Result<(), String> {
    actions::resume_all_audio(&state)
}

#[tauri::command]
pub async fn get_waveform_data(
    state: State<'_, Arc<AppStateContainer>>,
    file_path: String,
    width: u32,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<WaveformData, String> {
    let state_clone = state.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<WaveformData, String> {
        let (peaks, duration) = waveform::extract_peaks(&file_path, width, start, end, &state_clone.peak_cache)?;
        Ok(WaveformData { peaks, duration })
    })
    .await
    .map_err(|e| format!("Waveform task failed: {}", e))?
}

#[tauri::command]
pub fn get_server_info() -> Result<serde_json::Value, String> {
    let ip = crate::server::get_local_ip();
    Ok(serde_json::json!({
        "ip": ip,
        "port": 3333
    }))
}
