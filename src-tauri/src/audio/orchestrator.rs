use std::collections::HashMap;
use rand::Rng;
use crate::models::{Track, TrackMode, TrackPlaybackState, TrackTransition};
use super::ffi;
use super::sound::{PlayParams, PlaybackState};
use super::levels::{self, MuteState};
use super::transitions::{TransitionManager, TransitionAction, TickInputs};
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
    /// The next clip index, picked when THIS clip started (D7) so it can be
    /// preloaded ahead of the transition. `None` means the track ends after this
    /// clip. Shuffle commits its random choice here (one clip early).
    pub next_clip_index: Option<usize>,
    /// When the last clip start failed (D8); transition retries are suppressed
    /// for `START_RETRY_BACKOFF` after it.
    pub last_start_failure: Option<std::time::Instant>,
    /// When the last NEXT-CLIP preload (D7) failed (I-1); `request_next_clip_preloads`
    /// suppresses re-requesting this track's preload for `START_RETRY_BACKOFF` after
    /// it, mirroring `last_start_failure`'s protection against a ~20 Hz retry storm.
    /// Cleared when a preload for this track completes `Ok`, or whenever
    /// `next_clip_index` is reset/re-picked (the stamp no longer applies to the
    /// new target).
    pub last_preload_failure: Option<std::time::Instant>,
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
        next_clip_index: None,
        last_start_failure: None,
        last_preload_failure: None,
    });

    Ok(())
}

pub fn update_track_config(tracks: &mut HashMap<String, ManagedTrack>, config: &Track) {
    if let Some(track) = tracks.get_mut(&config.id) {
        track.config = config.clone();
        // The pre-picked next index was computed against the OLD clip list; a
        // shrunk list must not leave an out-of-range stored index (and a
        // changed list makes the pick stale anyway). Re-picked from the live
        // config at the next transition via start_clip.
        track.next_clip_index = None;
        // The stamp (if any) was for a preload target computed against the OLD
        // config; it no longer applies (I-1).
        track.last_preload_failure = None;
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
        Some(idx) => start_clip(tracks, engine, track_id, idx, None, true, mute_state, None),
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
    start_clip(tracks, engine, track_id, clip_index, None, true, mute_state, None)
}

/// Start the track at `position` seconds into a clip's file.
///
/// - With `clip_id`, that clip plays: this is a SEEK by the transport
///   scrubber within the clip that is already playing. It does not count as
///   a new play (`clips_played` is unchanged) and does not restart the
///   track's fade-in.
/// - Without it, the track's first clip is picked as usual (waveform
///   click-to-play), with the track fade-in.
///
/// `paused = true` leaves the track paused at `position` (seeking a paused
/// item). The start and the pause happen here, under the one audio lock the
/// caller holds, so the poll thread never sees the sound playing and the
/// operator hears nothing (the start's fade-in from silence is cut off by the
/// pause's fade-out before it becomes audible).
pub fn play_track_from(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    clip_id: Option<&str>,
    position: f64,
    paused: bool,
    mute_state: &MuteState,
) -> Result<(), String> {
    let clip_index = match clip_id {
        Some(cid) => {
            let track = tracks.get(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
            let idx = track.config.clips.iter().position(|c| c.id == cid)
                .ok_or_else(|| format!("Clip not found: {}", cid))?;
            Some(idx)
        }
        None => pick_first_clip(tracks, track_id)?,
    };
    let Some(idx) = clip_index else {
        return Err(format!("Track {} has no clips", track_id));
    };

    // Remember the play count so a failed start can put it back.
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    let clips_played_before = track.clips_played;
    let is_seek = clip_id.is_some();
    if is_seek {
        // A seek is not a new play: undo the `+= 1` that `start_clip` is
        // about to do, so a sequence's play count is unchanged by the seek.
        track.clips_played = track.clips_played.saturating_sub(1);
    } else {
        track.clips_played = 0;
    }

    // Only a fresh play gets the track's fade-in; a seek must not re-fade.
    let apply_track_fade = !is_seek;
    let result = start_clip(tracks, engine, track_id, idx, Some(position), apply_track_fade, mute_state, None);

    if let Err(e) = result {
        // `start_clip` failed before its `+= 1`, so restore the count we
        // changed above: a failed start leaves the track's count as it was.
        if let Some(track) = tracks.get_mut(track_id) {
            track.clips_played = clips_played_before;
        }
        return Err(e);
    }

    if paused {
        pause_track(tracks, track_id)?;
    }
    Ok(())
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
    // A stopped track has no next clip; leaving it set would make
    // `request_next_clip_preloads` keep re-opening files for a silent track.
    track.next_clip_index = None;
    // The stamp (if any) was for that abandoned next clip; clear it (I-1).
    track.last_preload_failure = None;
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
    pending: &mut HashMap<String, super::loader::PreloadTarget>,
    missing_entries: &mut Vec<String>,
    found_entries: &mut Vec<String>,
) {
    // Drop any held sounds whose fade-out has completed. Done first so it runs
    // every tick regardless of which transition branch returns early below.
    if let Some(track) = tracks.get_mut(track_id) {
        track.voice.drop_finished();
    }

    // Gather state for transition check
    let (position, clip_duration, clip_start, mode, is_paused, sound_playing, sound_stopped) = {
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
        (pos, dur, start, track.config.mode.clone(), track.is_paused, playing, stopped)
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
                    } else if !super::util::is_file_usable(&clip.file_path) {
                        missing_entries.push(clip.library_entry_id.clone());
                    }
                }
            }
            return;
        }
    }

    let (action, suppressed) = {
        let track = match tracks.get(track_id) {
            Some(t) => t,
            None => return,
        };
        let next_idx = track.next_clip_index;
        let suppressed = super::transitions::retry_suppressed(track.last_start_failure, std::time::Instant::now());
        let action = track.transitions.update(&TickInputs {
            position,
            clip_duration,
            clip_start,
            mode: &mode,
            is_paused,
            sound_playing,
            sound_stopped,
            next_clip_index: next_idx,
        });
        (action, suppressed)
    };

    // D8: after a failed start, suppress the transition actions that would
    // re-open a clip for START_RETRY_BACKOFF. CleanupOutgoing / None are not
    // opens, so they still run.
    if suppressed
        && matches!(
            action,
            TransitionAction::StartCrossfade { .. }
                | TransitionAction::AdvanceClip { .. }
                | TransitionAction::ScheduleGap { .. }
                | TransitionAction::StartPendingClip { .. }
        )
    {
        return;
    }

    execute_action(
        tracks, engine, track_id, action, mute_state, pending, missing_entries, found_entries,
        sound_stopped, is_paused,
    );
}

