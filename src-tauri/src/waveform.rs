use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::SystemTime;

/// How many raw audio samples are condensed into one peak value.
/// 256 is the Audacity/DAW standard — good balance of resolution vs file size.
const SAMPLES_PER_PEAK: u32 = 256;

/// Magic bytes at the start of every .peaks file.
const MAGIC: &[u8; 4] = b"LDPK";

/// Cache format version. Bump this if the binary layout changes.
const FORMAT_VERSION: u8 = 1;

/// Accumulates absolute mono sample values into peak buckets of
/// `SAMPLES_PER_PEAK` samples each.
struct PeakAccumulator {
    peaks: Vec<f32>,
    current_peak: f32,
    samples_in_bucket: u32,
    total_samples: u64,
}

impl PeakAccumulator {
    fn new() -> Self {
        Self { peaks: Vec::new(), current_peak: 0.0, samples_in_bucket: 0, total_samples: 0 }
    }

    /// Push one mono sample (absolute value taken internally).
    fn push(&mut self, sample: f32) {
        let v = sample.abs();
        if v > self.current_peak {
            self.current_peak = v;
        }
        self.samples_in_bucket += 1;
        self.total_samples += 1;
        if self.samples_in_bucket >= SAMPLES_PER_PEAK {
            self.peaks.push(self.current_peak);
            self.current_peak = 0.0;
            self.samples_in_bucket = 0;
        }
    }

    /// Flush the final partial bucket and return (peaks, total_samples).
    fn finish(mut self) -> (Vec<f32>, u64) {
        if self.samples_in_bucket > 0 {
            self.peaks.push(self.current_peak);
        }
        (self.peaks, self.total_samples)
    }
}

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
/// Also owns the `cache_dir` where .peaks files are stored on disk.
pub struct PeakCache {
    entries: RwLock<HashMap<PathBuf, CachedPeaks>>,
    cache_dir: PathBuf,
}

