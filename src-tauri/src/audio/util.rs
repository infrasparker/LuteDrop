use kira::Decibels;

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
/// Tries metadata first (time_base * n_frames), falls back to
/// byte-length estimate for VBR files without frame counts.
pub fn probe_duration(path: &str) -> Option<f64> {
    if std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("opus"))
        .unwrap_or(false)
    {
        return crate::audio::opus_decode::opus_duration_secs(std::path::Path::new(path));
    }

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
            return Some(dur);
        }
    }

    // Fallback: estimate from sample rate and n_frames if available
    if let (Some(sr), Some(nf)) = (track.codec_params.sample_rate, track.codec_params.n_frames) {
        let dur = nf as f64 / sr as f64;
        if dur > 0.0 {
            return Some(dur);
        }
    }

    None
}

/// Converts a linear amplitude (0.0 - 1.0) to kira Decibels.
/// Uses the standard formula: dB = 20 * log10(amplitude).
/// Clamps very small values to silence.
pub fn amplitude_to_decibels(amp: f64) -> Decibels {
    if amp <= 0.0001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * (amp as f32).log10())
    }
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
