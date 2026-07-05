use std::collections::HashMap;
use rand::Rng;
use crate::models::{Track, TrackMode, TrackPlaybackState, TrackTransition};
use super::ffi;
use super::sound::{PlayParams, PlaybackState};
use super::levels::{self, MuteState};
use super::transitions::{TransitionManager, TransitionAction};
use super::voice::{LoopAnchor, StopAtEnd, Voice};

pub struct ManagedTrack {
    /// Holds the *outgoing* crossfade sound (`OutgoingFade.handle`), which
    /// lives in `voice`'s group. Declared before `voice` so it drops first:
    /// sounds must be uninitialized before their group (see ffi.rs drop-order
    /// contract). Reordering these fields reintroduces mid-crossfade UB.
    pub transitions: TransitionManager,
    /// Single-sound playback for this track: owns the sub-mix group, the current
    /// clip's sound, the de-click fade-out reaper, and the seamless-loop
    /// crossfade. The multi-clip sequencing (which/when) lives in this module +
    /// `TransitionManager`; the actual play/stop/loop/reap is delegated to the
    /// Voice. Must outlive `transitions` (drops after it).
    pub voice: Voice,
    pub config: Track,
    pub current_clip_index: Option<usize>,
    pub clips_played: usize,
    pub is_paused: bool,
}

pub fn setup_track(
    engine: &ffi::Engine,
    tracks: &mut HashMap<String, ManagedTrack>,
    mute_state: &MuteState,
    config: &Track,
) -> Result<(), String> {
    if tracks.contains_key(&config.id) {
        update_track_config(tracks, config);
        return Ok(());
    }

    // Tracks use sample-accurate scheduled loop restarts anchored to the engine
    // clock, unlike the library preview.
    let voice = Voice::with_loop_anchor(engine, LoopAnchor::EngineClock)
        .map_err(|e| format!("Failed to create sub-track: {}", e))?;
    voice.set_group_volume(levels::track_group_volume(
        config.volume,
        mute_state.track_muted(&config.id),
    ));

    tracks.insert(config.id.clone(), ManagedTrack {
        transitions: TransitionManager::new(),
        voice,
        config: config.clone(),
        current_clip_index: None,
        clips_played: 0,
        is_paused: false,
    });

    Ok(())
}

pub fn update_track_config(tracks: &mut HashMap<String, ManagedTrack>, config: &Track) {
    if let Some(track) = tracks.get_mut(&config.id) {
        track.config = config.clone();
    }
}

pub fn has_track(tracks: &HashMap<String, ManagedTrack>, track_id: &str) -> bool {
    tracks.contains_key(track_id)
}

pub fn play_track(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    mute_state: &MuteState,
) -> Result<(), String> {
    let is_paused = tracks.get(track_id).map(|t| t.is_paused).unwrap_or(false);
    if is_paused {
        return resume_track(tracks, track_id);
    }

    if let Some(track) = tracks.get_mut(track_id) {
        track.clips_played = 0;
    }

    let clip_index = pick_first_clip(tracks, track_id)?;
    match clip_index {
        Some(idx) => start_clip(tracks, engine, track_id, idx, None, true, mute_state),
        None => Err(format!("Track {} has no clips", track_id)),
    }
}

pub fn play_track_clip(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    clip_id: &str,
    mute_state: &MuteState,
) -> Result<(), String> {
    let clip_index = {
        let track = tracks.get(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
        track.config.clips.iter().position(|c| c.id == clip_id)
            .ok_or_else(|| format!("Clip not found: {}", clip_id))?
    };
    if let Some(track) = tracks.get_mut(track_id) {
        track.clips_played = 0;
    }
    start_clip(tracks, engine, track_id, clip_index, None, true, mute_state)
}

pub fn play_track_from(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    position: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    if let Some(track) = tracks.get_mut(track_id) {
        track.clips_played = 0;
    }
    let clip_index = pick_first_clip(tracks, track_id)?;
    match clip_index {
        Some(idx) => start_clip(tracks, engine, track_id, idx, Some(position), true, mute_state),
        None => Err(format!("Track {} has no clips", track_id)),
    }
}

pub fn pause_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = match tracks.get_mut(track_id) {
        Some(t) => t,
        None => return Ok(()),
    };
    track.is_paused = true;
    track.transitions.reset();
    track.voice.pause();
    Ok(())
}

pub fn resume_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    track.is_paused = false;
    track.voice.resume();
    Ok(())
}

