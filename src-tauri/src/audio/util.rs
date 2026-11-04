/// Standard crossfade overlap duration in seconds.
pub const CROSSFADE_OVERLAP: f64 = 4.0;

/// File extensions (lowercase, no leading dot) treated as importable audio.
/// Single source of truth for folder scans and import filters on the backend.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "aac", "m4a", "wma", "opus"];

/// Returns true if `path`'s extension (case-insensitive) is in `AUDIO_EXTENSIONS`.
pub fn is_audio_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(ext)))
        .unwrap_or(false)
}

/// The one definition of "this library file is there": it exists, is a
/// regular file (not a folder) and is not empty. A zero-byte file is treated
/// as missing because it is what a failed copy or cloud-sync stub leaves
/// behind, and it can never play. The missing badge, playback and the
/// fix-missing dialog all call this so they always agree.
pub fn is_file_usable(path: &str) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false)
}

/// Quickly probe an audio file for its duration in seconds.
///
/// Uses the miniaudio shim's cheap length probe first (inits a decoder and reads
/// `ma_decoder_get_length_in_pcm_frames` / sample rate — NO full decode). If the
/// format can't report its length that way (e.g. a streamed source with unknown
/// length), falls back to a full decode and computes `frames / sample_rate`.
pub fn probe_duration(path: &str) -> Option<f64> {
    // Cheap path: decoder length query, no full read.
    if let Some(secs) = crate::audio::ffi::file_duration_secs(path) {
        return Some(secs);
    }

    // Fallback: stream-count frames (bounded memory), then frames / sample_rate.
    let mut dec = crate::audio::ffi::StreamingDecoder::open(path).ok()?;
    let channels = dec.channels.max(1) as u64;
    let sample_rate = dec.sample_rate;
    let mut frames: u64 = 0;
    loop {
        match dec.read_chunk() {
            Ok(Some(chunk)) => frames += chunk.len() as u64 / channels,
            Ok(None) => break,
            // A truncated count would report a wrong (short) duration —
            // match the old fallback's behavior and report unknown instead.
            Err(_) => return None,
        }
    }
    if sample_rate == 0 || frames == 0 {
        return None;
    }
    Some(frames as f64 / sample_rate as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn opus_is_recognized_as_audio() {
        assert!(is_audio_path(Path::new("song.opus")));
        assert!(is_audio_path(Path::new("SONG.OPUS")));
    }
}
