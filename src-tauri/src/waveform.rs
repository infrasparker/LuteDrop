use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// How many raw audio samples are condensed into one peak value.
/// 256 is the Audacity/DAW standard — good balance of resolution vs file size.
const SAMPLES_PER_PEAK: u32 = 256;

/// Magic bytes at the start of every .peaks file.
const MAGIC: &[u8; 4] = b"LDPK";

/// Cache format version. Bump this if the binary layout changes.
const FORMAT_VERSION: u8 = 1;

/// Pre-computed peak data for one audio file.
#[derive(Debug, Clone)]
pub struct CachedPeaks {
    pub sample_rate: u32,
    pub samples_per_peak: u32,
    pub total_samples: u64,
    pub peaks: Vec<f32>,
}

impl CachedPeaks {
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.total_samples as f64 / self.sample_rate as f64
    }
}

/// In-memory cache so we don't re-read .peaks files from disk on every zoom/pan.
pub struct PeakCache {
    entries: RwLock<HashMap<PathBuf, CachedPeaks>>,
}

impl PeakCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, path: &Path) -> Option<CachedPeaks> {
        self.entries.read().ok()?.get(path).cloned()
    }

    pub fn insert(&self, path: PathBuf, peaks: CachedPeaks) {
        if let Ok(mut map) = self.entries.write() {
            map.insert(path, peaks);
        }
    }
}

/// Get source file size and mtime (seconds since epoch) for cache invalidation.
fn source_file_meta(path: &Path) -> Result<(u64, u64), String> {
    let meta = fs::metadata(path).map_err(|e| format!("Cannot stat {}: {}", path.display(), e))?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok((size, mtime))
}

/// Path to the sidecar cache file for a given audio file.
fn cache_path(audio_path: &Path) -> PathBuf {
    let mut p = audio_path.as_os_str().to_owned();
    p.push(".peaks");
    PathBuf::from(p)
}

/// Write a CachedPeaks to the binary .peaks sidecar file.
fn write_cache(
    audio_path: &Path,
    cached: &CachedPeaks,
) -> Result<(), String> {
    let (src_size, src_mtime) = source_file_meta(audio_path)?;
    let out_path = cache_path(audio_path);

    let file = File::create(&out_path)
        .map_err(|e| format!("Cannot create {}: {}", out_path.display(), e))?;
    let mut w = BufWriter::new(file);

    // Header
    w.write_all(MAGIC).map_err(|e| e.to_string())?;
    w.write_all(&[FORMAT_VERSION]).map_err(|e| e.to_string())?;
    w.write_all(&cached.samples_per_peak.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&cached.sample_rate.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&cached.total_samples.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&(cached.peaks.len() as u32).to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&src_size.to_le_bytes()).map_err(|e| e.to_string())?;
    w.write_all(&src_mtime.to_le_bytes()).map_err(|e| e.to_string())?;

    // Peak data
    for &peak in &cached.peaks {
        w.write_all(&peak.to_le_bytes()).map_err(|e| e.to_string())?;
    }

    w.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// Read and validate a .peaks sidecar cache file.
/// Returns None if the file is missing, corrupt, or stale.
fn read_cache(audio_path: &Path) -> Option<CachedPeaks> {
    let cp = cache_path(audio_path);
    let mut file = File::open(&cp).ok()?;

    let (src_size, src_mtime) = source_file_meta(audio_path).ok()?;

    // Read header
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != MAGIC {
        return None;
    }

    let mut version = [0u8; 1];
    file.read_exact(&mut version).ok()?;
    if version[0] != FORMAT_VERSION {
        return None;
    }

    let mut buf4 = [0u8; 4];
    let mut buf8 = [0u8; 8];

    file.read_exact(&mut buf4).ok()?;
    let samples_per_peak = u32::from_le_bytes(buf4);

    file.read_exact(&mut buf4).ok()?;
    let sample_rate = u32::from_le_bytes(buf4);

    file.read_exact(&mut buf8).ok()?;
    let total_samples = u64::from_le_bytes(buf8);

    file.read_exact(&mut buf4).ok()?;
    let total_peaks = u32::from_le_bytes(buf4) as usize;

    file.read_exact(&mut buf8).ok()?;
    let cached_src_size = u64::from_le_bytes(buf8);

    file.read_exact(&mut buf8).ok()?;
    let cached_src_mtime = u64::from_le_bytes(buf8);

    // Invalidation check
    if cached_src_size != src_size || cached_src_mtime != src_mtime {
        return None;
    }

    // Read peak data
    let mut peaks = Vec::with_capacity(total_peaks);
    for _ in 0..total_peaks {
        file.read_exact(&mut buf4).ok()?;
        peaks.push(f32::from_le_bytes(buf4));
    }

    Some(CachedPeaks {
        sample_rate,
        samples_per_peak,
        total_samples,
        peaks,
    })
}

/// Decode the entire audio file and generate the peak cache.
/// Writes the .peaks sidecar file and returns the CachedPeaks.
fn generate_cache(audio_path: &Path) -> Result<CachedPeaks, String> {
    let file = File::open(audio_path)
        .map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = audio_path.extension().and_then(|e| e.to_str()) {
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

    let mut peaks: Vec<f32> = Vec::new();
    let mut current_peak: f32 = 0.0;
    let mut samples_in_bucket: u32 = 0;
    let mut total_samples: u64 = 0;

    loop {
        let packet = match format_reader.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(format!("Error reading packet: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("Decode error: {}", e)),
        };

        let spec = *decoded.spec();
        let num_frames = decoded.frames();
        let num_channels = spec.channels.count();

        if num_frames == 0 || num_channels == 0 {
            continue;
        }

        let mut sample_buf = SampleBuffer::<f32>::new(num_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        for frame_idx in 0..num_frames {
            // Mix to mono
            let mut mono_sum: f32 = 0.0;
            for ch in 0..num_channels {
                mono_sum += samples[frame_idx * num_channels + ch];
            }
            let mono_val = (mono_sum / num_channels as f32).abs();

            if mono_val > current_peak {
                current_peak = mono_val;
            }

            samples_in_bucket += 1;
            total_samples += 1;

            if samples_in_bucket >= SAMPLES_PER_PEAK {
                peaks.push(current_peak);
                current_peak = 0.0;
                samples_in_bucket = 0;
            }
        }
    }

    // Flush the last partial bucket
    if samples_in_bucket > 0 {
        peaks.push(current_peak);
    }

    let cached = CachedPeaks {
        sample_rate,
        samples_per_peak: SAMPLES_PER_PEAK,
        total_samples,
        peaks,
    };

    // Write to disk (best-effort — if it fails we still return the data)
    if let Err(e) = write_cache(audio_path, &cached) {
        log::warn!("Failed to write peak cache for {}: {}", audio_path.display(), e);
    }

    Ok(cached)
}
