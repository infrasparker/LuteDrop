use std::fs::File;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Extract waveform peaks from an audio file using seek-based sampling.
///
/// Instead of decoding the entire file, this seeks to evenly-spaced positions
/// and decodes multiple consecutive packets at each to compute the peak value.
/// For a 1-hour file with 1200 peaks and 8 packets per sample, this decodes
/// ~9600 packets (~7% of a typical MP3) — much faster than full decode but
/// much more accurate than single-frame sampling.
///
/// - `width`: number of peak bins
/// - `start`/`end`: optional time range in seconds (None = full file)
///
/// Returns (peaks, total_duration_seconds).
pub fn extract_peaks(
    file_path: &str,
    width: u32,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(Vec<f32>, f64), String> {
    let width = width.max(1) as usize;

    // Try the fast seek-based approach first. Fall back to full decode if
    // the format doesn't support seeking or duration is unknown.
    match extract_peaks_seek(file_path, width, start, end) {
        Ok(result) => Ok(result),
        Err(_) => extract_peaks_full_decode(file_path, width, start, end),
    }
}

/// Fast path: seek to evenly-spaced positions and decode multiple packets each
/// for accurate peak representation.
fn extract_peaks_seek(
    file_path: &str,
    width: usize,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(Vec<f32>, f64), String> {
    let file = File::open(file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(file_path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Failed to probe audio format: {}", e))?;

    let mut format_reader = probed.format;

    // Find the first audio track
    let track = format_reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio track found".to_string())?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| "Unknown sample rate".to_string())?;

    // We need both time_base and n_frames to compute duration.
    let time_base = codec_params
        .time_base
        .ok_or_else(|| "No time base available".to_string())?;
    let n_frames = codec_params
        .n_frames
        .ok_or_else(|| "No frame count available".to_string())?;

    let duration_time = time_base.calc_time(n_frames);
    let duration_seconds = duration_time.seconds as f64 + duration_time.frac;

    if duration_seconds <= 0.0 {
        return Ok((vec![0.0; width], 0.0));
    }

    // Determine the time range to sample
    let range_start = start.unwrap_or(0.0).max(0.0);
    let range_end = end.unwrap_or(duration_seconds).min(duration_seconds);

    if range_end <= range_start {
        return Ok((vec![0.0; width], duration_seconds));
    }

    let range_duration = range_end - range_start;
    let interval = range_duration / width as f64;

    // Create a decoder
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    // Choose seek mode: coarse for full-file overview (speed), accurate for
    // zoomed ranges (precision).
    let seek_mode = if start.is_some() || end.is_some() {
        SeekMode::Accurate
    } else {
        SeekMode::Coarse
    };

    let mut peaks = Vec::with_capacity(width);

    for i in 0..width {
        // Seek to the midpoint of each interval for a representative sample
        let seek_time = range_start + (i as f64 + 0.5) * interval;

        let peak = seek_and_sample(
            &mut format_reader,
            &mut decoder,
            track_id,
            sample_rate,
            seek_mode,
            seek_time,
        );

        peaks.push(peak);
    }

    // Normalize to 0.0–1.0
    let global_max = peaks.iter().cloned().fold(0.0_f32, f32::max);
    if global_max > 0.0 {
        for peak in &mut peaks {
            *peak /= global_max;
        }
    }

    Ok((peaks, duration_seconds))
}

/// Number of consecutive packets to decode at each sample position.
/// More packets = more accurate peak representation at each position.
/// For 1200 peaks this means ~9600 packets total — still a small fraction
/// of a typical file (~140k frames for a 1-hour MP3).
const PACKETS_PER_SAMPLE: usize = 8;

/// Seek to a position, decode multiple consecutive packets, and return the
/// max peak absolute value (mixed to mono) across all of them. Returns 0.0
/// on any error (EOF, seek failure, etc.).
fn seek_and_sample(
    format_reader: &mut Box<dyn FormatReader>,
    decoder: &mut Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    _sample_rate: u32,
    seek_mode: SeekMode,
    seek_seconds: f64,
) -> f32 {
    // Seek to the target time
    let seek_to = SeekTo::Time {
        time: Time::from(seek_seconds),
        track_id: Some(track_id),
    };

    if format_reader.seek(seek_mode, seek_to).is_err() {
        return 0.0;
    }

    let mut overall_max: f32 = 0.0;
    let mut packets_decoded: usize = 0;

    // Decode up to PACKETS_PER_SAMPLE consecutive packets from this position
    // and take the max peak across all of them.
    while packets_decoded < PACKETS_PER_SAMPLE {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(_) => break,
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let num_channels = spec.channels.count();

        if num_frames == 0 || num_channels == 0 {
            packets_decoded += 1;
            continue;
        }

        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        // Find max absolute value in this packet, mixing to mono
        for frame_idx in 0..num_frames {
            let mut mono_sum: f32 = 0.0;
            for ch in 0..num_channels {
                mono_sum += samples[frame_idx * num_channels + ch];
            }
            let mono_val = (mono_sum / num_channels as f32).abs();
            if mono_val > overall_max {
                overall_max = mono_val;
            }
        }

        packets_decoded += 1;
    }

    overall_max
}

/// Fallback: decode the entire file and compute peaks from all samples.
/// Used when seeking is not supported (e.g., piped/non-seekable input).
fn extract_peaks_full_decode(
    file_path: &str,
    width: usize,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(Vec<f32>, f64), String> {
    let file = File::open(file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(file_path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Failed to probe audio format: {}", e))?;

    let mut format_reader = probed.format;

    let track = format_reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio track found".to_string())?;

    let track_id = track.id;
    let codec_params = track.codec_params.clone();

    let sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| "Unknown sample rate".to_string())?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    // Decode all samples into mono
    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format_reader.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                break;
            }
            Err(e) => {
                return Err(format!("Error reading packet: {}", e));
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                continue;
            }
            Err(e) => {
                return Err(format!("Decode error: {}", e));
            }
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let num_channels = spec.channels.count();

        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        for frame_idx in 0..num_frames {
            let mut sum: f32 = 0.0;
            for ch in 0..num_channels {
                sum += samples[frame_idx * num_channels + ch];
            }
            all_samples.push(sum / num_channels as f32);
        }
    }

    let total_samples = all_samples.len();
    if total_samples == 0 {
        return Ok((vec![0.0; width], 0.0));
    }

    let duration_seconds = total_samples as f64 / sample_rate as f64;

    // Determine the sample range to bucket
    let range_start_sec = start.unwrap_or(0.0).max(0.0);
    let range_end_sec = end.unwrap_or(duration_seconds).min(duration_seconds);

    if range_end_sec <= range_start_sec {
        return Ok((vec![0.0; width], duration_seconds));
    }

    let start_idx = (range_start_sec * sample_rate as f64) as usize;
    let end_idx = ((range_end_sec * sample_rate as f64) as usize).min(total_samples);

    if end_idx <= start_idx {
        return Ok((vec![0.0; width], duration_seconds));
    }

    let range_len = end_idx - start_idx;
    let samples_per_bucket = range_len as f64 / width as f64;
    let mut peaks = Vec::with_capacity(width);

    for i in 0..width {
        let bucket_start = start_idx + (i as f64 * samples_per_bucket) as usize;
        let bucket_end =
            (start_idx + (((i + 1) as f64) * samples_per_bucket) as usize).min(end_idx);

        let mut max_val: f32 = 0.0;
        for j in bucket_start..bucket_end {
            let abs = all_samples[j].abs();
            if abs > max_val {
                max_val = abs;
            }
        }
        peaks.push(max_val);
    }

    // Normalize to 0.0–1.0
    let global_max = peaks.iter().cloned().fold(0.0_f32, f32::max);
    if global_max > 0.0 {
        for peak in &mut peaks {
            *peak /= global_max;
        }
    }

    Ok((peaks, duration_seconds))
}
