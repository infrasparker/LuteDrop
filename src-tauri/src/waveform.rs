use std::fs::File;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Extract waveform peaks from an audio file.
///
/// Returns a tuple of (peaks, duration_seconds) where peaks is a Vec<f32>
/// of length `width`, each value normalized to 0.0-1.0 representing the
/// max absolute amplitude in that time bucket.
pub fn extract_peaks(file_path: &str, width: u32) -> Result<(Vec<f32>, f64), String> {
    let width = width.max(1) as usize;

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
        .ok_or_else(|| "Unknown sample rate".to_string())? as f64;

    let _channels = codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(1);

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
                // Some formats require a reset after seeking; just break
                break;
            }
            Err(e) => {
                return Err(format!("Error reading packet: {}", e));
            }
        };

        // Skip packets from other tracks
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                // Skip corrupt frames
                continue;
            }
            Err(e) => {
                return Err(format!("Decode error: {}", e));
            }
        };

        // Convert to interleaved f32 samples
        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let num_channels = spec.channels.count();

        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        // Mix down to mono by averaging channels
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

    let duration_seconds = total_samples as f64 / sample_rate;

    // Downsample to `width` peaks by taking max absolute value per bucket
    let samples_per_bucket = total_samples as f64 / width as f64;
    let mut peaks = Vec::with_capacity(width);

    for i in 0..width {
        let start = (i as f64 * samples_per_bucket) as usize;
        let end = (((i + 1) as f64) * samples_per_bucket) as usize;
        let end = end.min(total_samples);

        let mut max_val: f32 = 0.0;
        for j in start..end {
            let abs = all_samples[j].abs();
            if abs > max_val {
                max_val = abs;
            }
        }
        peaks.push(max_val);
    }

    // Normalize peaks to 0.0-1.0
    let global_max = peaks.iter().cloned().fold(0.0_f32, f32::max);
    if global_max > 0.0 {
        for peak in &mut peaks {
            *peak /= global_max;
        }
    }

    Ok((peaks, duration_seconds))
}
