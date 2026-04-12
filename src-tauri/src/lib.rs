mod actions;
mod audio;
mod commands;
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
            // Load saved data from disk on startup
            let path = persistence::data_file_path(&app.handle());
            let initial_state = persistence::load_from_disk(&path);

            // Peak cache directory lives inside the app data folder
            let peak_cache_dir = path.parent().unwrap_or(path.as_ref()).join("peaks");

            // Create the audio engine
            let audio_engine = audio::AudioEngine::new()
                .expect("Failed to initialize audio engine");

            let container = Arc::new(AppStateContainer::new(initial_state, path, audio_engine, peak_cache_dir));

            // Share the Arc with Tauri's managed state
            app.manage(container.clone());

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
            commands::activate_comp,
            commands::stop_comp,
            commands::stop_all,
            commands::pause_all,
            commands::resume_all,
            commands::get_waveform_data,
            commands::get_server_info,
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
