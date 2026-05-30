use std::sync::Arc;
use std::time::Duration;
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::{FromFileError, PlaybackState};
use kira::track::TrackHandle;
use kira::{AudioManager, Decibels, Easing, Tween};
use super::util::probe_duration;
use super::opus_decode::decode_opus;

/// Wraps either a streaming sound (mp3/wav/ogg/flac/etc. via symphonia) or a
/// static, fully-in-memory sound (opus, decoded by us). Both kira handle types
/// expose the same control methods, so callers use `SoundHandle` uniformly.
pub enum Handle {
    Streaming(StreamingSoundHandle<FromFileError>),
    Static(StaticSoundHandle),
}

pub struct SoundHandle {
    pub handle: Handle,
    pub fade_out: Option<f64>,
    pub duration: Option<f64>,
    pub start_position: Option<f64>,
    /// Stored for manual crossfade looping in the poll loop.
    pub loop_region: Option<(f64, f64)>,
}

pub struct PlayParams {
    pub file_path: String,
    pub start_position: f64,
    pub volume: Decibels,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub loop_region: Option<(f64, f64)>,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
}

pub fn play(
    _manager: &mut AudioManager,
    track_handle: &mut TrackHandle,
    params: PlayParams,
) -> Result<SoundHandle, String> {
    // Loop region is NOT applied via Kira's native loop_region because it
    // performs a hard sample jump at the boundary, causing an audible pop.
    // Instead, loop_region is stored on the SoundHandle and managed manually
    // in the poll loop with a crossfade overlap.

    let is_opus = std::path::Path::new(&params.file_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("opus"))
        .unwrap_or(false);

    let handle = if is_opus {
        let decoded = decode_opus(std::path::Path::new(&params.file_path))
            .map_err(|e| format!("Failed to load opus file '{}': {}", params.file_path, e))?;
        let mut sound_data = StaticSoundData {
            sample_rate: decoded.sample_rate,
            frames: Arc::from(decoded.frames),
            settings: Default::default(),
            slice: None,
        };
        if params.start_position > 0.0 {
            sound_data = sound_data.start_position(params.start_position);
        }
        if let Some(fade_secs) = params.fade_in {
            if fade_secs > 0.0 {
                sound_data = sound_data.fade_in_tween(Tween {
                    duration: Duration::from_secs_f64(fade_secs),
                    easing: Easing::InPowi(2),
                    ..Default::default()
                });
            }
        }
        sound_data = sound_data.volume(params.volume);
        Handle::Static(
            track_handle
                .play(sound_data)
                .map_err(|e| format!("Failed to play opus sound: {}", e))?,
        )
    } else {
        let mut sound_data = StreamingSoundData::from_file(&params.file_path)
            .map_err(|e| format!("Failed to load audio file '{}': {}", params.file_path, e))?;
        if params.start_position > 0.0 {
            sound_data = sound_data.start_position(params.start_position);
        }
        if let Some(fade_secs) = params.fade_in {
            if fade_secs > 0.0 {
                sound_data = sound_data.fade_in_tween(Tween {
                    duration: Duration::from_secs_f64(fade_secs),
                    easing: Easing::InPowi(2),
                    ..Default::default()
                });
            }
        }
        sound_data = sound_data.volume(params.volume);
        Handle::Streaming(
            track_handle
                .play(sound_data)
                .map_err(|e| format!("Failed to play sound: {}", e))?,
        )
    };

    // Compute effective duration
    let raw_duration = probe_duration(&params.file_path);
    let duration = match (params.trim_end, raw_duration) {
        (Some(te), _) => Some(te),
        (None, Some(d)) => Some(d),
        _ => None,
    };

    Ok(SoundHandle {
        handle,
        fade_out: params.fade_out,
        duration,
        start_position: if params.start_position > 0.0 {
            Some(params.start_position)
        } else {
            params.trim_start
        },
        loop_region: params.loop_region,
    })
}

pub fn stop(sound: &mut SoundHandle, fade_override: Option<Duration>) {
    let fade = fade_override.unwrap_or_else(|| {
        sound.fade_out.map(Duration::from_secs_f64).unwrap_or(Duration::from_millis(10))
    });
    let tween = Tween { duration: fade, ..Default::default() };
    match &mut sound.handle {
        Handle::Streaming(h) => { let _ = h.stop(tween); }
        Handle::Static(h) => { let _ = h.stop(tween); }
    }
}

pub fn stop_quick(sound: &mut SoundHandle) {
    let tween = Tween { duration: Duration::from_millis(10), ..Default::default() };
    match &mut sound.handle {
        Handle::Streaming(h) => { let _ = h.stop(tween); }
        Handle::Static(h) => { let _ = h.stop(tween); }
    }
}

pub fn pause(sound: &mut SoundHandle) {
    let tween = Tween { duration: Duration::from_millis(50), ..Default::default() };
    match &mut sound.handle {
        Handle::Streaming(h) => { let _ = h.pause(tween); }
        Handle::Static(h) => { let _ = h.pause(tween); }
    }
}

pub fn resume(sound: &mut SoundHandle) {
    let tween = Tween { duration: Duration::from_millis(50), ..Default::default() };
    match &mut sound.handle {
        Handle::Streaming(h) => { let _ = h.resume(tween); }
        Handle::Static(h) => { let _ = h.resume(tween); }
    }
}

pub fn position(sound: &SoundHandle) -> f64 {
    match &sound.handle {
        Handle::Streaming(h) => h.position(),
        Handle::Static(h) => h.position(),
    }
}

pub fn state(sound: &SoundHandle) -> PlaybackState {
    match &sound.handle {
        Handle::Streaming(h) => h.state(),
        Handle::Static(h) => h.state(),
    }
}

pub fn set_volume(sound: &mut SoundHandle, volume: Decibels) {
    let tween = Tween { duration: Duration::from_millis(50), ..Default::default() };
    match &mut sound.handle {
        Handle::Streaming(h) => { let _ = h.set_volume(volume, tween); }
        Handle::Static(h) => { let _ = h.set_volume(volume, tween); }
    }
}