pub fn stop_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = match tracks.get_mut(track_id) {
        Some(t) => t,
        None => return Ok(()),
    };
    track.is_paused = false;
    track.current_clip_index = None;
    track.clips_played = 0;
    track.transitions.reset();
    // Deferred destruction: fade the sound out and park it until the ramp
    // completes, instead of dropping it immediately (which would uninit it
    // mid-fade → click). The Voice's reaper (in `poll_track`) drops it after.
    let fade = if track.config.fade_out > 0.0 { Some(track.config.fade_out) } else { Some(super::voice::DEFAULT_STOP_FADE_SECS) };
    track.voice.stop(fade);
    Ok(())
}

pub fn stop_all(tracks: &mut HashMap<String, ManagedTrack>) {
    let ids: Vec<String> = tracks.keys().cloned().collect();
    for id in ids {
        let _ = stop_track(tracks, &id);
    }
}

pub fn pause_all(tracks: &mut HashMap<String, ManagedTrack>) {
    let ids: Vec<String> = tracks.keys().cloned().collect();
    for id in ids {
        let is_playing = tracks.get(&id).map(|t| {
            if t.is_paused { return false; }
            if t.voice.has_current() {
                matches!(t.voice.state(), PlaybackState::Playing)
            } else {
                t.transitions.has_pending()
            }
        }).unwrap_or(false);
        if is_playing {
            let _ = pause_track(tracks, &id);
        }
    }
}

pub fn resume_all(tracks: &mut HashMap<String, ManagedTrack>) {
    let ids: Vec<String> = tracks.keys().cloned().collect();
    for id in ids {
        let is_paused = tracks.get(&id).map(|t| t.is_paused).unwrap_or(false);
        if is_paused {
            let _ = resume_track(tracks, &id);
        }
    }
}

/// Set a track's volume (group node). Master mute/volume is the engine node;
/// clip volumes are sound nodes — neither is touched here.
pub fn set_track_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    track_id: &str,
    volume: f64,
    mute_state: &MuteState,
) {
    if let Some(track) = tracks.get_mut(track_id) {
        track.config.volume = volume.clamp(0.0, 1.0);
        track.voice.set_group_volume(levels::track_group_volume(
            track.config.volume,
            mute_state.track_muted(track_id),
        ));
    }
}

/// Re-apply a track's group volume from its stored config + current mute state.
/// Used after a mute toggle.
pub fn refresh_track_group_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    track_id: &str,
    mute_state: &MuteState,
) {
    if let Some(track) = tracks.get(track_id) {
        let vol = levels::track_group_volume(track.config.volume, mute_state.track_muted(track_id));
        track.voice.set_group_volume(vol);
    }
}

pub fn set_clip_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    clip_id: &str,
    volume: f64,
    mute_state: &MuteState,
) {
    let volume = volume.clamp(0.0, 1.0);
    let mut found_track_id: Option<String> = None;
    for (track_id, track) in tracks.iter_mut() {
        for clip in &mut track.config.clips {
            if clip.id == clip_id {
                clip.volume = volume;
                found_track_id = Some(track_id.clone());
                break;
            }
        }
        if found_track_id.is_some() { break; }
    }
    if let Some(track_id) = found_track_id {
        refresh_current_sound_volume(tracks, &track_id, mute_state);
    }
}

