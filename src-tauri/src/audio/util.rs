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

    // Fallback: full decode, then frames / sample_rate.
    let decoded = crate::audio::ffi::decode_file(path).ok()?;
    if decoded.sample_rate == 0 || decoded.frames == 0 {
        return None;
    }
    Some(decoded.frames as f64 / decoded.sample_rate as f64)
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
