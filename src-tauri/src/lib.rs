//! LuteDrop backend: Tauri app for Lua-based audio composition.
//!
//! ## Architecture: Three Layers
//! - **Commands**: Tauri-invoked handlers translating frontend requests.
//! - **Actions**: Business logic orchestrating state mutations and notifications.
//! - **Persistence & Audio**: SQLite database and audio engine.
//!
//! ## Background Threads Spawned in Setup (5)
//! 1. **Peak preload**: Precomputes waveform peaks for the library on startup.
//! 2. **Backup loop**: Automatic database snapshots every 30 minutes.
//! 3. **WebSocket server**: Axum HTTP + WS for remote control (with tokio runtime).
//! 4. **50ms poll loop**: Drives audio engine, emits playback updates.
//! 5. **Broadcast restore**: Short-lived; turns the live stream back on if it
//!    was on at last exit (the stream's own encoder/server threads then run
//!    until it is turned off).
//!
//! ## Tauri Events Emitted to Frontend
//! - `playback-state-update`: Per-track playback position and state (every 50ms; listened: useAudio.ts).
//! - `master-volume-update`: Master gain changes (every 50ms; listened: useAudio.ts).
//! - `missing-entries-changed`: Audio file moved or deleted (listened: useAudio.ts).
//! - `missing-entry-found`: Audio file relocated or found (listened: useAudio.ts).
//! - `library-import-progress`: Bulk import progress (listened: useLibraryImport.ts).
//! - `library-import-complete`: Bulk import finished (listened: useLibraryImport.ts).
//! - `broadcast-listeners`: Live-stream listener count, `null` when off; sent only on change (listened: useBroadcast.ts).
//!
//! ## WebSocket Message Types to Remote Clients
//! - `"state"`: Full structural snapshot (app state, library, all playback).
//! - `"playback"`: Lightweight tick (positions + master volume).

mod actions;
mod audio;
mod broadcast;
mod commands;
mod database;
mod models;
mod server;
mod state;
mod waveform;

use std::sync::Arc;

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
            let db_path = {
                let app_data = app.path().app_data_dir().expect("failed to get app data dir");
                std::fs::create_dir_all(&app_data).ok();
                app_data.join("lutedrop.db")
            };
            // Peak cache directory lives inside the app data folder
            let app_data_dir = db_path.parent().unwrap_or(db_path.as_ref()).to_path_buf();
            let peak_cache_dir = app_data_dir.join("peaks");

            // Open (or create) the SQLite database
            let conn = database::open_db(&db_path)
                .expect("Failed to open SQLite database");

            // Ensure a default configuration exists (for fresh databases)
            if let Err(e) = database::ensure_default_configuration(&conn) {
                log::error!("Failed to ensure default configuration: {}", e);
            }

            // Create the audio engine
            let audio_engine = audio::AudioEngine::new()
                .expect("Failed to initialize audio engine");

            let container = Arc::new(AppStateContainer::new(conn, audio_engine, peak_cache_dir, db_path.clone()));

            // Scan for missing files before sharing state with frontend
            actions::scan_missing_files(&container);

            // Reclaim disk from peak cache files no longer referenced by the
            // library (deleted/relocated entries).
            actions::gc_caches(&container);

            // Give the container the app handle so it can emit events
            container.set_app_handle(app.handle().clone());

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

            // Start automatic database backups (on startup + every 30 min)
            let backup_container = container.clone();
            std::thread::spawn(move || {
                database::backup::start_backup_loop(backup_container);
            });

            // Start the axum WebSocket server on its own thread with a tokio runtime
            let server_container = container.clone();
            let remote_dist = app.path().resource_dir()
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default())
                .join("dist-remote")
                .to_string_lossy()
                .to_string();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create tokio runtime for server");
                rt.block_on(server::start_server(server_container, remote_dist));
            });

            // Turn the live broadcast back on if it was on at last exit. On its
            // own thread so a slow start never delays the window opening.
            let broadcast_container = container.clone();
            std::thread::spawn(move || {
                actions::restore_broadcast_on_startup(&broadcast_container);
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
            commands::add_clips,
            commands::delete_clip,
            commands::play_track,
            commands::play_track_from,
            commands::play_track_clip,
            commands::pause_track,
            commands::stop_track,
            commands::set_master_volume,
            commands::set_track_volume,
            commands::set_clip_volume,
            commands::set_library_volume,
            commands::activate_comp,
            commands::stop_comp,
            commands::pause_comp,
            commands::resume_comp,
            commands::stop_all,
            commands::pause_all,
            commands::resume_all,
            commands::get_waveform_data,
            commands::get_server_info,
            commands::import_library_entry,
            commands::bulk_import_library_entries,
            commands::scan_folder_for_audio,
            commands::update_library_entry_tags,
            commands::update_library_entry_type,
            commands::update_library_entry_markers,
            commands::update_library_entry_volume,
            commands::rename_library_entry,
            commands::delete_library_entry,
            commands::get_library_entry_usage,
            commands::relocate_library_entry,
            commands::play_library_sound,
            commands::stop_library_sound,
            commands::toggle_master_mute,
            commands::toggle_track_mute,
            commands::toggle_clip_mute,
            commands::toggle_comp_mute,
            commands::get_mute_state,
            commands::create_configuration,
            commands::duplicate_configuration,
            commands::load_configuration,
            commands::delete_configuration,
            commands::rename_configuration,
            commands::get_broadcast_status,
            commands::set_broadcast_enabled,
            commands::set_broadcast_settings,
            commands::regenerate_broadcast_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}

/// Polling loop that drives the audio engine and emits playback state updates.
/// Runs on its own thread every 50ms.
fn playback_polling_loop(app_handle: tauri::AppHandle, container: Arc<AppStateContainer>) {
    // Last listener count sent as `broadcast-listeners`, so the event goes out
    // only on change. Starts as "off", matching the stream at launch.
    let mut last_listeners: Option<u32> = None;
    loop {
        std::thread::sleep(audio::POLL_INTERVAL);

        let (states, master_vol, newly_missing, newly_found, listeners) = {
            let mut audio = container.lock_audio();
            let (states, missing, found) = audio.poll();
            let vol = audio.get_master_volume();
            let listeners = audio.broadcast_listeners().map(|n| n as u32);
            (states, vol, missing, found, listeners)
        };

        for state in &states {
            let _ = app_handle.emit("playback-state-update", state);
        }
        let _ = app_handle.emit("master-volume-update", master_vol);
        if listeners != last_listeners {
            let _ = app_handle.emit("broadcast-listeners", listeners);
            last_listeners = listeners;
        }

        // Update missing state — events are emitted automatically by mark_entry_missing/found
        for entry_id in &newly_missing {
            container.mark_entry_missing(entry_id);
        }
        for entry_id in &newly_found {
            container.mark_entry_found(entry_id);
        }

        server::broadcast_tick(&container);
    }
}
