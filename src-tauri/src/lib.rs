mod actions;
mod audio;
mod commands;
mod database;
mod models;
mod persistence;
mod server;
mod state;
mod waveform;

use std::sync::Arc;
use std::time::Duration;

use tauri::Emitter;
use tauri::Manager;
use state::AppStateContainer;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Determine file paths
            let db_path = persistence::db_file_path(&app.handle());
            let json_path = persistence::json_data_file_path(&app.handle());

            // Peak cache directory lives inside the app data folder
            let peak_cache_dir = db_path.parent().unwrap_or(db_path.as_ref()).join("peaks");

            // Open (or create) the SQLite database
            let conn = database::open_db(&db_path)
                .expect("Failed to open SQLite database");

            // Check and run migration from JSON if needed
            if database::migration::needs_migration(&conn, &json_path) {
                log::info!("Migrating data from JSON to SQLite...");
                match database::migration::migrate_from_json(&conn, &json_path) {
                    Ok((groups, entries, missing)) => {
                        log::info!(
                            "Migration complete: {} groups, {} library entries, {} missing files",
                            groups, entries, missing
                        );
                        if let Err(e) = database::migration::backup_json(&json_path) {
                            log::warn!("Failed to backup JSON file: {}", e);
                        }
                    }
                    Err(e) => {
                        log::error!("Migration failed: {}", e);
                    }
                }
            }

            // Backfill durations for any entries missing them
            match database::library::backfill_durations(&conn) {
                Ok(n) if n > 0 => log::info!("Backfilled duration for {} library entries", n),
                Err(e) => log::warn!("Failed to backfill durations: {}", e),
                _ => {}
            }

            // Create the audio engine
            let audio_engine = audio::AudioEngine::new()
                .expect("Failed to initialize audio engine");

            let container = Arc::new(AppStateContainer::new(conn, audio_engine, peak_cache_dir));

            // Share the Arc with Tauri's managed state
            app.manage(container.clone());

            // Preload peak caches in background
            let preload_container = container.clone();
            std::thread::spawn(move || {
                match actions::preload_peaks(&preload_container) {
                    Ok(n) => log::info!("Preloaded peaks for {} library entries", n),
                    Err(e) => log::warn!("Peak preload failed: {}", e),
                }
            });

            // Start the axum WebSocket server on its own thread with a tokio runtime
            let server_container = container.clone();
            let remote_dist = std::env::current_dir()
                .unwrap_or_default()
                .join("dist-remote")
                .to_string_lossy()
                .to_string();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create tokio runtime for server");
                rt.block_on(server::start_server(server_container, remote_dist));
            });

            // Spawn the playback state polling task
            let poll_container = container.clone();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                playback_polling_loop(app_handle, poll_container);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::load_data,
            commands::add_group,
            commands::delete_group,
            commands::rename_group,
            commands::add_comp,
            commands::rename_comp,
            commands::delete_comp,
            commands::add_track,
            commands::delete_track,
            commands::update_track,
            commands::add_clip,
            commands::delete_clip,
            commands::play_track,
            commands::play_track_from,
            commands::play_track_clip,
            commands::pause_track,
            commands::stop_track,
            commands::set_master_volume,
            commands::set_track_volume,
            commands::set_clip_volume,
            commands::update_clip_volume,
            commands::activate_comp,
            commands::stop_comp,
            commands::stop_all,
            commands::pause_all,
            commands::resume_all,
            commands::get_waveform_data,
            commands::get_server_info,
            commands::import_library_entry,
            commands::update_library_entry_tags,
            commands::update_library_entry_markers,
            commands::update_library_entry_trim,
            commands::update_library_entry_loop_points,
            commands::update_library_entry_volume,
            commands::rename_library_entry,
            commands::delete_library_entry,
            commands::relocate_library_entry,
            commands::check_missing_files,
            commands::play_preview,
            commands::stop_preview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}

/// Polling loop that checks for finished sounds and emits playback state updates.
/// Runs on its own thread every 50ms.
fn playback_polling_loop(app_handle: tauri::AppHandle, container: Arc<AppStateContainer>) {
    loop {
        std::thread::sleep(Duration::from_millis(50));

        // Lock the audio engine
        let mut audio = match container.audio.lock() {
            Ok(a) => a,
            Err(_) => continue,
        };

        // Check for crossfade triggers (approaching end of clip)
        let crossfades = audio.check_crossfade_triggers();
        for (track_id, next_idx, duration) in crossfades {
            if let Err(e) = audio.begin_crossfade(&track_id, next_idx, duration) {
                log::warn!("Failed to start crossfade for track {}: {}", track_id, e);
            }
        }

        // Check for sounds that finished naturally and advance/stop tracks
        let finished = audio.check_finished_sounds();
        for (track_id, next_clip_index) in finished {
            if let Err(e) = audio.advance_track(&track_id, next_clip_index) {
                log::warn!("Failed to advance track {}: {}", track_id, e);
            }
        }

        // Get all playback states and master volume, then emit to the frontend
        let states = audio.get_all_playback_states();
        let master_vol = audio.get_master_volume();
        drop(audio); // Release the lock before emitting

        // Emit to desktop frontend
        for state in &states {
            let _ = app_handle.emit("playback-state-update", state);
        }
        let _ = app_handle.emit("master-volume-update", master_vol);

        // Broadcast to remote clients
        server::broadcast_state(&container);
    }
}