impl PeakCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        fs::create_dir_all(&cache_dir).ok();
        Self {
            entries: RwLock::new(HashMap::new()),
            cache_dir,
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

    /// Evict a path from the in-memory cache and delete its .peaks file from disk.
    pub fn evict(&self, path: &Path) {
        if let Ok(mut map) = self.entries.write() {
            map.remove(path);
        }
        let disk_path = cache_path(path, &self.cache_dir);
        let _ = fs::remove_file(disk_path);
    }

    /// Deletes any on-disk .peaks file not referenced by a current source path.
    /// Run at startup to reclaim space from deleted/relocated library entries.
    pub fn gc(&self, valid_paths: &[String]) {
        let expected: HashSet<PathBuf> = valid_paths
            .iter()
            .map(|p| cache_path(Path::new(p), &self.cache_dir))
            .collect();
        if let Ok(entries) = fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("peaks")
                    && !expected.contains(&path)
                {
                    let _ = fs::remove_file(path);
                }
            }
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

/// Path to the cache file for a given audio file, inside the app's cache directory.
/// Uses a hash of the absolute audio path as the filename.
fn cache_path(audio_path: &Path, cache_dir: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    audio_path.hash(&mut hasher);
    let hash = hasher.finish();
    cache_dir.join(format!("{:016x}.peaks", hash))
}

/// Write a CachedPeaks to the binary .peaks cache file.
fn write_cache(
    audio_path: &Path,
    cached: &CachedPeaks,
    cache_dir: &Path,
) -> Result<(), String> {
    let (src_size, src_mtime) = source_file_meta(audio_path)?;
    let out_path = cache_path(audio_path, cache_dir);

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

/// Read and validate a .peaks cache file.
/// Returns None if the file is missing, corrupt, or stale.
fn read_cache(audio_path: &Path, cache_dir: &Path) -> Option<CachedPeaks> {
    let cp = cache_path(audio_path, cache_dir);
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
/// Writes the .peaks file to cache_dir and returns the CachedPeaks.
fn generate_cache(audio_path: &Path, cache_dir: &Path) -> Result<CachedPeaks, String> {
    // Decode the whole file (any miniaudio-supported format incl. .opus) to
    // interleaved f32 via the shim. The .peaks cache stays keyed by the original path.
    let path_str = audio_path
        .to_str()
        .ok_or_else(|| format!("Non-UTF8 path: {}", audio_path.display()))?;
    let decoded = crate::audio::ffi::decode_file(path_str)?;

    let channels = decoded.channels.max(1) as usize;
    let sample_rate = decoded.sample_rate;

    let mut acc = PeakAccumulator::new();

    // Mix each frame to mono: average the per-channel samples.
    for frame in decoded.samples.chunks_exact(channels) {
        let mono_sum: f32 = frame.iter().sum();
        acc.push(mono_sum / channels as f32);
    }

    let (peaks, _acc_total) = acc.finish();
    let cached = CachedPeaks {
        sample_rate,
        samples_per_peak: SAMPLES_PER_PEAK,
        total_samples: decoded.frames,
        peaks,
    };

    // Write to disk (best-effort — if it fails we still return the data)
    if let Err(e) = write_cache(audio_path, &cached, cache_dir) {
        log::warn!("Failed to write peak cache for {}: {}", audio_path.display(), e);
    }

    Ok(cached)
}

/// Downsample cached peaks to the requested width and optional time range.
/// Returns (downsampled_peaks, duration_seconds).
fn downsample_peaks(
    cached: &CachedPeaks,
    width: usize,
    start: Option<f64>,
    end: Option<f64>,
) -> (Vec<f32>, f64) {
    let duration = cached.duration_seconds();
    if cached.peaks.is_empty() || duration <= 0.0 {
        return (vec![0.0; width], duration);
    }

    let range_start = start.unwrap_or(0.0).max(0.0);
    let range_end = end.unwrap_or(duration).min(duration);

    if range_end <= range_start {
        return (vec![0.0; width], duration);
    }

    // Convert time range to peak indices
    let peaks_per_second = cached.sample_rate as f64 / cached.samples_per_peak as f64;
    let start_idx = (range_start * peaks_per_second) as usize;
    let end_idx = ((range_end * peaks_per_second).ceil() as usize).min(cached.peaks.len());

    if end_idx <= start_idx {
        return (vec![0.0; width], duration);
    }

    let slice = &cached.peaks[start_idx..end_idx];
    let slice_len = slice.len();

    let mut result = Vec::with_capacity(width);
    for i in 0..width {
        let bucket_start = i * slice_len / width;
        let bucket_end = ((i + 1) * slice_len / width).max(bucket_start + 1).min(slice_len);

        let mut max_val: f32 = 0.0;
        for &v in &slice[bucket_start..bucket_end] {
            if v > max_val {
                max_val = v;
            }
        }
        result.push(max_val);
    }

    // Normalize to 0.0–1.0
    let global_max = result.iter().cloned().fold(0.0_f32, f32::max);
    if global_max > 0.0 {
        for v in &mut result {
            *v /= global_max;
        }
    }

    (result, duration)
}

/// Public entry point. Returns (peaks, duration_seconds).
///
/// Uses a three-tier lookup: in-memory cache -> disk .peaks file -> full decode.
/// The `peak_cache` parameter is the shared in-memory cache from app state.
pub fn extract_peaks(
    file_path: &str,
    width: u32,
    start: Option<f64>,
    end: Option<f64>,
    peak_cache: &PeakCache,
) -> Result<(Vec<f32>, f64), String> {
    let width = width.max(1) as usize;
    let audio_path = Path::new(file_path);

    // Tier 1: in-memory cache
    if let Some(cached) = peak_cache.get(audio_path) {
        return Ok(downsample_peaks(&cached, width, start, end));
    }

    // Tier 2: disk cache
    if let Some(cached) = read_cache(audio_path, &peak_cache.cache_dir) {
        peak_cache.insert(audio_path.to_path_buf(), cached.clone());
        return Ok(downsample_peaks(&cached, width, start, end));
    }

    // Tier 3: full decode -> write cache -> store in memory
    let cached = generate_cache(audio_path, &peak_cache.cache_dir)?;
    peak_cache.insert(audio_path.to_path_buf(), cached.clone());
    Ok(downsample_peaks(&cached, width, start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a CachedPeaks with known values for testing.
    fn make_test_peaks(peaks: Vec<f32>) -> CachedPeaks {
        CachedPeaks {
            sample_rate: 44100,
            samples_per_peak: SAMPLES_PER_PEAK,
            total_samples: peaks.len() as u64 * SAMPLES_PER_PEAK as u64,
            peaks,
        }
    }

    #[test]
    fn downsample_identity_when_width_equals_len() {
        let cached = make_test_peaks(vec![0.2, 0.8, 0.4, 0.6]);
        let (result, dur) = downsample_peaks(&cached, 4, None, None);
        assert_eq!(result.len(), 4);
        // After normalization, max (0.8) becomes 1.0
        assert!((result[1] - 1.0).abs() < 0.001);
        assert!(dur > 0.0);
    }

    #[test]
    fn downsample_reduces_width() {
        // 8 peaks downsampled to 2 bins: each bin takes max of 4 peaks
        let cached = make_test_peaks(vec![0.1, 0.3, 0.2, 0.5, 0.4, 0.9, 0.1, 0.2]);
        let (result, _) = downsample_peaks(&cached, 2, None, None);
        assert_eq!(result.len(), 2);
        // Bin 0: max(0.1,0.3,0.2,0.5) = 0.5 → 0.5/0.9 ≈ 0.556
        // Bin 1: max(0.4,0.9,0.1,0.2) = 0.9 → 0.9/0.9 = 1.0
        assert!((result[1] - 1.0).abs() < 0.001);
        assert!(result[0] < result[1]);
    }

    #[test]
    fn downsample_with_time_range() {
        // 100 peaks at 44100 Hz, 256 samples/peak → each peak ≈ 5.8ms
        let mut peaks_data = vec![0.1_f32; 100];
        peaks_data[50] = 0.9; // spike in the middle
        let cached = make_test_peaks(peaks_data);

        let dur = cached.duration_seconds();
        let mid = dur / 2.0;

        // Request only the second half — should contain the spike
        let (result, _) = downsample_peaks(&cached, 10, Some(mid), None);
        let max_val = result.iter().cloned().fold(0.0_f32, f32::max);
        assert!((max_val - 1.0).abs() < 0.001, "spike should be in the second half");

        // Request only the first quarter — should NOT contain the spike
        let (result2, _) = downsample_peaks(&cached, 10, None, Some(dur * 0.25));
        // All values are 0.1 → normalized, all become 1.0 (they're all equal)
        // That's fine — the point is the spike isn't here
        assert_eq!(result2.len(), 10);
    }

    #[test]
    fn downsample_empty_peaks() {
        let cached = make_test_peaks(vec![]);
        let (result, dur) = downsample_peaks(&cached, 100, None, None);
        assert_eq!(result.len(), 100);
        assert!(result.iter().all(|&v| v == 0.0));
        assert_eq!(dur, 0.0);
    }

    #[test]
    fn cache_round_trip() {
        let dir = std::env::temp_dir().join("lutedrop_test_cache");
        let _ = fs::create_dir_all(&dir);
        let cache_dir = dir.join("peaks");
        let _ = fs::create_dir_all(&cache_dir);
        let fake_audio = dir.join("test.mp3");
        {
            let mut f = File::create(&fake_audio).unwrap();
            f.write_all(b"fake audio data for testing").unwrap();
        }

        let original = make_test_peaks(vec![0.1, 0.5, 0.9, 0.3, 0.7]);

        // Write cache
        write_cache(&fake_audio, &original, &cache_dir).unwrap();

        // Read it back
        let loaded = read_cache(&fake_audio, &cache_dir).expect("should read back successfully");
        assert_eq!(loaded.sample_rate, original.sample_rate);
        assert_eq!(loaded.samples_per_peak, original.samples_per_peak);
        assert_eq!(loaded.total_samples, original.total_samples);
        assert_eq!(loaded.peaks.len(), original.peaks.len());
        for (a, b) in loaded.peaks.iter().zip(original.peaks.iter()) {
            assert!((a - b).abs() < 0.0001);
        }

        // Clean up
        let _ = fs::remove_file(cache_path(&fake_audio, &cache_dir));
        let _ = fs::remove_file(&fake_audio);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn peak_accumulator_buckets_by_samples_per_peak() {
        let mut acc = PeakAccumulator::new();
        for _ in 0..SAMPLES_PER_PEAK { acc.push(0.5); }
        for _ in 0..SAMPLES_PER_PEAK { acc.push(0.9); }
        let (peaks, total) = acc.finish();
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 0.5).abs() < 1e-6);
        assert!((peaks[1] - 0.9).abs() < 1e-6);
        assert_eq!(total, (2 * SAMPLES_PER_PEAK) as u64);
    }

    #[test]
    fn cache_invalidation_on_size_change() {
        let dir = std::env::temp_dir().join("lutedrop_test_invalidation");
        let _ = fs::create_dir_all(&dir);
        let cache_dir = dir.join("peaks");
        let _ = fs::create_dir_all(&cache_dir);
        let fake_audio = dir.join("test2.mp3");
        {
            let mut f = File::create(&fake_audio).unwrap();
            f.write_all(b"original content").unwrap();
        }

        let original = make_test_peaks(vec![0.5, 0.5]);
        write_cache(&fake_audio, &original, &cache_dir).unwrap();

        // Modify the source file (different size)
        {
            let mut f = File::create(&fake_audio).unwrap();
            f.write_all(b"modified content that is longer now").unwrap();
        }

        // Cache should be stale
        assert!(read_cache(&fake_audio, &cache_dir).is_none(), "cache should be invalidated after source change");

        // Clean up
        let _ = fs::remove_dir_all(&dir);
    }
}
