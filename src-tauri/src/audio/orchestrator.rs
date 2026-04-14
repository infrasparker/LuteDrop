use std::collections::HashMap;
use kira::sound::PlaybackState;
use kira::track::TrackBuilder;
use kira::AudioManager;
use rand::Rng;
use crate::models::{Track, TrackMode, TrackPlaybackState};
use super::sound::{self, SoundHandle, PlayParams};
use super::mixer::{self, MuteState};
use super::transitions::{TransitionManager, TransitionAction};

/// Must match LOOP_CROSSFADE_SECS in mod.rs.
const LOOP_CROSSFADE_SECS: f64 = 0.06;

pub struct ManagedTrack {
    pub track_handle: kira::track::TrackHandle,
    pub config: Track,
    pub current_sound: Option<SoundHandle>,
    pub current_clip_index: Option<usize>,
    pub is_paused: bool,
    pub transitions: TransitionManager,
}

pub fn setup_track(
    manager: &mut AudioManager,
    tracks: &mut HashMap<String, ManagedTrack>,
    config: &Track,
) -> Result<(), String> {
    if tracks.contains_key(&config.id) {
        update_track_config(tracks, config);
        return Ok(());
    }

    let track_handle = manager
        .add_sub_track(TrackBuilder::new())
        .map_err(|e| format!("Failed to create sub-track: {}", e))?;

    tracks.insert(config.id.clone(), ManagedTrack {
        track_handle,
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
    manager: &mut AudioManager,
    track_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let is_paused = tracks.get(track_id).map(|t| t.is_paused).unwrap_or(false);
    if is_paused {
        return resume_track(tracks, track_id);
    }

    let clip_index = pick_first_clip(tracks, track_id)?;
    match clip_index {
        Some(idx) => start_clip(tracks, manager, track_id, idx, None, true, master_volume, mute_state),
        None => Err(format!("Track {} has no clips", track_id)),
    }
}

pub fn play_track_clip(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    clip_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let clip_index = {
        let track = tracks.get(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
        track.config.clips.iter().position(|c| c.id == clip_id)
            .ok_or_else(|| format!("Clip not found: {}", clip_id))?
    };
    start_clip(tracks, manager, track_id, clip_index, None, true, master_volume, mute_state)
}

pub fn play_track_from(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    position: f64,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let clip_index = pick_first_clip(tracks, track_id)?;
    match clip_index {
        Some(idx) => start_clip(tracks, manager, track_id, idx, Some(position), true, master_volume, mute_state),
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
                Some(snd) => matches!(sound::state(snd), PlaybackState::Playing | PlaybackState::Resuming),
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

pub fn set_track_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    track_id: &str,
    volume: f64,
    master_volume: f64,
    mute_state: &MuteState,
) {
    let found = if let Some(track) = tracks.get_mut(track_id) {
        track.config.volume = volume.clamp(0.0, 1.0);
        true
    } else {
        false
    };
    if found {
        update_sound_volume(tracks, track_id, master_volume, mute_state);
    }
}

pub fn set_clip_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    clip_id: &str,
    volume: f64,
    master_volume: f64,
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
        update_sound_volume(tracks, &track_id, master_volume, mute_state);
    }
}

/// Poll a single track for transition actions and execute them.
pub fn poll_track(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    master_volume: f64,
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
                 matches!(st, PlaybackState::Playing | PlaybackState::Resuming),
                 matches!(st, PlaybackState::Stopped))
            }
            None => (0.0, false, false),
        };
        let dur = track.current_sound.as_ref().and_then(|s| s.duration);
        let start = track.current_sound.as_ref().and_then(|s| s.start_position);
        (pos, dur, start, track.config.mode.clone(), track.current_clip_index, track.config.clips.len(), track.is_paused, playing, stopped)
    };

    // Handle manual loop crossfade for seamless mode before checking transitions
    if sound_playing && !is_paused {
        let needs_loop = {
            let track = match tracks.get(track_id) { Some(t) => t, None => return };
            match &track.current_sound {
                Some(snd) => matches!(snd.loop_region, Some((_, le)) if position >= le - LOOP_CROSSFADE_SECS),
                None => false,
            }
        };
        if needs_loop {
            if let Some(track) = tracks.get_mut(track_id) {
                if let Some(ref mut snd) = track.current_sound {
                    let loop_region = snd.loop_region;
                    sound::stop(snd, Some(std::time::Duration::from_secs_f64(LOOP_CROSSFADE_SECS)));

                    if let (Some((ls, _)), Some(clip_idx)) = (loop_region, track.current_clip_index) {
                        if let Some(clip) = track.config.clips.get(clip_idx) {
                            let volume = mixer::compute_volume(
                                master_volume, track.config.volume, clip.volume, clip.library_volume,
                                mute_state, &track.config.id, &clip.id,
                            );
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
                            match sound::play(manager, &mut track.track_handle, params) {
                                Ok(new_snd) => {
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
            match start_clip(tracks, manager, track_id, next_clip_index, None, false, master_volume, mute_state) {
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
            match start_clip(tracks, manager, track_id, next_clip_index, None, false, master_volume, mute_state) {
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
            match start_clip(tracks, manager, track_id, clip_index, None, false, master_volume, mute_state) {
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
            let playing = matches!(st, PlaybackState::Playing | PlaybackState::Resuming);
            let paused = matches!(st, PlaybackState::Paused | PlaybackState::Pausing | PlaybackState::WaitingToResume);
            (playing, paused, sound::position(snd))
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

fn start_clip(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    clip_index: usize,
    start_position: Option<f64>,
    apply_track_fade: bool,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    let clip = track.config.clips.get(clip_index)
        .ok_or_else(|| format!("Clip index {} out of range", clip_index))?.clone();

    // Stop current sound
    if let Some(ref mut snd) = track.current_sound {
        sound::stop_quick(snd);
    }

    // Compute volume
    let volume = mixer::compute_volume(
        master_volume, track.config.volume, clip.volume, clip.library_volume,
        mute_state, &track.config.id, &clip.id,
    );

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

    let snd = sound::play(manager, &mut track.track_handle, params)?;
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

fn update_sound_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    track_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
) {
    if let Some(track) = tracks.get_mut(track_id) {
        let (clip_vol, lib_vol, clip_id) = track.current_clip_index
            .and_then(|idx| track.config.clips.get(idx))
            .map(|c| (c.volume, c.library_volume, c.id.clone()))
            .unwrap_or((1.0, 1.0, String::new()));

        let vol = mixer::compute_volume(
            master_volume, track.config.volume, clip_vol, lib_vol,
            mute_state, &track.config.id, &clip_id,
        );

        if let Some(ref mut snd) = track.current_sound {
            sound::set_volume(snd, vol);
        }
    }
}
