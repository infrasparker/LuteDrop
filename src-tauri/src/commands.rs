use serde::Serialize;
use tauri::State;
use std::sync::Arc;
use crate::actions;
use crate::models::{
    AppState, AutoTagReport, CaseImportReport, FlagBand, FlagKind, AutoTagSettings, BroadcastStatus, EntryTagScores, LibraryEntryUsage, MoodResetImpact, MoodVocabulary,
    Role, SceneConfig, TrackUpdates, ScryHit, ScryPoint, EnergyPreview, TuneResult};
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
pub fn delete_comp(state: State<'_, Arc<AppStateContainer>>, comp_id: String) -> Result<AppState, String> {
    actions::delete_comp(&state, &comp_id)
}

#[tauri::command]
pub fn add_track(state: State<'_, Arc<AppStateContainer>>, comp_id: String, name: String) -> Result<AppState, String> {
    actions::add_track(&state, &comp_id, name)
}

#[tauri::command]
pub fn delete_track(state: State<'_, Arc<AppStateContainer>>, track_id: String) -> Result<AppState, String> {
    actions::delete_track(&state, &track_id)
}

#[tauri::command]
pub fn update_track(state: State<'_, Arc<AppStateContainer>>, track_id: String, updates: TrackUpdates) -> Result<AppState, String> {
    actions::update_track(&state, &track_id, updates)
}

#[tauri::command]
pub fn add_clip(state: State<'_, Arc<AppStateContainer>>, track_id: String, library_entry_id: String) -> Result<AppState, String> {
    actions::add_clip(&state, &track_id, &library_entry_id)
}

#[tauri::command]
pub fn add_clips(state: State<'_, Arc<AppStateContainer>>, track_id: String, library_entry_ids: Vec<String>) -> Result<AppState, String> {
    actions::add_clips(&state, &track_id, &library_entry_ids)
}

#[tauri::command]
pub fn delete_clip(state: State<'_, Arc<AppStateContainer>>, track_id: String, clip_id: String) -> Result<AppState, String> {
    actions::delete_clip(&state, &track_id, &clip_id)
}

// --- Configuration commands ---

#[tauri::command]
pub fn create_configuration(state: State<'_, Arc<AppStateContainer>>, name: String) -> Result<AppState, String> {
    actions::create_configuration(&state, name)
}

#[tauri::command]
pub fn duplicate_configuration(state: State<'_, Arc<AppStateContainer>>, name: String) -> Result<AppState, String> {
    actions::duplicate_configuration(&state, name)
}

#[tauri::command]
pub fn load_configuration(state: State<'_, Arc<AppStateContainer>>, config_id: String) -> Result<AppState, String> {
    actions::load_configuration(&state, config_id)
}

#[tauri::command]
pub fn delete_configuration(state: State<'_, Arc<AppStateContainer>>, config_id: String) -> Result<AppState, String> {
    actions::delete_configuration(&state, config_id)
}

#[tauri::command]
pub fn rename_configuration(state: State<'_, Arc<AppStateContainer>>, config_id: String, name: String) -> Result<AppState, String> {
    actions::rename_configuration(&state, config_id, name)
}

#[tauri::command]
pub fn play_track(state: State<'_, Arc<AppStateContainer>>, track_id: String) -> Result<(), String> {
    actions::play_track_audio(&state, &track_id)
}

/// `clip_id` is optional: when the frontend omits it, the track's first clip
/// is picked (waveform click-to-play); the transport scrubber passes the
/// playing clip so a seek stays in that clip. `paused` (optional, default
/// false) seeks a paused track and leaves it paused.
#[tauri::command]
pub fn play_track_from(
    state: State<'_, Arc<AppStateContainer>>,
    track_id: String,
    clip_id: Option<String>,
    position: f64,
    paused: Option<bool>,
) -> Result<(), String> {
    actions::play_track_from_position(&state, &track_id, clip_id.as_deref(), position, paused.unwrap_or(false))
}

#[tauri::command]
pub fn play_track_clip(state: State<'_, Arc<AppStateContainer>>, track_id: String, clip_id: String) -> Result<(), String> {
    actions::play_track_clip(&state, &track_id, &clip_id)
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
    actions::set_master_volume(&state, volume)
}