/// After a clip start attempt, record the clip's library entry as found (Ok) or,
/// if its file is missing, as missing (Err). Shared by the transition arms.
fn report_clip_result(
    track: &ManagedTrack,
    clip_index: usize,
    result: &Result<(), String>,
    found_entries: &mut Vec<String>,
    missing_entries: &mut Vec<String>,
) {
    let Some(clip) = track.config.clips.get(clip_index) else { return };
    match result {
        Ok(_) => found_entries.push(clip.library_entry_id.clone()),
        Err(_) => {
            if !super::util::is_file_usable(&clip.file_path) {
                missing_entries.push(clip.library_entry_id.clone());
            }
        }
    }
}

/// Execute the `TransitionAction` decided by `TransitionManager::update`.
/// Extracted from `poll_track` so that function reads as: gather state →
/// seamless-loop poll → suppression gate → `execute_action`.
fn execute_action(
    tracks: &mut HashMap<String, ManagedTrack>,
    engine: &ffi::Engine,
    track_id: &str,
    action: TransitionAction,
    mute_state: &MuteState,
    pending: &mut HashMap<String, super::loader::PreloadTarget>,
    missing_entries: &mut Vec<String>,
    found_entries: &mut Vec<String>,
    sound_stopped: bool,
    is_paused: bool,
) {
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
            let result = start_clip(tracks, engine, track_id, next_clip_index, None, false, mute_state, Some(pending));
            if let Some(track) = tracks.get(track_id) {
                report_clip_result(track, next_clip_index, &result, found_entries, missing_entries);
            }
        }
        TransitionAction::AdvanceClip { next_clip_index } => {
            let result = start_clip(tracks, engine, track_id, next_clip_index, None, false, mute_state, Some(pending));
            if let Some(track) = tracks.get(track_id) {
                report_clip_result(track, next_clip_index, &result, found_entries, missing_entries);
            }
        }
        TransitionAction::ScheduleGap { next_clip_index, delay_seconds } => {
            if let Some(track) = tracks.get_mut(track_id) {
                track.voice.clear_current();
                track.transitions.schedule_gap(next_clip_index, delay_seconds);
            }
        }
        TransitionAction::StartPendingClip { clip_index } => {
            let result = start_clip(tracks, engine, track_id, clip_index, None, false, mute_state, Some(pending));
            if let Some(track) = tracks.get(track_id) {
                report_clip_result(track, clip_index, &result, found_entries, missing_entries);
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
    // `has_sound` is false while a Gap transition waits to start the next
    // clip: the track still counts as playing, but there is no sound (so no
    // meaningful position) until the gap ends.
    let has_sound = track.voice.has_current();
    let (is_playing, is_paused_state, current_time) = if has_sound {
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
        has_sound,
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
    // Some(pending) for poll-driven transitions (may consume a ready preload);
    // None for user-initiated immediate plays (always synchronous, per D9).
    pending: Option<&mut HashMap<String, super::loader::PreloadTarget>>,
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

    // D7: consume a ready-and-matching preload if one exists; else open now.
    let preloaded = pending.and_then(|pend| {
        let matches = pend
            .get(track_id)
            .map(|p| p.clip_index == Some(clip_index) && p.ready.is_some())
            .unwrap_or(false);
        if matches {
            pend.remove(track_id).and_then(|mut p| p.ready.take())
        } else {
            None
        }
    });

    let result = match preloaded {
        Some(handle) => {
            // Preloaded off-lock; attach + start here. Crossfade clips skip the
            // trim-stop (same rule as the synchronous path below).
            track
                .voice
                .start_preloaded(handle, &params, params.volume, fade_in, !uses_crossfade)
        }
        None => {
            if uses_crossfade {
                track.voice.play(engine, params)
            } else {
                track.voice.play_with_trim_stop(engine, params)
            }
        }
    };

    if let Err(e) = result {
        track.last_start_failure = Some(std::time::Instant::now());
        return Err(e);
    }

    track.current_clip_index = Some(clip_index);
    track.clips_played += 1;
    track.transitions.clip_started();
    track.is_paused = false;
    track.last_start_failure = None;
    // Pre-pick the NEXT clip now, so the poll can preload it (D7). Uses the same
    // pick logic that TransitionManager::update used to call.
    track.next_clip_index = super::transitions::pick_next_clip_index(
        &track.config.mode,
        Some(clip_index),
        track.config.clips.len(),
        track.clips_played,
    );
    // The freshly-picked index is a new preload target; any earlier preload
    // failure stamp was for a different (now-stale) target (I-1).
    track.last_preload_failure = None;

    Ok(())
}

/// Build the `PlayParams` for `clip_index` on `track` WITHOUT playing — used to
/// request an off-lock preload of the pre-picked next clip (D7). Returns None if
/// the index is out of range. Mirrors the param building in `start_clip`.
pub fn preload_params_for(
    track: &ManagedTrack,
    clip_index: usize,
    mute_state: &MuteState,
) -> Option<PlayParams> {
    let clip = track.config.clips.get(clip_index)?.clone();
    let volume = levels::clip_sound_volume(clip.volume, clip.library_volume, mute_state.clip_muted(&clip.id));
    let effective_start = clip.trim_start.unwrap_or(0.0);
    let loop_region = match &track.config.mode {
        TrackMode::Seamless { loop_start, loop_end } => {
            let ls = clip.trim_start.map(|ts| (*loop_start).max(ts)).unwrap_or(*loop_start);
            let raw_le = if *loop_end > 0.0 { *loop_end } else { clip.trim_end.unwrap_or(f64::MAX) };
            let le = clip.trim_end.map(|te| raw_le.min(te)).unwrap_or(raw_le);
            if le < f64::MAX && le > ls { Some((ls, le)) } else { None }
        }
        _ => None,
    };
    // Transitions never apply the track fade-in to a mid-sequence clip
    // (apply_track_fade = false in the poll path), so the preloaded next clip
    // carries no fade_in; the crossfade / advance supplies any fade itself.
    Some(PlayParams {
        file_path: clip.file_path.clone(),
        start_position: effective_start,
        volume,
        trim_start: clip.trim_start,
        trim_end: clip.trim_end,
        loop_region,
        fade_in: None,
        fade_out: if track.config.fade_out > 0.0 { Some(track.config.fade_out) } else { None },
    })
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
