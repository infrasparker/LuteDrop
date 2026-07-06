use std::time::Duration;
use super::ffi;

/// Playback state of a single sound. Replaces kira's (the audio library this
/// engine replaced) `PlaybackState`; the audio module (`mod.rs`,
/// `orchestrator.rs`) matches on these variants.
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
}

/// One playing/loaded clip backed by a miniaudio `ma_sound`.
pub struct SoundHandle {
    pub sound: ffi::Sound,
    pub fade_out: Option<f64>,
    pub duration: Option<f64>,
    pub start_position: Option<f64>,
    /// Stored for manual crossfade looping in the poll loop.
    pub loop_region: Option<(f64, f64)>,
    /// We track paused state ourselves; miniaudio reports a paused sound as
    /// "not playing", which is indistinguishable from "stopped" otherwise.
    pub paused: bool,
}

#[derive(Clone)]
pub struct PlayParams {
    pub file_path: String,
    pub start_position: f64,
    /// Linear volume (0.0..=1.0). miniaudio multiplies engine × group × sound.
    pub volume: f32,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub loop_region: Option<(f64, f64)>,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
}

/// Create a streaming sound in `group` and start it.
///
/// Opus (and every other format) streams natively now — no path resolver /
/// transcode cache. Loop region is NOT applied to the sound itself; it is stored
/// on the handle and managed with a crossfade overlap by the poll loop.
pub fn play(
    engine: &ffi::Engine,
    group: &ffi::Group,
    params: PlayParams,
) -> Result<SoundHandle, String> {
    let sound = engine.create_sound(Some(group), &params.file_path, true)?;
    sound.set_volume(params.volume);

    let fade_in = params.fade_in.unwrap_or(0.0);
    sound
        .play(params.start_position, fade_in)
        .map_err(|e| format!("Failed to play sound '{}': {}", params.file_path, e))?;

    // Effective duration: explicit trim_end wins, else the file length.
    let duration = match params.trim_end {
        Some(te) => Some(te),
        None => {
            let len = sound.length_secs();
            if len > 0.0 {
                Some(len)
            } else {
                None
            }
        }
    };

    Ok(SoundHandle {
        sound,
        fade_out: params.fade_out,
        duration,
        start_position: if params.start_position > 0.0 {
            Some(params.start_position)
        } else {
            params.trim_start
        },
        loop_region: params.loop_region,
        paused: false,
    })
}

/// Create a group-less streaming sound and pre-seek it to its start point,
/// WITHOUT attaching it to a group or starting it. Runs on the loader thread
/// (both the open and the seek may do file I/O). The poll tick later attaches
/// it to a live group, sets volume, and starts it (`Voice::start_preloaded`).
pub fn preload(engine: &ffi::EngineRef, params: &PlayParams) -> Result<SoundHandle, String> {
    let sound = engine.create_sound_unattached(&params.file_path)?;

    // Seek to the same effective start `start_clip`/`play` would use. Because
    // ld_sound_play only seeks when start_secs > 0, `start_preloaded` starts
    // this sound with start_secs == 0 and it begins from HERE — no re-seek,
    // which is the whole point of pre-seeking off-lock.
    let seek_to = params.start_position.max(params.trim_start.unwrap_or(0.0));
    if seek_to > 0.0 {
        sound.seek_secs(seek_to);
    }

    let duration = match params.trim_end {
        Some(te) => Some(te),
        None => {
            let len = sound.length_secs();
            if len > 0.0 { Some(len) } else { None }
        }
    };

    Ok(SoundHandle {
        sound,
        fade_out: params.fade_out,
        duration,
        start_position: if params.start_position > 0.0 {
            Some(params.start_position)
        } else {
            params.trim_start
        },
        loop_region: params.loop_region,
        paused: false,
    })
}

pub fn stop(sound: &mut SoundHandle, fade_override: Option<Duration>) {
    let fade = fade_override
        .map(|d| d.as_secs_f64())
        .or(sound.fade_out)
        .unwrap_or(0.01);
    sound.sound.stop(fade);
    sound.paused = false;
}

/// Stop with a minimal de-click fade (~4 ms floor in the shim); cursor is
/// discarded and the sound is not parked for the reaper.
pub fn stop_quick(sound: &mut SoundHandle) {
    sound.sound.stop(0.0);
    sound.paused = false;
}

pub fn pause(sound: &mut SoundHandle) {
    sound.sound.pause();
    sound.paused = true;
}

pub fn resume(sound: &mut SoundHandle) {
    sound.sound.resume();
    sound.paused = false;
}

pub fn position(sound: &SoundHandle) -> f64 {
    sound.sound.position_secs()
}

pub fn state(sound: &SoundHandle) -> PlaybackState {
    if sound.paused {
        PlaybackState::Paused
    } else if sound.sound.at_end() || !sound.sound.is_playing() {
        PlaybackState::Stopped
    } else {
        PlaybackState::Playing
    }
}

pub fn set_volume(sound: &mut SoundHandle, volume: f32) {
    sound.sound.set_volume(volume);
}