#[tauri::command]
pub fn set_track_volume(state: State<'_, Arc<AppStateContainer>>, track_id: String, volume: f64) -> Result<(), String> {
    actions::set_track_volume(&state, &track_id, volume)
}

#[tauri::command]
pub fn set_clip_volume(state: State<'_, Arc<AppStateContainer>>, clip_id: String, volume: f64) -> Result<(), String> {
    actions::set_clip_volume(&state, &clip_id, volume)
}

#[tauri::command]
pub fn set_library_volume(state: State<'_, Arc<AppStateContainer>>, volume: f64) -> Result<(), String> {
    actions::set_library_volume(&state, volume)
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
pub fn pause_comp(state: State<'_, Arc<AppStateContainer>>, comp_id: String) -> Result<(), String> {
    actions::pause_comp_audio(&state, &comp_id)
}

#[tauri::command]
pub fn resume_comp(state: State<'_, Arc<AppStateContainer>>, comp_id: String) -> Result<(), String> {
    actions::resume_comp_audio(&state, &comp_id)
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
    entry_id: Option<String>,
) -> Result<WaveformData, String> {
    // Check file exists
    if !crate::audio::util::is_file_usable(&file_path) {
        if let Some(ref eid) = entry_id {
            state.mark_entry_missing(eid);
        }
        return Err(format!("Audio file not found: {}", file_path));
    }
    // File exists — clear stale missing state if any
    if let Some(ref eid) = entry_id {
        state.mark_entry_found(eid);
    }
    let state_clone = state.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<WaveformData, String> {
        let (peaks, duration) = waveform::extract_peaks(&file_path, width, start, end, &state_clone.peak_cache)?;
        Ok(WaveformData { peaks, duration })
    })
    .await
    .map_err(|e| format!("Waveform task failed: {}", e))?
}

#[tauri::command]
pub fn get_server_info(state: State<'_, Arc<AppStateContainer>>) -> Result<serde_json::Value, String> {
    let ip = crate::server::get_local_ip();
    let port = crate::server::REMOTE_PORT;
    Ok(serde_json::json!({
        "ip": ip,
        "port": port,
        "token": state.remote_token,
        "url": format!("http://{}:{}/?token={}", ip, port, state.remote_token),
    }))
}

// --- Library commands ---

#[tauri::command]
pub async fn import_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    file_path: String,
    content_hash: Option<String>,
    name: String,
    duration: Option<f64>,
    sample_rate: Option<i64>,
    file_size: Option<i64>,
    entry_type: String,
) -> Result<AppState, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        actions::import_library_entry(&container, file_path, content_hash, name, duration, sample_rate, file_size, entry_type)
    })
    .await
    .map_err(|e| format!("Import task failed: {e}"))?
}

#[tauri::command]
pub fn update_library_entry_tags(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    tags: Vec<String>,
    seen_auto_tags: Option<Vec<String>>,
) -> Result<AppState, String> {
    actions::update_library_entry_tags(&state, &entry_id, tags, seen_auto_tags)
}

#[tauri::command]
pub fn bulk_edit_library_entry_tags(
    state: State<'_, Arc<AppStateContainer>>,
    entry_ids: Vec<String>,
    add: Vec<String>,
    remove: Vec<String>,
) -> Result<AppState, String> {
    actions::bulk_edit_library_entry_tags(&state, entry_ids, add, remove)
}

#[tauri::command]
pub fn keep_auto_tags(
    state: State<'_, Arc<AppStateContainer>>,
    entry_ids: Vec<String>,
    tags: Option<Vec<String>>,
) -> Result<AppState, String> {
    actions::keep_auto_tags(&state, entry_ids, tags)
}

#[tauri::command]
pub fn set_entry_role(
    state: State<'_, Arc<AppStateContainer>>,
    entry_ids: Vec<String>,
    role: Option<Role>,
) -> Result<AppState, String> {
    actions::set_entry_role(&state, entry_ids, role)
}

#[tauri::command]
pub fn set_entry_mood_level(
    state: State<'_, Arc<AppStateContainer>>,
    entry_ids: Vec<String>,
    mood: String,
    level: Option<f64>,
) -> Result<AppState, String> {
    actions::set_entry_mood_level(&state, entry_ids, mood, level)
}

