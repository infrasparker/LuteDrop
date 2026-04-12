use std::fs;
use std::path::PathBuf;
use tauri::Manager;
use crate::models::AppState;

/// Returns the path to the JSON data file in the OS app data directory.
pub fn data_file_path(app: &tauri::AppHandle) -> PathBuf {
    let app_data = app
        .path()
        .app_data_dir()
        .expect("failed to get app data dir");
    fs::create_dir_all(&app_data).ok();
    app_data.join("lutedrop-data.json")
}

pub fn load_from_disk(path: &PathBuf) -> AppState {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => AppState::default(),
    }
}

pub fn save_to_disk(path: &PathBuf, state: &AppState) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}
