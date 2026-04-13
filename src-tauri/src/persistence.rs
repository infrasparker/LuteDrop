use std::path::PathBuf;
use tauri::Manager;

pub fn json_data_file_path(app: &tauri::AppHandle) -> PathBuf {
    let app_data = app.path().app_data_dir().expect("failed to get app data dir");
    std::fs::create_dir_all(&app_data).ok();
    app_data.join("lutedrop-data.json")
}

pub fn db_file_path(app: &tauri::AppHandle) -> PathBuf {
    let app_data = app.path().app_data_dir().expect("failed to get app data dir");
    std::fs::create_dir_all(&app_data).ok();
    app_data.join("lutedrop.db")
}