#[tauri::command]
pub fn set_mood_flag(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    mood: String,
    kind: FlagKind,
    band: Option<FlagBand>,
) -> Result<AppState, String> {
    actions::set_mood_flag(&state, entry_id, mood, kind, band)
}

#[tauri::command]
pub fn clear_mood_flag(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    mood: String,
) -> Result<AppState, String> {
    actions::clear_mood_flag(&state, entry_id, mood)
}

#[tauri::command]
pub fn keep_mood_flag(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    mood: String,
) -> Result<AppState, String> {
    actions::keep_mood_flag(&state, entry_id, mood)
}

#[tauri::command]
pub fn import_case_studies(
    state: State<'_, Arc<AppStateContainer>>,
    path: String,
) -> Result<CaseImportReport, String> {
    actions::import_case_studies(&state, path)
}

#[tauri::command]
pub fn set_mood_vocabulary(
    state: State<'_, Arc<AppStateContainer>>,
    vocabulary: MoodVocabulary,
) -> Result<AppState, String> {
    actions::set_mood_vocabulary(&state, vocabulary)
}

/// Runs Tune on a mood. The analyzer's search can take tens of seconds, so it
/// runs on a worker thread like the previews.
#[tauri::command]
pub async fn tune_mood(
    state: State<'_, Arc<AppStateContainer>>,
    mood_id: String,
    extra_words: Vec<String>,
    bank: Vec<String>,
) -> Result<TuneResult, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::tune_mood(&container, &mood_id, &extra_words, &bank))
        .await
        .map_err(|e| format!("Tune failed: {e}"))?
}

#[tauri::command]
pub fn accept_tuned_lines(
    state: State<'_, Arc<AppStateContainer>>,
    mood_id: String,
    lines_tuned_from: Vec<String>,
    lines: Vec<String>,
    holding_entry_ids: Vec<String>,
) -> Result<AppState, String> {
    actions::accept_tuned_lines(&state, mood_id, lines_tuned_from, lines, holding_entry_ids)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn accept_tuned_split(
    state: State<'_, Arc<AppStateContainer>>,
    mood_id: String,
    lines_tuned_from: Vec<String>,
    keep_lines: Vec<String>,
    new_id: String,
    new_name: String,
    new_lines: Vec<String>,
    move_entry_ids: Vec<String>,
    keep_holding_entry_ids: Vec<String>,
    new_holding_entry_ids: Vec<String>,
) -> Result<AppState, String> {
    actions::accept_tuned_split(
        &state,
        mood_id,
        lines_tuned_from,
        keep_lines,
        new_id,
        new_name,
        new_lines,
        move_entry_ids,
        keep_holding_entry_ids,
        new_holding_entry_ids,
    )
}

#[tauri::command]
pub fn mood_reset_impact(state: State<'_, Arc<AppStateContainer>>) -> Result<MoodResetImpact, String> {
    actions::mood_reset_impact(&state)
}

#[tauri::command]
pub fn reset_mood_vocabulary(state: State<'_, Arc<AppStateContainer>>) -> Result<AppState, String> {
    actions::reset_mood_vocabulary(&state)
}

#[tauri::command]
pub fn recalibrate_moods(state: State<'_, Arc<AppStateContainer>>) -> Result<AppState, String> {
    actions::recalibrate_moods(&state)
}

#[tauri::command]
pub fn reject_auto_tags(
    state: State<'_, Arc<AppStateContainer>>,
    entry_ids: Vec<String>,
    tags: Option<Vec<String>>,
) -> Result<AppState, String> {
    actions::reject_auto_tags(&state, entry_ids, tags)
}

/// Clears the user's vote on `tags` for each of `entry_ids`; see
/// `library::reset_entry_tag_votes`.
#[tauri::command]
pub fn reset_entry_tag_votes(
    state: State<'_, Arc<AppStateContainer>>,
    entry_ids: Vec<String>,
    tags: Vec<String>,
) -> Result<AppState, String> {
    actions::reset_entry_tag_votes(&state, entry_ids, tags)
}

#[tauri::command]
pub fn update_library_entry_type(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    entry_type: String,
) -> Result<AppState, String> {
    actions::update_library_entry_type(&state, &entry_id, &entry_type)
}

#[tauri::command]
pub fn update_library_entry_markers(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<AppState, String> {
    actions::update_library_entry_markers(&state, &entry_id, trim_start, trim_end, loop_start, loop_end)
}

#[tauri::command]
pub fn update_library_entry_volume(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    volume: f64,
) -> Result<AppState, String> {
    actions::update_library_entry_volume(&state, &entry_id, volume)
}

#[tauri::command]
pub fn rename_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    name: String,
) -> Result<AppState, String> {
    actions::rename_library_entry(&state, &entry_id, name)
}

#[tauri::command]
pub fn delete_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
) -> Result<AppState, String> {
    actions::delete_library_entry(&state, &entry_id)
}

