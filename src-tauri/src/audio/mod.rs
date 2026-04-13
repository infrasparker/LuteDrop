pub mod mixer;
pub mod orchestrator;
pub mod sound;
pub mod transitions;
pub mod util;

use std::collections::HashMap;
use kira::track::TrackBuilder;
use kira::{AudioManager, AudioManagerSettings};

use crate::models::{Track, TrackPlaybackState};
use mixer::MuteState;
use orchestrator::ManagedTrack;
use sound::{PlayParams, SoundHandle};
use util::amplitude_to_decibels;

const LIBRARY_TRACK_ID: &str = "__library";

pub struct AudioEngine {
    manager: AudioManager,
    tracks: HashMap<String, ManagedTrack>,
    master_volume: f64,
    mute_state: MuteState,
    library_sound: Option<SoundHandle>,
    library_track_handle: kira::track::TrackHandle,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let mut manager = AudioManager::new(AudioManagerSettings::default())
            .map_err(|e| format!("Failed to create audio manager: {}", e))?;
        let library_track_handle = manager.add_sub_track(TrackBuilder::default())
            .map_err(|e| format!("Failed to create library track: {}", e))?;
        Ok(Self {
            manager,
            tracks: HashMap::new(),
            master_volume: 1.0,
            mute_state: MuteState::new(),
            library_sound: None,
            library_track_handle,
        })
    }

    // --- Track operations ---

    pub fn setup_track(&mut self, config: &Track) -> Result<(), String> {
        orchestrator::setup_track(&mut self.manager, &mut self.tracks, config)
    }

    pub fn update_track_config(&mut self, config: &Track) {
        orchestrator::update_track_config(&mut self.tracks, config);
    }

    pub fn has_track(&self, track_id: &str) -> bool {
        orchestrator::has_track(&self.tracks, track_id)
    }

    pub fn play_track(&mut self, track_id: &str) -> Result<(), String> {
        self.stop_library_sound();
        orchestrator::play_track(&mut self.tracks, &mut self.manager, track_id, self.master_volume, &self.mute_state)
    }

    pub fn play_track_clip(&mut self, track_id: &str, clip_id: &str) -> Result<(), String> {
        self.stop_library_sound();
        orchestrator::play_track_clip(&mut self.tracks, &mut self.manager, track_id, clip_id, self.master_volume, &self.mute_state)
    }

    pub fn play_track_from(&mut self, track_id: &str, position: f64) -> Result<(), String> {
        orchestrator::play_track_from(&mut self.tracks, &mut self.manager, track_id, position, self.master_volume, &self.mute_state)
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
    }

    pub fn resume_all(&mut self) {
        orchestrator::resume_all(&mut self.tracks);
    }

    // --- Volume ---

    pub fn get_master_volume(&self) -> f64 {
        self.master_volume
    }

    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        let ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in ids {
            let vol = self.tracks.get(&id).map(|t| t.config.volume).unwrap_or(1.0);
            orchestrator::set_track_volume(&mut self.tracks, &id, vol, self.master_volume, &self.mute_state);
        }
        // Update library sound volume too
        if let Some(ref mut snd) = self.library_sound {
            sound::set_volume(snd, amplitude_to_decibels(self.master_volume));
        }
    }

    pub fn set_track_volume(&mut self, track_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_track_volume(&mut self.tracks, track_id, volume, self.master_volume, &self.mute_state);
        Ok(())
    }

    pub fn set_clip_volume(&mut self, clip_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_clip_volume(&mut self.tracks, clip_id, volume, self.master_volume, &self.mute_state);
        Ok(())
    }

    // --- Mute ---

    #[allow(dead_code)]
    pub fn mute_master(&mut self) { self.mute_state.master = true; self.refresh_all_volumes(); }
    #[allow(dead_code)]
    pub fn unmute_master(&mut self) { self.mute_state.master = false; self.refresh_all_volumes(); }
    #[allow(dead_code)]
    pub fn mute_track(&mut self, track_id: &str) { self.mute_state.set_track_muted(track_id, true); self.refresh_track_volume(track_id); }
    #[allow(dead_code)]
    pub fn unmute_track(&mut self, track_id: &str) { self.mute_state.set_track_muted(track_id, false); self.refresh_track_volume(track_id); }
    #[allow(dead_code)]
    pub fn mute_clip(&mut self, clip_id: &str) { self.mute_state.set_clip_muted(clip_id, true); }
    #[allow(dead_code)]
    pub fn unmute_clip(&mut self, clip_id: &str) { self.mute_state.set_clip_muted(clip_id, false); }

    // --- Library playback ---

    pub fn play_library_sound(&mut self, params: PlayParams) -> Result<(), String> {
        self.stop_library_sound();
        let snd = sound::play(&mut self.manager, &mut self.library_track_handle, params)?;
        self.library_sound = Some(snd);
        Ok(())
    }

    pub fn stop_library_sound(&mut self) {
        if let Some(ref mut snd) = self.library_sound {
            sound::stop(snd, None);
        }
        self.library_sound = None;
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
                is_playing: matches!(st, kira::sound::PlaybackState::Playing | kira::sound::PlaybackState::Resuming),
                is_paused: matches!(st, kira::sound::PlaybackState::Paused | kira::sound::PlaybackState::Pausing),
                current_clip_id: None,
                current_time: sound::position(snd),
            });
        }

        states
    }

    // --- Polling ---

    pub fn poll(&mut self) -> Vec<TrackPlaybackState> {
        let track_ids: Vec<String> = self.tracks.keys()
            .filter(|id| id.as_str() != LIBRARY_TRACK_ID)
            .cloned()
            .collect();

        for id in &track_ids {
            orchestrator::poll_track(&mut self.tracks, &mut self.manager, id, self.master_volume, &self.mute_state);
        }

        let mut states: Vec<TrackPlaybackState> = self.tracks.values()
            .filter(|t| t.config.id != LIBRARY_TRACK_ID)
            .map(orchestrator::get_playback_state)
            .collect();

        // Check library sound: stop at trim_end if reached
        let mut library_stopped = false;
        if let Some(ref mut snd) = self.library_sound {
            let st = sound::state(snd);
            let pos = sound::position(snd);
            let is_playing = matches!(st, kira::sound::PlaybackState::Playing | kira::sound::PlaybackState::Resuming);

            // Stop if we've reached trim_end (duration stores trim_end when set)
            // But only for non-looping sounds (loop_region handles its own end)
            if is_playing {
                if let Some(end) = snd.duration {
                    if pos >= end {
                        sound::stop_quick(snd);
                        library_stopped = true;
                    }
                }
            }
        }
        if library_stopped {
            self.library_sound = None;
        }

        // Always emit library sound state so frontend can detect when it stops
        let lib_state = match &self.library_sound {
            Some(snd) => {
                let st = sound::state(snd);
                TrackPlaybackState {
                    track_id: LIBRARY_TRACK_ID.to_string(),
                    is_playing: matches!(st, kira::sound::PlaybackState::Playing | kira::sound::PlaybackState::Resuming),
                    is_paused: matches!(st, kira::sound::PlaybackState::Paused | kira::sound::PlaybackState::Pausing),
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

        states
    }

    // --- Internal ---

    fn refresh_all_volumes(&mut self) {
        let ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in ids {
            self.refresh_track_volume(&id);
        }
    }

    fn refresh_track_volume(&mut self, track_id: &str) {
        if let Some(track) = self.tracks.get(track_id) {
            let vol = track.config.volume;
            orchestrator::set_track_volume(&mut self.tracks, track_id, vol, self.master_volume, &self.mute_state);
        }
    }
}
