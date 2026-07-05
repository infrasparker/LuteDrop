pub mod ffi;
pub mod mixer;
pub mod orchestrator;
pub mod sound;
pub mod transitions;
pub mod util;
pub mod voice;

use std::collections::HashMap;

use crate::models::{Track, TrackPlaybackState};
use mixer::MuteState;
use orchestrator::ManagedTrack;
use sound::{PlayParams, PlaybackState};
use voice::Voice;

const LIBRARY_TRACK_ID: &str = "__library";

pub struct AudioEngine {
    // Fields are declared so playing sounds / groups drop before the engine.
    // Rust drops struct fields in declaration order; miniaudio requires the
    // object graph torn down innermost-first (Sounds → Groups → Engine).
    tracks: HashMap<String, ManagedTrack>,
    /// Single-sound playback for the library preview: owns its sub-mix group, the
    /// current sound, the de-click fade-out reaper, and the seamless-loop
    /// crossfade. Declared before `engine` so its group/sounds drop first.
    library: Voice,
    master_volume: f64,
    mute_state: MuteState,
    /// Owns the whole audio graph; declared LAST so it outlives all groups/sounds.
    engine: ffi::Engine,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let engine = ffi::Engine::new()?;
        // Library preview does NOT sample-accurately anchor its loop restart
        // (matches the pre-refactor library behavior).
        let library = Voice::new(&engine)?;
        Ok(Self {
            tracks: HashMap::new(),
            library,
            master_volume: 1.0,
            mute_state: MuteState::new(),
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
        self.library.pause();
    }

    pub fn resume_all(&mut self) {
        orchestrator::resume_all(&mut self.tracks);
        self.library.resume();
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

    pub fn set_library_volume(&mut self, volume: f64) {
        self.library.set_volume(volume.clamp(0.0, 1.0) as f32);
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
        // A SEEK is `stop_library_sound` + `play_library_sound`; the de-click stop
        // (below) keeps the outgoing sound mixing through its fade so the
        // replacement doesn't click.
        self.stop_library_sound();
        // Scheduled de-clicked stop at trim_end — replaces the old late
        // poll-check hard cut. The library preview never crossfades and
        // `poll_loop(stop_at_end=true)` still detects the resulting Stopped
        // state to finish. For a looping preview `play_scheduled` no-ops the
        // schedule (loop_region is Some).
        self.library.play_scheduled(&self.engine, params)
    }

    pub fn stop_library_sound(&mut self) {
        // Use the sound's own fade_out if it has one, else a small de-click
        // default. The Voice parks the outgoing sound until the fade finishes.
        self.library.stop(None);
    }

    // --- State queries ---

    /// Returns current playback state of all tracks without driving transitions.
    /// Used by server.rs for snapshot building.
    pub fn get_all_playback_states(&self) -> Vec<TrackPlaybackState> {
        let mut states: Vec<TrackPlaybackState> = self.tracks.values()
            .filter(|t| t.config.id != LIBRARY_TRACK_ID)
            .map(orchestrator::get_playback_state)
            .collect();

        if self.library.has_current() {
            let st = self.library.state();
            states.push(TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: matches!(st, PlaybackState::Playing),
                is_paused: matches!(st, PlaybackState::Paused),
                current_clip_id: None,
                current_time: self.library.position(),
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

        // Drop finished held sounds, then drive the library's seamless-loop
        // crossfade / stop-at-trim-end. The Voice holds the outgoing loop
        // instance so it keeps mixing through its fade (no mid-fade drop / pop).
        self.library.drop_finished();
        self.library.poll_loop(&self.engine, true, true);

        // Always emit library sound state so frontend can detect when it stops.
        let lib_state = if self.library.has_current() {
            let st = self.library.state();
            TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: matches!(st, PlaybackState::Playing),
                is_paused: matches!(st, PlaybackState::Paused),
                current_clip_id: None,
                current_time: self.library.position(),
            }
        } else {
            TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: false,
                is_paused: false,
                current_clip_id: None,
                current_time: 0.0,
            }
        };
        states.push(lib_state);

        (states, missing_entries, found_entries)
    }
}