#[tauri::command]
pub fn get_library_entry_usage(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
) -> Result<Vec<LibraryEntryUsage>, String> {
    actions::get_library_entry_usage(&state, &entry_id)
}

#[tauri::command]
pub fn scan_folder_for_audio(folder_path: String) -> Result<Vec<String>, String> {
    actions::scan_folder_for_audio(&folder_path)
}

#[tauri::command]
pub fn bulk_import_library_entries(
    state: State<'_, Arc<AppStateContainer>>,
    paths: Vec<String>,
    tags: Vec<String>,
    entry_type: String,
) -> Result<(), String> {
    actions::start_bulk_import_library_entries(state.inner(), paths, tags, entry_type)
}

#[tauri::command]
// Async on purpose: relocating hashes the whole new file and probes its length,
// which is slow on a network share. A plain command would block the UI.
pub async fn relocate_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    new_file_path: String,
) -> Result<AppState, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::relocate_library_entry(&container, &entry_id, new_file_path))
        .await
        .map_err(|e| e.to_string())?
}

/// Async on purpose: the search folder may be a network share, and a plain
/// (non-async) command runs on the main thread, which would freeze the window
/// during the scan. `spawn_blocking` moves the work to a worker thread.
#[tauri::command]
pub async fn find_missing_file_candidates(
    state: State<'_, Arc<AppStateContainer>>,
    folder: String,
) -> Result<crate::models::FixMissingReport, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::find_missing_file_candidates(&container, &folder))
        .await
        .map_err(|e| format!("Scan task failed: {e}"))?
}

#[tauri::command]
pub fn play_library_sound(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    file_path: String,
    position: f64,
    volume: f64,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
    // Optional (default false): seek a paused preview and leave it paused.
    paused: Option<bool>,
) -> Result<(), String> {
    actions::play_library_sound(
        &state, &entry_id, &file_path, position, volume,
        trim_start, trim_end, loop_start, loop_end,
        paused.unwrap_or(false),
    )
}

#[tauri::command]
pub fn stop_library_sound(state: State<'_, Arc<AppStateContainer>>) -> Result<(), String> {
    actions::stop_library_sound(&state)
}

// --- Mute commands ---

#[tauri::command]
pub fn toggle_master_mute(state: State<'_, Arc<AppStateContainer>>) -> Result<bool, String> {
    actions::toggle_master_mute(&state)
}

#[tauri::command]
pub fn toggle_track_mute(state: State<'_, Arc<AppStateContainer>>, track_id: String) -> Result<bool, String> {
    actions::toggle_track_mute(&state, &track_id)
}

#[tauri::command]
pub fn toggle_clip_mute(state: State<'_, Arc<AppStateContainer>>, clip_id: String) -> Result<bool, String> {
    actions::toggle_clip_mute(&state, &clip_id)
}

#[tauri::command]
pub fn toggle_comp_mute(state: State<'_, Arc<AppStateContainer>>, comp_id: String) -> Result<bool, String> {
    actions::toggle_comp_mute(&state, &comp_id)
}

#[tauri::command]
pub fn get_mute_state(state: State<'_, Arc<AppStateContainer>>) -> Result<serde_json::Value, String> {
    actions::get_mute_state(&state)
}

#[tauri::command]
pub fn get_broadcast_status(state: State<'_, Arc<AppStateContainer>>) -> Result<BroadcastStatus, String> {
    actions::get_broadcast_status(&state)
}

#[tauri::command]
pub fn set_broadcast_enabled(state: State<'_, Arc<AppStateContainer>>, enabled: bool) -> Result<BroadcastStatus, String> {
    actions::set_broadcast_enabled(&state, enabled)
}