/// Poll a single track for transition actions and execute them.
pub fn poll_track(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    mute_state: &MuteState,
    missing_entries: &mut Vec<String>,
    found_entries: &mut Vec<String>,
) {
    // Drop any held sounds whose fade-out has completed. Done first so it runs
    // every tick regardless of which transition branch returns early below.
    if let Some(track) = tracks.get_mut(track_id) {
        track.voice.drop_finished();
    }

    // Gather state for transition check
    let (position, clip_duration, clip_start, mode, current_clip_index, clip_count, clips_played, is_paused, sound_playing, sound_stopped) = {
        let track = match tracks.get(track_id) {
            Some(t) => t,
            None => return,
        };
        let (pos, playing, stopped) = if track.voice.has_current() {
            let st = track.voice.state();
            (track.voice.position(),
             matches!(st, PlaybackState::Playing),
             matches!(st, PlaybackState::Stopped))
        } else {
            (0.0, false, false)
        };
        let dur = track.voice.duration();
        let start = track.voice.start_position();
        (pos, dur, start, track.config.mode.clone(), track.current_clip_index, track.config.clips.len(), track.clips_played, track.is_paused, playing, stopped)
    };

    // Handle the seamless loop crossfade before checking transitions. The Voice
    // performs the crossfade (anchored restart + park the outgoing) when within
    // the crossfade window; tracks gate it on `!is_paused` and never stop a
    // non-looping sound at trim_end here (the transition state machine does).
    {
        let looped = {
            let track = match tracks.get_mut(track_id) { Some(t) => t, None => return };
            matches!(
                track.voice.poll_loop(engine, sound_playing && !is_paused, StopAtEnd::No),
                super::voice::PollResult::Looped
            )
        };
        if looped {
            // Report the restarted clip's library entry, or a missing file, the
            // same way the old `schedule_loop_restart` did.
            if let Some(track) = tracks.get(track_id) {
                if let Some(clip) = track.current_clip_index.and_then(|idx| track.config.clips.get(idx)) {
                    if track.voice.has_current() {
                        found_entries.push(clip.library_entry_id.clone());
                    } else if !std::path::Path::new(&clip.file_path).exists() {
                        missing_entries.push(clip.library_entry_id.clone());
                    }
                }
            }
            return;
        }
    }

    let action = {
        let track = match tracks.get(track_id) {
            Some(t) => t,
            None => return,
        };
        track.transitions.update(
            position, clip_duration, clip_start, &mode,
            current_clip_index, clip_count, clips_played, is_paused, sound_playing, sound_stopped,
        )
    };

    match action {
        TransitionAction::StartCrossfade { next_clip_index, duration } => {
            if let Some(track) = tracks.get_mut(track_id) {
                // Hand the outgoing sound to the clip-crossfade transition (which
                // owns its own outgoing fade + cleanup). `take_current` removes it
                // without stopping so the transition can fade it.
                if let Some(old) = track.voice.take_current() {
                    track.transitions.begin_crossfade(old, duration);
                }
            }
            match start_clip(tracks, engine, track_id, next_clip_index, None, false, mute_state) {
                Ok(_) => {
                    if let Some(track) = tracks.get(track_id) {
                        if let Some(clip) = track.config.clips.get(next_clip_index) {
                            found_entries.push(clip.library_entry_id.clone());
                        }
                    }
                }
                Err(_) => {
                    if let Some(track) = tracks.get(track_id) {
                        if let Some(clip) = track.config.clips.get(next_clip_index) {
                            if !std::path::Path::new(&clip.file_path).exists() {
                                missing_entries.push(clip.library_entry_id.clone());
                            }
                        }
                    }
                }
            }
        }
        TransitionAction::AdvanceClip { next_clip_index } => {
            match start_clip(tracks, engine, track_id, next_clip_index, None, false, mute_state) {
                Ok(_) => {
                    if let Some(track) = tracks.get(track_id) {
                        if let Some(clip) = track.config.clips.get(next_clip_index) {
                            found_entries.push(clip.library_entry_id.clone());
                        }
                    }
                }
                Err(_) => {
                    if let Some(track) = tracks.get(track_id) {
                        if let Some(clip) = track.config.clips.get(next_clip_index) {
                            if !std::path::Path::new(&clip.file_path).exists() {
                                missing_entries.push(clip.library_entry_id.clone());
                            }
                        }
                    }
                }
            }
        }
        TransitionAction::ScheduleGap { next_clip_index, delay_seconds } => {
            if let Some(track) = tracks.get_mut(track_id) {
                track.voice.clear_current();
                track.transitions.schedule_gap(next_clip_index, delay_seconds);
            }
        }
        TransitionAction::StartPendingClip { clip_index } => {
            match start_clip(tracks, engine, track_id, clip_index, None, false, mute_state) {
                Ok(_) => {
                    if let Some(track) = tracks.get(track_id) {
                        if let Some(clip) = track.config.clips.get(clip_index) {
                            found_entries.push(clip.library_entry_id.clone());
                        }
                    }
                }
                Err(_) => {
                    if let Some(track) = tracks.get(track_id) {
                        if let Some(clip) = track.config.clips.get(clip_index) {
                            if !std::path::Path::new(&clip.file_path).exists() {
                                missing_entries.push(clip.library_entry_id.clone());
                            }
                        }
                    }
                }
            }
        }
        TransitionAction::CleanupOutgoing => {
            if let Some(track) = tracks.get_mut(track_id) {
                track.transitions.cleanup_outgoing();
            }
        }
        TransitionAction::None => {
            // Check if sound stopped naturally (no transition action means end of track)
            if sound_stopped && !is_paused {
                let _ = stop_track(tracks, track_id);
            }
        }
    }
}

