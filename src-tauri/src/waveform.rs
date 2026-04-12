use std::collections::HashMap;
use std::fs::File;
use std::sync::{LazyLock, Mutex};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Cached decoded audio: mono samples + sample rate.
struct CachedAudio {
    samples: Vec<f32>,
    sample_rate: u32,
}

static WAVEFORM_CACHE: LazyLock<Mutex<HashMap<String, CachedAudio>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Decode an audio file to mono f32 samples and cache the result.
/// If already cached, returns immediately.
fn ensure_cached(file_path: &str) -> Result<(), String> {
    let cache = WAVEFORM_CACHE.lock().map_err(|e| format!("Cache lock poisoned: {}", e))?;
    if cache.contains_key(file_path) {
        return Ok(());
    }
    drop(cache); // release lock before heavy decode work

    // Open the file
    let file = File::open(file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Probe the format
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

    // Create a decoder
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

    // Store in cache
    let mut cache = WAVEFORM_CACHE.lock().map_err(|e| format!("Cache lock poisoned: {}", e))?;
    cache.insert(
        file_path.to_string(),
        CachedAudio {
            samples: all_samples,
            sample_rate,
        },
    );

    Ok(())
}

/// Compute peaks from cached samples for a given time range.
///
/// - `width`: number of peak bins to produce
/// - `start`/`end`: optional time range in seconds. If None, uses the full file.
///
/// Returns (peaks, duration_seconds).
fn peaks_from_cache(
    file_path: &str,
    width: usize,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(Vec<f32>, f64), String> {
    let cache = WAVEFORM_CACHE.lock().map_err(|e| format!("Cache lock poisoned: {}", e))?;
    let cached = cache
        .get(file_path)
        .ok_or_else(|| "Audio not in cache".to_string())?;

    let total_samples = cached.samples.len();
    if total_samples == 0 {
        return Ok((vec![0.0; width], 0.0));
    }

    let sample_rate = cached.sample_rate as f64;
    let duration_seconds = total_samples as f64 / sample_rate;

    // Determine the sample range to bucket
    let range_start_sec = start.unwrap_or(0.0).max(0.0);
    let range_end_sec = end.unwrap_or(duration_seconds).min(duration_seconds);

    if range_end_sec <= range_start_sec {
        return Ok((vec![0.0; width], duration_seconds));
    }

    let start_idx = (range_start_sec * sample_rate) as usize;
    let end_idx = ((range_end_sec * sample_rate) as usize).min(total_samples);

    if end_idx <= start_idx {
        return Ok((vec![0.0; width], duration_seconds));
    }

    let range_len = end_idx - start_idx;
    let samples_per_bucket = range_len as f64 / width as f64;
    let mut peaks = Vec::with_capacity(width);

    for i in 0..width {
        let bucket_start = start_idx + (i as f64 * samples_per_bucket) as usize;
        let bucket_end = (start_idx + (((i + 1) as f64) * samples_per_bucket) as usize).min(end_idx);

        let mut max_val: f32 = 0.0;
        for j in bucket_start..bucket_end {
            let abs = cached.samples[j].abs();
            if abs > max_val {
                max_val = abs;
            }
        }
        peaks.push(max_val);
    }

    // Normalize to 0.0-1.0
    let global_max = peaks.iter().cloned().fold(0.0_f32, f32::max);
    if global_max > 0.0 {
        for peak in &mut peaks {
            *peak /= global_max;
        }
    }

    Ok((peaks, duration_seconds))
}

/// Extract waveform peaks from an audio file.
///
/// Decodes the file once and caches the mono samples. Subsequent calls for the
/// same file (even with different width or time range) skip decoding entirely.
///
/// - `width`: number of peak bins
/// - `start`/`end`: optional time range in seconds (None = full file)
///
/// Returns (peaks, duration_seconds).
pub fn extract_peaks(
    file_path: &str,
    width: u32,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<(Vec<f32>, f64), String> {
    let width = width.max(1) as usize;

    // Decode and cache if not already done
    ensure_cached(file_path)?;

    // Compute peaks from cached samples
    peaks_from_cache(file_path, width, start, end)
}