#[tauri::command]
pub fn set_broadcast_settings(
    state: State<'_, Arc<AppStateContainer>>,
    port: u16,
    public_base: String,
) -> Result<BroadcastStatus, String> {
    actions::set_broadcast_settings(&state, port, public_base)
}

#[tauri::command]
pub fn regenerate_broadcast_key(state: State<'_, Arc<AppStateContainer>>) -> Result<BroadcastStatus, String> {
    actions::regenerate_broadcast_key(&state)
}

// --- Auto-tagging commands ---

#[tauri::command]
pub fn get_autotag_settings(state: State<'_, Arc<AppStateContainer>>) -> Result<AutoTagSettings, String> {
    actions::get_autotag_settings(&state)
}

#[tauri::command]
pub fn save_autotag_settings(
    state: State<'_, Arc<AppStateContainer>>,
    settings: AutoTagSettings,
) -> Result<(), String> {
    actions::save_autotag_settings(&state, settings)
}

#[tauri::command]
pub fn get_scene_config(state: State<'_, Arc<AppStateContainer>>) -> Result<SceneConfig, String> {
    actions::get_scene_config(&state)
}

#[tauri::command]
pub fn save_scene_config(state: State<'_, Arc<AppStateContainer>>, config: SceneConfig) -> Result<(), String> {
    actions::save_scene_config(&state, config)
}

/// Async on purpose: the request can take seconds when the analyzer host is
/// slow or down, and a plain command would freeze the window meanwhile.
#[tauri::command]
pub async fn test_autotag_connection(settings: AutoTagSettings) -> Result<String, String> {
    tokio::task::spawn_blocking(move || actions::test_autotag_connection(&settings))
        .await
        .map_err(|e| format!("Connection test failed: {e}"))?
}

#[tauri::command]
pub fn analyze_library(state: State<'_, Arc<AppStateContainer>>) -> Result<(), String> {
    actions::analyze_library(&state)
}

#[tauri::command]
pub fn get_autotag_report(state: State<'_, Arc<AppStateContainer>>) -> AutoTagReport {
    actions::get_autotag_report(&state)
}

/// The three Scry commands are async for the same reason as
/// `test_autotag_connection`: each waits on the analyzer. The database lock
/// is taken only inside, to read entries.
#[tauri::command]
pub async fn similar_entries(
    state: State<'_, Arc<AppStateContainer>>,
    entry_ids: Vec<String>,
    limit: u32,
) -> Result<Vec<ScryHit>, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::similar_entries(&container, &entry_ids, limit))
        .await
        .map_err(|e| format!("Similar search failed: {e}"))?
}

#[tauri::command]
pub async fn search_scry(
    state: State<'_, Arc<AppStateContainer>>,
    text: String,
    limit: u32,
) -> Result<Vec<ScryHit>, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::search_scry(&container, &text, limit))
        .await
        .map_err(|e| format!("Scry search failed: {e}"))?
}

/// Async like the Scry commands: it waits on the analyzer.
#[tauri::command]
pub async fn preview_energy_prompts(
    state: State<'_, Arc<AppStateContainer>>,
    positive: Vec<String>,
    negative: Vec<String>,
) -> Result<EnergyPreview, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::preview_energy_prompts(&container, &positive, &negative))
        .await
        .map_err(|e| format!("Energy preview failed: {e}"))?
}

#[tauri::command]
pub async fn preview_mood_prompts(
    state: State<'_, Arc<AppStateContainer>>,
    prompts: Vec<String>,
) -> Result<Vec<ScryHit>, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::preview_mood_prompts(&container, &prompts))
        .await
        .map_err(|e| format!("Mood preview failed: {e}"))?
}

#[tauri::command]
pub async fn scry_map(state: State<'_, Arc<AppStateContainer>>) -> Result<Vec<ScryPoint>, String> {
    let container = state.inner().clone();
    tokio::task::spawn_blocking(move || actions::scry_map(&container))
        .await
        .map_err(|e| format!("Scry map failed: {e}"))?
}

#[tauri::command]
pub fn get_entry_tag_scores(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
) -> Result<Option<EntryTagScores>, String> {
    actions::get_entry_tag_scores(&state, &entry_id)
}
