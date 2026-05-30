use std::time::Duration;
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{FromFileError, PlaybackState};
use kira::track::TrackHandle;
use kira::{AudioManager, Decibels, Easing, Tween};
use super::util::probe_duration;

pub struct SoundHandle {
    pub handle: StreamingSoundHandle<FromFileError>,
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
    // Formats symphonia can't stream (opus) are transcoded to a cached WAV here;
    // everything else resolves to its own path unchanged. Either way, kira then
    // streams `playable` from disk — no whole-file decode held in RAM.
    let playable = super::cache::resolve(std::path::Path::new(&params.file_path))?;

    let mut sound_data = StreamingSoundData::from_file(&playable)
        .map_err(|e| format!("Failed to load audio file '{}': {}", params.file_path, e))?;

    // Apply start position
    if params.start_position > 0.0 {
        sound_data = sound_data.start_position(params.start_position);
    }

    // Loop region is NOT applied via Kira's native loop_region because it
    // performs a hard sample jump at the boundary, causing an audible pop.
    // Instead, loop_region is stored on the SoundHandle and managed manually
    // in the poll loop with a crossfade overlap.

    // Apply fade in
    if let Some(fade_secs) = params.fade_in {
        if fade_secs > 0.0 {
            sound_data = sound_data.fade_in_tween(Tween {
                duration: Duration::from_secs_f64(fade_secs),
                easing: Easing::InPowi(2),
                ..Default::default()
            });
        }
    }

    // Apply volume
    sound_data = sound_data.volume(params.volume);

    // Play on the given mixer sub-track
    let handle = track_handle
        .play(sound_data)
        .map_err(|e| format!("Failed to play sound: {}", e))?;

    // Compute effective duration
    let raw_duration = probe_duration(playable.to_str().unwrap_or(&params.file_path));
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
        sound.fade_out
            .map(Duration::from_secs_f64)
            .unwrap_or(Duration::from_millis(10))
    });
    let _ = sound.handle.stop(Tween {
        duration: fade,
        ..Default::default()
    });
}

pub fn stop_quick(sound: &mut SoundHandle) {
    let _ = sound.handle.stop(Tween {
        duration: Duration::from_millis(10),
        ..Default::default()
    });
}

pub fn pause(sound: &mut SoundHandle) {
    sound.handle.pause(Tween {
        duration: Duration::from_millis(50),
        ..Default::default()
    });
}

pub fn resume(sound: &mut SoundHandle) {
    sound.handle.resume(Tween {
        duration: Duration::from_millis(50),
        ..Default::default()
    });
}

pub fn position(sound: &SoundHandle) -> f64 {
    sound.handle.position()
}

pub fn state(sound: &SoundHandle) -> PlaybackState {
    sound.handle.state()
}

pub fn set_volume(sound: &mut SoundHandle, volume: Decibels) {
    sound.handle.set_volume(volume, Tween {
        duration: Duration::from_millis(50),
        ..Default::default()
    });
}
