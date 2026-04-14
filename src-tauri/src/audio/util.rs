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

/// Finds the nearest zero crossing to `target_secs` within ±`window_secs`.
/// Returns the snapped time, or the original target if no crossing is found.
///
/// NOTE: Kira's loop_region performs a hard sample jump at the loop boundary,
/// which causes an audible pop if the waveform amplitude doesn't match at
/// loop_start and loop_end. Snapping loop points to zero crossings eliminates
/// this in most cases. For truly seamless loop crossfading (as FMOD/Wwise
/// provide natively), we would need either a lower-level audio engine or
/// sample-buffer-level control that Kira does not currently expose.
pub fn find_nearest_zero_crossing(path: &str, target_secs: f64, window_secs: f64) -> f64 {
    match find_zero_crossing_inner(path, target_secs, window_secs) {
        Some(t) => t,
        None => target_secs,
    }
}

fn find_zero_crossing_inner(path: &str, target_secs: f64, window_secs: f64) -> Option<f64> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    use symphonia::core::units::Time;

    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;

    let mut format_reader = probed.format;
    let track = format_reader.tracks().iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let sample_rate = codec_params.sample_rate? as f64;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default()).ok()?;

    let range_start = (target_secs - window_secs).max(0.0);
    let range_end = target_secs + window_secs;
    let target_sample = (target_secs * sample_rate) as i64;

    // Seek to just before the search window to avoid decoding from the start
    if range_start > 0.5 {
        let seek_to = (range_start - 0.1).max(0.0);
        let _ = format_reader.seek(
            SeekMode::Coarse,
            SeekTo::Time { time: Time::from(seek_to), track_id: Some(track_id) },
        );
        decoder.reset();
    }

    let mut best_crossing: Option<(i64, f64)> = None;
    let mut prev_sample: f32 = 0.0;
    let mut started = false;

    loop {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track_id { continue; }

        // Compute packet start time from its timestamp
        let packet_ts = packet.ts();
        let time_base = codec_params.time_base;

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(_) => continue,
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let num_channels = spec.channels.count();
        if num_frames == 0 || num_channels == 0 { continue; }

        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        for frame_idx in 0..num_frames {
            let sample_ts = packet_ts + frame_idx as u64;
            let time = match time_base {
                Some(tb) => {
                    let t = tb.calc_time(sample_ts);
                    t.seconds as f64 + t.frac
                }
                None => sample_ts as f64 / sample_rate,
            };

            if time > range_end {
                return best_crossing.map(|(_, t)| round6(t));
            }

            if time >= range_start {
                let mut mono: f32 = 0.0;
                for ch in 0..num_channels {
                    mono += samples[frame_idx * num_channels + ch];
                }
                mono /= num_channels as f32;

                if started && ((prev_sample >= 0.0 && mono < 0.0) || (prev_sample < 0.0 && mono >= 0.0)) {
                    let dist = ((time * sample_rate) as i64 - target_sample).abs();
                    if best_crossing.is_none() || dist < best_crossing.unwrap().0 {
                        best_crossing = Some((dist, time));
                    }
                }
                prev_sample = mono;
                started = true;
            }
        }
    }

    best_crossing.map(|(_, t)| round6(t))
}

fn round6(v: f64) -> f64 {
    (v * 1_000_000.0).round() / 1_000_000.0
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