pub fn get_playback_state(track: &ManagedTrack) -> TrackPlaybackState {
    let (is_playing, is_paused_state, current_time) = if track.voice.has_current() {
        let st = track.voice.state();
        (matches!(st, PlaybackState::Playing), matches!(st, PlaybackState::Paused), track.voice.position())
    } else {
        (track.transitions.has_pending(), track.is_paused, 0.0)
    };

    let current_clip_id = track.current_clip_index
        .and_then(|idx| track.config.clips.get(idx))
        .map(|clip| clip.id.clone());

    TrackPlaybackState {
        track_id: track.config.id.clone(),
        is_playing,
        is_paused: is_paused_state,
        current_clip_id,
        current_time,
    }
}

// --- Internal helpers ---

fn start_clip(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    clip_index: usize,
    start_position: Option<f64>,
    apply_track_fade: bool,
    mute_state: &MuteState,
) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    let clip = track.config.clips.get(clip_index)
        .ok_or_else(|| format!("Clip index {} out of range", clip_index))?.clone();

    // Clip-level (sound node) volume. Track/master gain come from group/engine nodes.
    let volume = levels::clip_sound_volume(clip.volume, clip.library_volume, mute_state.clip_muted(&clip.id));

    // Build play params
    let effective_start = start_position.or(clip.trim_start).unwrap_or(0.0);

    // Compute loop region for seamless mode
    let loop_region = match &track.config.mode {
        TrackMode::Seamless { loop_start, loop_end } => {
            let ls = clip.trim_start.map(|ts| (*loop_start).max(ts)).unwrap_or(*loop_start);
            let raw_le = if *loop_end > 0.0 { *loop_end } else { clip.trim_end.unwrap_or(f64::MAX) };
            let le = clip.trim_end.map(|te| raw_le.min(te)).unwrap_or(raw_le);
            if le < f64::MAX && le > ls {
                Some((ls, le))
            } else {
                None
            }
        }
        _ => None,
    };

    let fade_in = if apply_track_fade && track.config.fade_in > 0.0 {
        Some(track.config.fade_in)
    } else {
        None
    };

    let params = PlayParams {
        file_path: clip.file_path.clone(),
        start_position: effective_start,
        volume,
        trim_start: clip.trim_start,
        trim_end: clip.trim_end,
        loop_region,
        fade_in,
        fade_out: if track.config.fade_out > 0.0 { Some(track.config.fade_out) } else { None },
    };

    // Scheduled de-clicked stop at trim_end — replaces the old late poll-check
    // hard cut — EXCEPT for crossfade transitions: there the transition
    // machine starts the next clip before trim_end and hands the outgoing off
    // (`take_current` + `begin_crossfade`) for its own fade, so a scheduled
    // trim-end stop would short-cut the crossfade. Looping (Seamless) clips
    // also skip it, but `play_with_trim_stop` already no-ops when
    // `loop_region` is set, so we only special-case crossfade here. Gapless /
    // Gap advance off the resulting Stopped state, which the scheduled stop
    // still produces.
    let uses_crossfade = matches!(
        &track.config.mode,
        TrackMode::Sequence { transition: TrackTransition::Crossfade, .. }
            | TrackMode::Shuffle { transition: TrackTransition::Crossfade, .. }
    );

    if uses_crossfade {
        track.voice.play(engine, params)?;
    } else {
        track.voice.play_with_trim_stop(engine, params)?;
    }
    track.current_clip_index = Some(clip_index);
    track.clips_played += 1;
    track.transitions.clip_started();
    track.is_paused = false;

    Ok(())
}

fn pick_first_clip(tracks: &HashMap<String, ManagedTrack>, track_id: &str) -> Result<Option<usize>, String> {
    let track = tracks.get(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    if track.config.clips.is_empty() {
        return Ok(None);
    }
    match &track.config.mode {
        TrackMode::Shuffle { .. } => {
            let mut rng = rand::thread_rng();
            Ok(Some(rng.gen_range(0..track.config.clips.len())))
        }
        _ => Ok(Some(0)),
    }
}

fn refresh_current_sound_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    track_id: &str,
    mute_state: &MuteState,
) {
    if let Some(track) = tracks.get_mut(track_id) {
        let (clip_vol, lib_vol, clip_id) = track.current_clip_index
            .and_then(|idx| track.config.clips.get(idx))
            .map(|c| (c.volume, c.library_volume, c.id.clone()))
            .unwrap_or((1.0, 1.0, String::new()));

        let vol = levels::clip_sound_volume(clip_vol, lib_vol, mute_state.clip_muted(&clip_id));
        track.voice.set_volume(vol);
    }
}
