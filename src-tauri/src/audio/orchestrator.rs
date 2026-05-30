use std::collections::HashMap;
use rand::Rng;
use crate::models::{Track, TrackMode, TrackPlaybackState};
use super::ffi;
use super::sound::{self, SoundHandle, PlayParams, PlaybackState};
use super::mixer::{self, MuteState};
use super::transitions::{TransitionManager, TransitionAction};

/// Must match LOOP_CROSSFADE_SECS in mod.rs.
const LOOP_CROSSFADE_SECS: f64 = 0.06;

pub struct ManagedTrack {
    pub group: ffi::Group,
    pub config: Track,
    pub current_sound: Option<SoundHandle>,
    pub current_clip_index: Option<usize>,
    pub is_paused: bool,
    pub transitions: TransitionManager,
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

    let group = engine
        .create_group()
        .map_err(|e| format!("Failed to create sub-track: {}", e))?;
    group.set_volume(mixer::track_group_volume(
        config.volume,
        mute_state.track_muted(&config.id),
    ));

    tracks.insert(config.id.clone(), ManagedTrack {
        group,
        config: config.clone(),
        current_sound: None,
        current_clip_index: None,
        is_paused: false,
        transitions: TransitionManager::new(),
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
    start_clip(tracks, engine, track_id, clip_index, None, true, mute_state)
}

pub fn play_track_from(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    position: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
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
    if let Some(ref mut snd) = track.current_sound {
        sound::pause(snd);
    }
    Ok(())
}

pub fn resume_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    track.is_paused = false;
    if let Some(ref mut snd) = track.current_sound {
        sound::resume(snd);
    }
    Ok(())
}

pub fn stop_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = match tracks.get_mut(track_id) {
        Some(t) => t,
        None => return Ok(()),
    };
    track.is_paused = false;
    track.current_clip_index = None;
    track.transitions.reset();
    if let Some(ref mut snd) = track.current_sound {
        let fade = if track.config.fade_out > 0.0 {
            Some(std::time::Duration::from_secs_f64(track.config.fade_out))
        } else {
            None
        };
        sound::stop(snd, fade);
    }
    track.current_sound = None;
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
            match &t.current_sound {
                Some(snd) => matches!(sound::state(snd), PlaybackState::Playing),
                None => t.transitions.has_pending(),
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
        track.group.set_volume(mixer::track_group_volume(
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
        let vol = mixer::track_group_volume(track.config.volume, mute_state.track_muted(track_id));
        track.group.set_volume(vol);
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
    // Gather state for transition check
    let (position, clip_duration, clip_start, mode, current_clip_index, clip_count, is_paused, sound_playing, sound_stopped) = {
        let track = match tracks.get(track_id) {
            Some(t) => t,
            None => return,
        };
        let (pos, playing, stopped) = match &track.current_sound {
            Some(snd) => {
                let st = sound::state(snd);
                (sound::position(snd),
                 matches!(st, PlaybackState::Playing),
                 matches!(st, PlaybackState::Stopped))
            }
            None => (0.0, false, false),
        };
        let dur = track.current_sound.as_ref().and_then(|s| s.duration);
        let start = track.current_sound.as_ref().and_then(|s| s.start_position);
        (pos, dur, start, track.config.mode.clone(), track.current_clip_index, track.config.clips.len(), track.is_paused, playing, stopped)
    };

    // Handle seamless loop crossfade before checking transitions.
    if sound_playing && !is_paused {
        let needs_loop = {
            let track = match tracks.get(track_id) { Some(t) => t, None => return };
            match &track.current_sound {
                Some(snd) => matches!(snd.loop_region, Some((_, le)) if position >= le - LOOP_CROSSFADE_SECS),
                None => false,
            }
        };
        if needs_loop {
            schedule_loop_restart(tracks, engine, track_id, position, mute_state, missing_entries, found_entries);
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
            current_clip_index, clip_count, is_paused, sound_playing, sound_stopped,
        )
    };

    match action {
        TransitionAction::StartCrossfade { next_clip_index, duration } => {
            if let Some(track) = tracks.get_mut(track_id) {
                let old_sound = track.current_sound.take();
                if let Some(old) = old_sound {
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
                track.current_sound = None;
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
    let (is_playing, is_paused_state, current_time) = match &track.current_sound {
        Some(snd) => {
            let st = sound::state(snd);
            (matches!(st, PlaybackState::Playing), matches!(st, PlaybackState::Paused), sound::position(snd))
        }
        None => (track.transitions.has_pending(), track.is_paused, 0.0),
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

/// Sample-accurate seamless loop restart: create the next loop instance and
/// schedule it to begin exactly at the loop boundary (engine clock + lead),
/// while the outgoing sound fades out over the crossfade window.
fn schedule_loop_restart(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    position: f64,
    mute_state: &MuteState,
    missing_entries: &mut Vec<String>,
    found_entries: &mut Vec<String>,
) {
    let track = match tracks.get_mut(track_id) {
        Some(t) => t,
        None => return,
    };

    let (loop_region, clip_idx) = match (&track.current_sound, track.current_clip_index) {
        (Some(snd), Some(idx)) => (snd.loop_region, idx),
        _ => return,
    };
    let (ls, le) = match loop_region {
        Some(r) => r,
        None => return,
    };

    let clip = match track.config.clips.get(clip_idx) {
        Some(c) => c.clone(),
        None => return,
    };

    // Lead = how many engine frames until the loop point, so the next instance
    // starts sample-accurately at `le`. Uses the ENGINE sample rate (the device
    // clock that schedule_start_frame is measured in), not the file rate.
    let seconds_until_loop = (le - position).max(0.0);
    let lead = (seconds_until_loop * engine.sample_rate() as f64) as u64;
    let start_frame = engine.time_frames() + lead;

    let volume = mixer::clip_sound_volume(clip.volume, clip.library_volume, mute_state.clip_muted(&clip.id));

    // Fade the outgoing instance out over the crossfade window.
    if let Some(ref mut snd) = track.current_sound {
        sound::stop(snd, Some(std::time::Duration::from_secs_f64(LOOP_CROSSFADE_SECS)));
    }

    let params = PlayParams {
        file_path: clip.file_path.clone(),
        start_position: ls,
        volume,
        trim_start: clip.trim_start,
        trim_end: clip.trim_end,
        loop_region,
        fade_in: Some(LOOP_CROSSFADE_SECS),
        fade_out: None,
    };

    match sound::play(engine, &track.group, params) {
        Ok(new_snd) => {
            new_snd.sound.schedule_start_frame(start_frame);
            track.current_sound = Some(new_snd);
            found_entries.push(clip.library_entry_id.clone());
        }
        Err(_) => {
            if !std::path::Path::new(&clip.file_path).exists() {
                missing_entries.push(clip.library_entry_id.clone());
            }
        }
    }
}

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

    // Stop current sound
    if let Some(ref mut snd) = track.current_sound {
        sound::stop_quick(snd);
    }

    // Clip-level (sound node) volume. Track/master gain come from group/engine nodes.
    let volume = mixer::clip_sound_volume(clip.volume, clip.library_volume, mute_state.clip_muted(&clip.id));

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

    let snd = sound::play(engine, &track.group, params)?;
    track.current_sound = Some(snd);
    track.current_clip_index = Some(clip_index);
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

        let vol = mixer::clip_sound_volume(clip_vol, lib_vol, mute_state.clip_muted(&clip_id));

        if let Some(ref mut snd) = track.current_sound {
            sound::set_volume(snd, vol);
        }
    }
}
