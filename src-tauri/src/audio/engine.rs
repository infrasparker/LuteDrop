use std::collections::HashMap;
use std::time::{Duration, Instant};

use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{EndPosition, FromFileError, PlaybackPosition, PlaybackState, Region};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, Easing, Tween};
use rand::Rng;

use crate::models::{Track, TrackMode, TrackPlaybackState, TrackTransition};

/// Converts a linear amplitude (0.0 - 1.0) to kira Decibels.
/// Uses the standard formula: dB = 20 * log10(amplitude).
/// Clamps very small values to silence.
fn amplitude_to_decibels(amp: f64) -> Decibels {
    if amp <= 0.0001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * (amp as f32).log10())
    }
}

/// State for a track that has been set up in the engine.
struct ActiveTrack {
    /// The kira mixer sub-track handle. Sounds play on this track.
    track_handle: TrackHandle,
    /// Handle to the currently playing sound, if any.
    sound_handle: Option<StreamingSoundHandle<FromFileError>>,
    /// The track configuration snapshot (clips, mode, volume, fades).
    config: Track,
    /// Index of the currently playing clip within config.clips.
    current_clip_index: Option<usize>,
    /// Whether the track is logically paused (as opposed to stopped).
    is_paused: bool,
    /// For gap transitions: when to start the next clip.
    pending_start: Option<PendingStart>,
}

/// A deferred clip start, used for gap transitions.
struct PendingStart {
    clip_index: usize,
    start_at: Instant,
}

