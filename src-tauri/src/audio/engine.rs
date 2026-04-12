use std::collections::HashMap;
use std::time::{Duration, Instant};

use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{EndPosition, FromFileError, PlaybackPosition, PlaybackState, Region};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, Easing, Tween};
use rand::Rng;

use crate::models::{Track, TrackMode, TrackPlaybackState, TrackTransition};

/// Quickly probe an audio file for its duration in seconds.
/// Tries metadata first (time_base * n_frames), falls back to
/// byte-length estimate for VBR files without frame counts.
fn probe_duration(path: &str) -> Option<f64> {
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let track = probed.format.tracks().iter().find(|t| {
        t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL
    })?;

    // Try exact duration from metadata
    if let (Some(tb), Some(nf)) = (track.codec_params.time_base, track.codec_params.n_frames) {
        let time = tb.calc_time(nf);
        let dur = time.seconds as f64 + time.frac;
        if dur > 0.0 {
            log::info!("Probed duration for {}: {:.1}s (metadata)", path, dur);
            return Some(dur);
        }
    }

    // Fallback: estimate from sample rate and n_frames if available
    if let (Some(sr), Some(nf)) = (track.codec_params.sample_rate, track.codec_params.n_frames) {
        let dur = nf as f64 / sr as f64;
        if dur > 0.0 {
            log::info!("Probed duration for {}: {:.1}s (sample rate estimate)", path, dur);
            return Some(dur);
        }
    }

    log::warn!("Could not probe duration for {}", path);
    None
}

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
    /// Handle to the previous sound that's fading out during a crossfade.
    outgoing_handle: Option<StreamingSoundHandle<FromFileError>>,
    /// Duration of the currently playing clip in seconds (for crossfade timing).
    current_clip_duration: Option<f64>,
    /// Whether we've already started the crossfade for the current clip.
    crossfade_started: bool,
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
    /// Debug: when true, new clips start 15s before the end.
    pub debug_skip_to_end: bool,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let manager = AudioManager::new(AudioManagerSettings::default())
            .map_err(|e| format!("Failed to create audio manager: {}", e))?;
        Ok(Self {
            manager,
            tracks: HashMap::new(),
            master_volume: 1.0,
            debug_skip_to_end: false,
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
                outgoing_handle: None,
                current_clip_duration: None,
                crossfade_started: false,
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

    /// Play a track starting from a specific clip (by clip ID).
    pub fn play_track_clip(&mut self, track_id: &str, clip_id: &str) -> Result<(), String> {
        let active = self
            .tracks
            .get(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;

        let clip_index = active
            .config
            .clips
            .iter()
            .position(|c| c.id == clip_id)
            .ok_or_else(|| format!("Clip not found: {}", clip_id))?;

        self.start_clip_fresh(track_id, clip_index)
    }

    /// Play a track from a specific time position (in seconds).
    /// Always starts fresh (stops any current sound, ignores pause state).
    pub fn play_track_from(&mut self, track_id: &str, start_position: f64) -> Result<(), String> {
        let clip_index = self.pick_first_clip(track_id)?;
        match clip_index {
            Some(idx) => self.start_clip_at(track_id, idx, Some(start_position)),
            None => Err(format!("Track {} has no clips", track_id)),
        }
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

        // Pick the first clip and start it (fresh start = apply track fade)
        let clip_index = self.pick_first_clip(track_id)?;
        match clip_index {
            Some(idx) => self.start_clip_fresh(track_id, idx),
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

    /// Pause all tracks that are currently playing.
    pub fn pause_all(&mut self) {
        let track_ids: Vec<String> = self.tracks.keys().cloned().collect();
        for track_id in track_ids {
            let is_playing = self.tracks.get(&track_id)
                .map(|t| {
                    if t.is_paused { return false; }
                    match &t.sound_handle {
                        Some(handle) => matches!(handle.state(), PlaybackState::Playing | PlaybackState::Resuming),
                        None => t.pending_start.is_some(),
                    }
                })
                .unwrap_or(false);

            if is_playing {
                let _ = self.pause_track(&track_id);
            }
        }
    }

    /// Resume all tracks that are currently paused.
    pub fn resume_all(&mut self) {
        let track_ids: Vec<String> = self.tracks.keys().cloned().collect();
        for track_id in track_ids {
            let is_paused = self.tracks.get(&track_id)
                .map(|t| t.is_paused)
                .unwrap_or(false);

            if is_paused {
                let _ = self.resume_track(&track_id);
            }
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

            // Clean up finished outgoing crossfade handle
            if let Some(ref handle) = active.outgoing_handle {
                if matches!(handle.state(), PlaybackState::Stopped) {
                    // Will be cleaned up in the mutable pass below
                }
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

        // Clean up stopped outgoing handles
        for active in self.tracks.values_mut() {
            if let Some(ref handle) = active.outgoing_handle {
                if matches!(handle.state(), PlaybackState::Stopped) {
                    active.outgoing_handle = None;
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

    /// Start a clip as part of a transition (no track fade-in).
    fn start_clip(&mut self, track_id: &str, clip_index: usize) -> Result<(), String> {
        self.start_clip_inner(track_id, clip_index, None, false)
    }

    /// Start a clip as a fresh track start (applies track fade-in).
    fn start_clip_fresh(&mut self, track_id: &str, clip_index: usize) -> Result<(), String> {
        self.start_clip_inner(track_id, clip_index, None, true)
    }

    /// Start a clip from a position (applies track fade-in).
    fn start_clip_at(&mut self, track_id: &str, clip_index: usize, start_position: Option<f64>) -> Result<(), String> {
        self.start_clip_inner(track_id, clip_index, start_position, true)
    }

    /// Internal: start a specific clip on a track.
    /// `apply_track_fade` controls whether the track's fade_in is applied
    /// (true for fresh starts, false for clip-to-clip transitions).
    fn start_clip_inner(
        &mut self,
        track_id: &str,
        clip_index: usize,
        start_position: Option<f64>,
        apply_track_fade: bool,
    ) -> Result<(), String> {
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

        // Apply start position if provided (click-to-preview)
        // Debug mode: skip to 15s before end for testing crossfades
        let effective_start = if start_position.is_some() {
            start_position
        } else if self.debug_skip_to_end {
            probe_duration(&clip.file_path).map(|d| (d - 15.0).max(0.0))
        } else {
            None
        };
        if let Some(pos) = effective_start {
            sound_data = sound_data.start_position(pos);
        }

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

        // Apply track fade-in only on fresh starts, not clip-to-clip transitions
        if apply_track_fade {
            let fade_in = active.config.fade_in;
            if fade_in > 0.0 {
                sound_data = sound_data.fade_in_tween(Tween {
                    duration: Duration::from_secs_f64(fade_in),
                    easing: Easing::InPowi(2),
                    ..Default::default()
                });
            }
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
        active.current_clip_duration = probe_duration(&clip.file_path);
        active.crossfade_started = false;
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
                // Old clip already stopped — just start new one with fade in
                self.begin_crossfade(track_id, next_clip_index, duration)
            }
        }
    }

    /// Begin a crossfade: fade out the current sound and start the next clip with fade in.
    /// The old sound keeps playing (fading out) while the new one fades in.
    pub fn begin_crossfade(
        &mut self,
        track_id: &str,
        next_clip_index: usize,
        crossfade_duration: f64,
    ) -> Result<(), String> {
        let active = self
            .tracks
            .get_mut(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;

        log::info!("Beginning crossfade on track {} to clip {}, duration {}s", track_id, next_clip_index, crossfade_duration);

        // Move current sound to outgoing (it will fade out and be cleaned up)
        let mut old_handle = active.sound_handle.take();
        // Drop any previous outgoing handle
        active.outgoing_handle = None;

        // Equal-power crossfade: fade-out stays loud longer (InPowi),
        // fade-in ramps up faster (OutPowi). This keeps combined volume
        // near constant through the overlap instead of dipping at midpoint.
        if let Some(ref mut handle) = old_handle {
            let _ = handle.stop(Tween {
                duration: Duration::from_secs_f64(crossfade_duration),
                easing: Easing::InPowi(2),
                ..Default::default()
            });
        }
        active.outgoing_handle = old_handle;
        active.crossfade_started = true;

        // Load and start the next clip with fade in
        let clip = active
            .config
            .clips
            .get(next_clip_index)
            .ok_or_else(|| format!("Clip index {} out of range", next_clip_index))?
            .clone();

        let mut sound_data = StreamingSoundData::from_file(&clip.file_path)
            .map_err(|e| format!("Failed to load audio file '{}': {}", clip.file_path, e))?;

        let combined_volume = self.master_volume * active.config.volume * clip.volume;
        sound_data = sound_data.volume(amplitude_to_decibels(combined_volume));
        sound_data = sound_data.fade_in_tween(Tween {
            duration: Duration::from_secs_f64(crossfade_duration),
            easing: Easing::OutPowi(2),
            ..Default::default()
        });

        let sound_handle = active
            .track_handle
            .play(sound_data)
            .map_err(|e| format!("Failed to play sound: {}", e))?;

        active.sound_handle = Some(sound_handle);
        active.current_clip_index = Some(next_clip_index);
        active.current_clip_duration = probe_duration(&clip.file_path);
        active.crossfade_started = false;

        Ok(())
    }

    /// Check for tracks that need to start a crossfade (approaching end of clip).
    /// Returns list of (track_id, next_clip_index, crossfade_duration) to process.
    pub fn check_crossfade_triggers(&mut self) -> Vec<(String, usize, f64)> {
        let mut triggers = Vec::new();

        for (track_id, active) in &self.tracks {
            if active.is_paused || active.crossfade_started {
                continue;
            }

            // Only applies to tracks with crossfade transition
            let crossfade_duration = match &active.config.mode {
                TrackMode::Sequence { transition: TrackTransition::Crossfade { duration }, .. } => *duration,
                TrackMode::Shuffle { transition: TrackTransition::Crossfade { duration }, .. } => *duration,
                _ => continue,
            };

            let clip_duration = match active.current_clip_duration {
                Some(d) => d,
                None => continue,
            };

            let position = match &active.sound_handle {
                Some(handle) => {
                    if !matches!(handle.state(), PlaybackState::Playing | PlaybackState::Resuming) {
                        continue;
                    }
                    handle.position()
                }
                None => continue,
            };

            // Trigger crossfade when we're within crossfade_duration of the end
            if position >= clip_duration - crossfade_duration {
                let next = pick_next_clip_index(&active.config, active.current_clip_index);
                if let Some(next_idx) = next {
                    triggers.push((track_id.clone(), next_idx, crossfade_duration));
                }
            }
        }

        triggers
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
