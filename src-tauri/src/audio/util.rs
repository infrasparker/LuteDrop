use kira::Decibels;

/// Standard crossfade overlap duration in seconds.
pub const CROSSFADE_OVERLAP: f64 = 4.0;

/// Quickly probe an audio file for its duration in seconds.
/// Tries metadata first (time_base * n_frames), falls back to
/// byte-length estimate for VBR files without frame counts.
pub fn probe_duration(path: &str) -> Option<f64> {
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