pub struct AudioEngine {
    manager: AudioManager,
    tracks: HashMap<String, ActiveTrack>,
    master_volume: f64,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let manager = AudioManager::new(AudioManagerSettings::default())
            .map_err(|e| format!("Failed to create audio manager: {}", e))?;
        Ok(Self {
            manager,
            tracks: HashMap::new(),
            master_volume: 1.0,
        })
    }

    /// Creates a mixer sub-track for the given track config and stores it.
    pub fn setup_track(&mut self, track: &Track) -> Result<(), String> {
        // If already set up, just update the config
        if self.tracks.contains_key(&track.id) {
            self.update_track_config(track);
            return Ok(());
        }

        let track_handle = self
            .manager
            .add_sub_track(TrackBuilder::new())
            .map_err(|e| format!("Failed to create sub-track: {}", e))?;

        self.tracks.insert(
            track.id.clone(),
            ActiveTrack {
                track_handle,
                sound_handle: None,
                config: track.clone(),
                current_clip_index: None,
                is_paused: false,
                pending_start: None,
            },
        );

        Ok(())
    }

    /// Removes a track, stopping any active sound.
    #[allow(dead_code)]
    pub fn remove_track(&mut self, track_id: &str) {
        if let Some(mut active) = self.tracks.remove(track_id) {
            if let Some(ref mut handle) = active.sound_handle {
                let _ = handle.stop(Tween {
                    duration: Duration::from_millis(50),
                    ..Default::default()
                });
            }
            // TrackHandle is dropped here, which removes the mixer track
        }
    }

    /// Updates the stored config snapshot for a track.
    pub fn update_track_config(&mut self, track: &Track) {
        if let Some(active) = self.tracks.get_mut(&track.id) {
            active.config = track.clone();
        }
    }

    pub fn has_track(&self, track_id: &str) -> bool {
        self.tracks.contains_key(track_id)
    }

    /// Play a track. If paused, resume. Otherwise start from the first clip.
    pub fn play_track(&mut self, track_id: &str) -> Result<(), String> {
        // Check if paused first
        let is_paused = self
            .tracks
            .get(track_id)
            .map(|t| t.is_paused)
            .unwrap_or(false);

        if is_paused {
            return self.resume_track(track_id);
        }

        // Pick the first clip and start it
        let clip_index = self.pick_first_clip(track_id)?;
        match clip_index {
            Some(idx) => self.start_clip(track_id, idx),
            None => Err(format!("Track {} has no clips", track_id)),
        }
    }

    fn resume_track(&mut self, track_id: &str) -> Result<(), String> {
        let active = self
            .tracks
            .get_mut(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;

        active.is_paused = false;

        if let Some(ref mut handle) = active.sound_handle {
            handle.resume(Tween {
                duration: Duration::from_millis(50),
                ..Default::default()
            });
        }

        Ok(())
    }

    /// Pause the current sound on a track.
    pub fn pause_track(&mut self, track_id: &str) -> Result<(), String> {
        let active = self
            .tracks
            .get_mut(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;

        active.is_paused = true;
        active.pending_start = None;

        if let Some(ref mut handle) = active.sound_handle {
            handle.pause(Tween {
                duration: Duration::from_millis(50),
                ..Default::default()
            });
        }

        Ok(())
    }

    /// Stop a track with fade out if configured.
    pub fn stop_track(&mut self, track_id: &str) -> Result<(), String> {
        let active = self
            .tracks
            .get_mut(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;

        active.is_paused = false;
        active.current_clip_index = None;
        active.pending_start = None;

        if let Some(ref mut handle) = active.sound_handle {
            let fade_out = active.config.fade_out;
            let tween = if fade_out > 0.0 {
                Tween {
                    duration: Duration::from_secs_f64(fade_out),
                    easing: Easing::OutPowi(2),
                    ..Default::default()
                }
            } else {
                Tween {
                    duration: Duration::from_millis(10),
                    ..Default::default()
                }
            };
            let _ = handle.stop(tween);
        }

        active.sound_handle = None;

        Ok(())
    }

    /// Stop all tracks.
    pub fn stop_all(&mut self) {
        let track_ids: Vec<String> = self.tracks.keys().cloned().collect();
        for track_id in track_ids {
            let _ = self.stop_track(&track_id);
        }
    }

    /// Returns the current master volume.
    pub fn get_master_volume(&self) -> f64 {
        self.master_volume
    }

    /// Set the master volume (0.0 - 1.0). Updates all active sounds.
    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        // Update all currently playing sounds
        let track_ids: Vec<String> = self.tracks.keys().cloned().collect();
        for track_id in track_ids {
            self.update_sound_volume(&track_id);
        }
    }

    /// Set the volume for a specific track. Updates active sound if playing.
    pub fn set_track_volume(&mut self, track_id: &str, volume: f64) -> Result<(), String> {
        {
            let active = self
                .tracks
                .get_mut(track_id)
                .ok_or_else(|| format!("Track not found: {}", track_id))?;
            active.config.volume = volume.clamp(0.0, 1.0);
        }

        self.update_sound_volume(track_id);
        Ok(())
    }

    /// Set the volume for a specific clip. Finds which track has this clip and updates.
    pub fn set_clip_volume(&mut self, clip_id: &str, volume: f64) -> Result<(), String> {
        let volume = volume.clamp(0.0, 1.0);

        // Find which track has this clip
        let mut found_track_id: Option<String> = None;
        for (track_id, active) in &mut self.tracks {
            for clip in &mut active.config.clips {
                if clip.id == clip_id {
                    clip.volume = volume;
                    found_track_id = Some(track_id.clone());
                    break;
                }
            }
            if found_track_id.is_some() {
                break;
            }
        }

        match found_track_id {
            Some(track_id) => {
                self.update_sound_volume(&track_id);
                Ok(())
            }
            None => Err(format!("Clip not found: {}", clip_id)),
        }
    }

    /// Returns the current playback state of all set-up tracks.
    pub fn get_all_playback_states(&self) -> Vec<TrackPlaybackState> {
        self.tracks
            .values()
            .map(|active| {
                let (is_playing, is_paused, current_time) = match &active.sound_handle {
                    Some(handle) => {
                        let state = handle.state();
                        let playing = matches!(
                            state,
                            PlaybackState::Playing | PlaybackState::Resuming
                        );
                        let paused = matches!(
                            state,
                            PlaybackState::Paused
                                | PlaybackState::Pausing
                                | PlaybackState::WaitingToResume
                        );
                        let position = handle.position();
                        (playing, paused, position)
                    }
                    None => {
                        // If there's a pending start, the track is "playing" (waiting for gap)
                        let has_pending = active.pending_start.is_some();
                        (has_pending, active.is_paused, 0.0)
                    }
                };

                let current_clip_id = active
                    .current_clip_index
                    .and_then(|idx| active.config.clips.get(idx))
                    .map(|clip| clip.id.clone());

                TrackPlaybackState {
                    track_id: active.config.id.clone(),
                    is_playing,
                    is_paused,
                    current_clip_id,
                    current_time,
                }
            })
            .collect()
    }

    /// Check for sounds that finished naturally. Returns (track_id, next_clip_index).
    /// For seamless mode, sounds loop natively so they never "finish" here.
    pub fn check_finished_sounds(&mut self) -> Vec<(String, Option<usize>)> {
        let mut finished = Vec::new();

        for (track_id, active) in &self.tracks {
            // Skip paused tracks
            if active.is_paused {
                continue;
            }

            // Check for pending gap starts
            if let Some(ref pending) = active.pending_start {
                if Instant::now() >= pending.start_at {
                    finished.push((track_id.clone(), Some(pending.clip_index)));
                }
                continue;
            }

            // Check if the sound has stopped naturally
            if let Some(ref handle) = active.sound_handle {
                let state = handle.state();
                if matches!(state, PlaybackState::Stopped) {
                    // Sound finished. Determine next clip.
                    let next = pick_next_clip_index(
                        &active.config,
                        active.current_clip_index,
                    );
                    finished.push((track_id.clone(), next));
                }
            }
        }

        finished
    }

    /// Advance a track to the next clip, or stop it if there's no next clip.
    pub fn advance_track(
        &mut self,
        track_id: &str,
        next_clip_index: Option<usize>,
    ) -> Result<(), String> {
        match next_clip_index {
            Some(idx) => {
                // Check if this is a pending gap start
                let is_pending = self
                    .tracks
                    .get(track_id)
                    .map(|a| a.pending_start.is_some())
                    .unwrap_or(false);

                if is_pending {
                    // Clear the pending start and actually start the clip
                    if let Some(active) = self.tracks.get_mut(track_id) {
                        active.pending_start = None;
                    }
                    self.start_clip(track_id, idx)
                } else {
                    // Determine transition type
                    self.start_next_with_transition(track_id, idx)
                }
            }
            None => {
                // No more clips, stop the track
                self.stop_track(track_id)
            }
        }
    }

    // --- Internal methods ---

    /// Start a specific clip on a track, applying volume, loop region, trim, and fade in.
    fn start_clip(&mut self, track_id: &str, clip_index: usize) -> Result<(), String> {
        let active = self
            .tracks
            .get_mut(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;

        let clip = active
            .config
            .clips
            .get(clip_index)
            .ok_or_else(|| format!("Clip index {} out of range", clip_index))?
            .clone();

        // Stop any currently playing sound
        if let Some(ref mut handle) = active.sound_handle {
            let _ = handle.stop(Tween {
                duration: Duration::from_millis(10),
                ..Default::default()
            });
        }

        // Load the audio file (streaming: decodes in chunks from disk, no upfront delay)
        let mut sound_data = StreamingSoundData::from_file(&clip.file_path)
            .map_err(|e| format!("Failed to load audio file '{}': {}", clip.file_path, e))?;

        // Calculate combined volume: master * track * clip
        let combined_volume = self.master_volume * active.config.volume * clip.volume;

        // Apply loop region for seamless mode
        match &active.config.mode {
            TrackMode::Seamless {
                loop_start,
                loop_end,
            } => {
                let ls = *loop_start;
                let le = *loop_end;
                let region_start = PlaybackPosition::Seconds(ls);
                let region_end = if le > 0.0 {
                    EndPosition::Custom(PlaybackPosition::Seconds(le))
                } else {
                    EndPosition::EndOfAudio
                };
                sound_data = sound_data.loop_region(Region {
                    start: region_start,
                    end: region_end,
                });
            }
            _ => {}
        }

        // Apply fade in
        let fade_in = active.config.fade_in;
        if fade_in > 0.0 {
            sound_data = sound_data.fade_in_tween(Tween {
                duration: Duration::from_secs_f64(fade_in),
                easing: Easing::InPowi(2),
                ..Default::default()
            });
        }

        // Set volume
        sound_data = sound_data.volume(amplitude_to_decibels(combined_volume));

        // Play the sound on the track's mixer sub-track
        let sound_handle = active
            .track_handle
            .play(sound_data)
            .map_err(|e| format!("Failed to play sound: {}", e))?;

        active.sound_handle = Some(sound_handle);
        active.current_clip_index = Some(clip_index);
        active.is_paused = false;
        active.pending_start = None;

        Ok(())
    }

    /// Determine the first clip to play for a track.
    fn pick_first_clip(&self, track_id: &str) -> Result<Option<usize>, String> {
        let active = self
            .tracks
            .get(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;

        if active.config.clips.is_empty() {
            return Ok(None);
        }

        match &active.config.mode {
            TrackMode::Shuffle { .. } => {
                let mut rng = rand::thread_rng();
                let idx = rng.gen_range(0..active.config.clips.len());
                Ok(Some(idx))
            }
            _ => Ok(Some(0)),
        }
    }

    /// Start the next clip with the appropriate transition.
    fn start_next_with_transition(
        &mut self,
        track_id: &str,
        next_clip_index: usize,
    ) -> Result<(), String> {
        let transition = {
            let active = self
                .tracks
                .get(track_id)
                .ok_or_else(|| format!("Track not found: {}", track_id))?;

            match &active.config.mode {
                TrackMode::Sequence { transition, .. } => transition.clone(),
                TrackMode::Shuffle { transition, .. } => transition.clone(),
                TrackMode::Seamless { .. } => TrackTransition::Gapless,
            }
        };

        match transition {
            TrackTransition::Gapless => {
                self.start_clip(track_id, next_clip_index)
            }
            TrackTransition::Gap { min, max } => {
                // Schedule the clip to start after a random gap
                let mut rng = rand::thread_rng();
                let gap_secs = rng.gen_range(min..=max);
                let start_at = Instant::now() + Duration::from_secs_f64(gap_secs);

                if let Some(active) = self.tracks.get_mut(track_id) {
                    active.sound_handle = None;
                    active.pending_start = Some(PendingStart {
                        clip_index: next_clip_index,
                        start_at,
                    });
                }
                Ok(())
            }
            TrackTransition::Crossfade { duration } => {
                // The old clip has already stopped naturally. Start the new one with a fade in.
                // We temporarily override the track's fade_in for this clip start.
                let original_fade_in = {
                    let active = self
                        .tracks
                        .get(track_id)
                        .ok_or_else(|| format!("Track not found: {}", track_id))?;
                    active.config.fade_in
                };

                // Set fade_in to crossfade duration for this transition
                if let Some(active) = self.tracks.get_mut(track_id) {
                    active.config.fade_in = duration;
                }

                let result = self.start_clip(track_id, next_clip_index);

                // Restore original fade_in
                if let Some(active) = self.tracks.get_mut(track_id) {
                    active.config.fade_in = original_fade_in;
                }

                result
            }
        }
    }

    /// Recalculate and apply the combined volume to a track's active sound.
    fn update_sound_volume(&mut self, track_id: &str) {
        if let Some(active) = self.tracks.get_mut(track_id) {
            let clip_volume = active
                .current_clip_index
                .and_then(|idx| active.config.clips.get(idx))
                .map(|clip| clip.volume)
                .unwrap_or(1.0);

            let combined = self.master_volume * active.config.volume * clip_volume;

            if let Some(ref mut handle) = active.sound_handle {
                handle.set_volume(
                    amplitude_to_decibels(combined),
                    Tween {
                        duration: Duration::from_millis(50),
                        ..Default::default()
                    },
                );
            }
        }
    }

    /// Whether the engine has any tracks currently playing or paused.
    #[allow(dead_code)]
    pub fn has_active_playback(&self) -> bool {
        self.tracks.values().any(|t| {
            t.sound_handle.is_some() || t.is_paused || t.pending_start.is_some()
        })
    }
}

/// Determine the next clip index for sequence/shuffle modes.
/// Returns None if there's no next clip (end of sequence with repeat=false).
fn pick_next_clip_index(config: &Track, current_index: Option<usize>) -> Option<usize> {
    if config.clips.is_empty() {
        return None;
    }

    match &config.mode {
        TrackMode::Seamless { .. } => {
            // Seamless tracks loop natively via kira, they don't advance clips
            None
        }
        TrackMode::Sequence { repeat, .. } => {
            let current = current_index.unwrap_or(0);
            let next = current + 1;
            if next < config.clips.len() {
                Some(next)
            } else if *repeat {
                Some(0)
            } else {
                None
            }
        }
        TrackMode::Shuffle { repeat, .. } => {
            if *repeat || config.clips.len() > 1 {
                let mut rng = rand::thread_rng();
                // Avoid playing the same clip twice in a row if possible
                if config.clips.len() > 1 {
                    loop {
                        let idx = rng.gen_range(0..config.clips.len());
                        if Some(idx) != current_index {
                            return Some(idx);
                        }
                    }
                } else {
                    Some(0)
                }
            } else {
                None
            }
        }
    }
}
