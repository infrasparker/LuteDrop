pub mod ffi;
pub mod mixer;
pub mod orchestrator;
pub mod sound;
pub mod transitions;
pub mod util;

use std::collections::HashMap;

use crate::models::{Track, TrackPlaybackState};
use mixer::MuteState;
use orchestrator::ManagedTrack;
use sound::{PlayParams, PlaybackState, SoundHandle};

const LIBRARY_TRACK_ID: &str = "__library";

/// How far before loop_end (in seconds) to trigger the crossfade restart.
/// Must be larger than the poll interval (50ms) to reliably catch it.
const LOOP_CROSSFADE_SECS: f64 = 0.06;

/// Info needed to restart a library sound at loop_start for crossfade looping.
struct LibraryLoopInfo {
    file_path: String,
    volume: f32,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_region: Option<(f64, f64)>,
    loop_start_position: f64,
}

pub struct AudioEngine {
    // Fields are declared so playing sounds / groups drop before the engine.
    // Rust drops struct fields in declaration order; miniaudio requires the
    // object graph torn down innermost-first (Sounds → Groups → Engine).
    tracks: HashMap<String, ManagedTrack>,
    library_sound: Option<SoundHandle>,
    library_loop_info: Option<LibraryLoopInfo>,
    master_volume: f64,
    mute_state: MuteState,
    /// Sub-mix group for library preview playback.
    library_group: ffi::Group,
    /// Owns the whole audio graph; declared LAST so it outlives all groups/sounds.
    engine: ffi::Engine,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let engine = ffi::Engine::new()?;
        let library_group = engine.create_group()?;
        Ok(Self {
            tracks: HashMap::new(),
            library_sound: None,
            library_loop_info: None,
            master_volume: 1.0,
            mute_state: MuteState::new(),
            library_group,
            engine,
        })
    }

    // --- Track operations ---

    pub fn setup_track(&mut self, config: &Track) -> Result<(), String> {
        orchestrator::setup_track(&self.engine, &mut self.tracks, &self.mute_state, config)
    }

    pub fn update_track_config(&mut self, config: &Track) {
        orchestrator::update_track_config(&mut self.tracks, config);
    }

    pub fn has_track(&self, track_id: &str) -> bool {
        orchestrator::has_track(&self.tracks, track_id)
    }

    pub fn play_track(&mut self, track_id: &str) -> Result<(), String> {
        self.stop_library_sound();
        orchestrator::play_track(&mut self.tracks, &self.engine, track_id, &self.mute_state)
    }

    pub fn play_track_clip(&mut self, track_id: &str, clip_id: &str) -> Result<(), String> {
        self.stop_library_sound();
        orchestrator::play_track_clip(&mut self.tracks, &self.engine, track_id, clip_id, &self.mute_state)
    }

    pub fn play_track_from(&mut self, track_id: &str, position: f64) -> Result<(), String> {
        orchestrator::play_track_from(&mut self.tracks, &self.engine, track_id, position, &self.mute_state)
    }

    pub fn pause_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::pause_track(&mut self.tracks, track_id)
    }

    pub fn resume_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::resume_track(&mut self.tracks, track_id)
    }

    pub fn stop_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::stop_track(&mut self.tracks, track_id)
    }

    pub fn stop_all(&mut self) {
        orchestrator::stop_all(&mut self.tracks);
        self.stop_library_sound();
    }

    pub fn pause_all(&mut self) {
        orchestrator::pause_all(&mut self.tracks);
        if let Some(ref mut snd) = self.library_sound {
            sound::pause(snd);
        }
    }

    pub fn resume_all(&mut self) {
        orchestrator::resume_all(&mut self.tracks);
        if let Some(ref mut snd) = self.library_sound {
            sound::resume(snd);
        }
    }

    // --- Volume ---
    //
    // Native miniaudio hierarchy: master = engine node, track = group node,
    // clip = sound node. miniaudio multiplies them automatically, so there is
    // no master×track×clip recompute / per-sound re-apply loop anymore.

    pub fn get_master_volume(&self) -> f64 {
        self.master_volume
    }

    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        if !self.mute_state.master {
            self.engine.set_master_volume(self.master_volume as f32);
        }
    }

    pub fn set_track_volume(&mut self, track_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_track_volume(&mut self.tracks, track_id, volume, &self.mute_state);
        Ok(())
    }

    pub fn set_clip_volume(&mut self, clip_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_clip_volume(&mut self.tracks, clip_id, volume, &self.mute_state);
        Ok(())
    }

    // --- Mute ---
    //
    // Mute sets the affected node's volume to 0; unmute restores the stored
    // (intended) level. MuteState tracks intent for bookkeeping / snapshots.

    pub fn toggle_master_mute(&mut self) -> bool {
        self.mute_state.master = !self.mute_state.master;
        let vol = if self.mute_state.master { 0.0 } else { self.master_volume as f32 };
        self.engine.set_master_volume(vol);
        self.mute_state.master
    }

    pub fn toggle_track_mute(&mut self, track_id: &str) -> bool {
        let new_state = !self.mute_state.tracks.get(track_id).copied().unwrap_or(false);
        self.mute_state.set_track_muted(track_id, new_state);
        orchestrator::refresh_track_group_volume(&mut self.tracks, track_id, &self.mute_state);
        new_state
    }

    pub fn toggle_clip_mute(&mut self, clip_id: &str) -> bool {
        let new_state = !self.mute_state.clips.get(clip_id).copied().unwrap_or(false);
        self.mute_state.set_clip_muted(clip_id, new_state);
        // Re-apply the owning track's currently-playing sound volume.
        let track_id = self.tracks.iter()
            .find(|(_, t)| t.config.clips.iter().any(|c| c.id == clip_id))
            .map(|(id, _)| id.clone());
        if let Some(tid) = track_id {
            // set_clip_volume with the existing clip volume re-applies the sound node.
            let cur = self.tracks.get(&tid)
                .and_then(|t| t.config.clips.iter().find(|c| c.id == clip_id))
                .map(|c| c.volume)
                .unwrap_or(1.0);
            orchestrator::set_clip_volume(&mut self.tracks, clip_id, cur, &self.mute_state);
        }
        new_state
    }

    pub fn toggle_comp_mute(&mut self, track_ids: &[String]) -> bool {
        let all_muted = track_ids.iter().all(|id| {
            self.mute_state.tracks.get(id).copied().unwrap_or(false)
        });
        let new_state = !all_muted;
        for id in track_ids {
            self.mute_state.set_track_muted(id, new_state);
            orchestrator::refresh_track_group_volume(&mut self.tracks, id, &self.mute_state);
        }
        new_state
    }

    pub fn get_mute_snapshot(&self) -> (bool, std::collections::HashMap<String, bool>, std::collections::HashMap<String, bool>) {
        (self.mute_state.master, self.mute_state.tracks.clone(), self.mute_state.clips.clone())
    }

    // --- Library playback ---

    pub fn play_library_sound(&mut self, params: PlayParams) -> Result<(), String> {
        self.stop_library_sound();
        let restart_info = params.loop_region.map(|(ls, _)| LibraryLoopInfo {
            file_path: params.file_path.clone(),
            volume: params.volume,
            trim_start: params.trim_start,
            trim_end: params.trim_end,
            loop_region: params.loop_region,
            loop_start_position: ls,
        });
        let snd = sound::play(&self.engine, &self.library_group, params)?;
        self.library_sound = Some(snd);
        self.library_loop_info = restart_info;
        Ok(())
    }

    pub fn stop_library_sound(&mut self) {
        if let Some(ref mut snd) = self.library_sound {
            sound::stop(snd, None);
        }
        self.library_sound = None;
        self.library_loop_info = None;
    }

    // --- State queries ---

    /// Returns current playback state of all tracks without driving transitions.
    /// Used by server.rs for snapshot building.
    pub fn get_all_playback_states(&self) -> Vec<TrackPlaybackState> {
        let mut states: Vec<TrackPlaybackState> = self.tracks.values()
            .filter(|t| t.config.id != LIBRARY_TRACK_ID)
            .map(orchestrator::get_playback_state)
            .collect();

        if let Some(ref snd) = self.library_sound {
            let st = sound::state(snd);
            states.push(TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: matches!(st, PlaybackState::Playing),
                is_paused: matches!(st, PlaybackState::Paused),
                current_clip_id: None,
                current_time: sound::position(snd),
            });
        }

        states
    }

    // --- Polling ---

    pub fn poll(&mut self) -> (Vec<TrackPlaybackState>, Vec<String>, Vec<String>) {
        let mut missing_entries: Vec<String> = Vec::new();
        let mut found_entries: Vec<String> = Vec::new();
        let track_ids: Vec<String> = self.tracks.keys()
            .filter(|id| id.as_str() != LIBRARY_TRACK_ID)
            .cloned()
            .collect();

        for id in &track_ids {
            orchestrator::poll_track(&mut self.tracks, &self.engine, id, &self.mute_state, &mut missing_entries, &mut found_entries);
        }

        let mut states: Vec<TrackPlaybackState> = self.tracks.values()
            .filter(|t| t.config.id != LIBRARY_TRACK_ID)
            .map(orchestrator::get_playback_state)
            .collect();

        // Check library sound: handle loop crossfade or stop at trim_end
        let mut library_stopped = false;
        let mut library_needs_loop = false;
        if let Some(ref mut snd) = self.library_sound {
            let st = sound::state(snd);
            let pos = sound::position(snd);
            let is_playing = matches!(st, PlaybackState::Playing);

            if is_playing {
                if let Some((_ls, le)) = snd.loop_region {
                    // Crossfade-loop: fade out current and restart at loop_start
                    if pos >= le - LOOP_CROSSFADE_SECS {
                        sound::stop(snd, Some(std::time::Duration::from_secs_f64(LOOP_CROSSFADE_SECS)));
                        library_needs_loop = true;
                    }
                } else if let Some(end) = snd.duration {
                    // Non-looping: stop at trim_end
                    if pos >= end {
                        sound::stop_quick(snd);
                        library_stopped = true;
                    }
                }
            }
        }

        if library_needs_loop {
            if let Some(ref info) = self.library_loop_info {
                let params = PlayParams {
                    file_path: info.file_path.clone(),
                    start_position: info.loop_start_position,
                    volume: info.volume,
                    trim_start: info.trim_start,
                    trim_end: info.trim_end,
                    loop_region: info.loop_region,
                    fade_in: Some(LOOP_CROSSFADE_SECS),
                    fade_out: None,
                };
                match sound::play(&self.engine, &self.library_group, params) {
                    Ok(new_snd) => { self.library_sound = Some(new_snd); }
                    Err(_) => { self.library_sound = None; self.library_loop_info = None; }
                }
            } else {
                self.library_sound = None;
            }
        } else if library_stopped {
            self.library_sound = None;
        }

        // Always emit library sound state so frontend can detect when it stops
        let lib_state = match &self.library_sound {
            Some(snd) => {
                let st = sound::state(snd);
                TrackPlaybackState {
                    track_id: LIBRARY_TRACK_ID.to_string(),
                    is_playing: matches!(st, PlaybackState::Playing),
                    is_paused: matches!(st, PlaybackState::Paused),
                    current_clip_id: None,
                    current_time: sound::position(snd),
                }
            }
            None => TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: false,
                is_paused: false,
                current_clip_id: None,
                current_time: 0.0,
            },
        };
        states.push(lib_state);

        (states, missing_entries, found_entries)
    }
}
