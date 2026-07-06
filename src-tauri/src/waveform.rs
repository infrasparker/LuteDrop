use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// How many raw audio samples are condensed into one peak value.
/// 256 is the Audacity/DAW standard — good balance of resolution vs file size.
const SAMPLES_PER_PEAK: u32 = 256;

/// Cap on files held in the in-memory peak cache. Peaks are a few hundred KB
/// per file, so this bounds the cache at ~10-20 MB while keeping every
/// recently-viewed waveform instant.
const MAX_CACHED_FILES: usize = 32;

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

struct LruEntry {
    peaks: CachedPeaks,
    last_used: u64,
}

/// In-memory cache so we don't re-read .peaks files from disk on every
/// zoom/pan — LRU-capped at MAX_CACHED_FILES. Also owns the `cache_dir`
/// where .peaks files are stored on disk, and the per-path generation locks
/// that serialize concurrent decodes of the same uncached file.
pub struct PeakCache {
    /// (entries, monotonic use counter for LRU ordering)
    entries: Mutex<(HashMap<PathBuf, LruEntry>, u64)>,
    /// Per-path locks handed to extract_peaks' miss path. Entries are tiny
    /// (an Arc<Mutex<()>>) and bounded by distinct files ever generated, so
    /// no eviction is needed here.
    generating: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    cache_dir: PathBuf,
}

impl PeakCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        fs::create_dir_all(&cache_dir).ok();
        Self {
            entries: Mutex::new((HashMap::new(), 0)),
            generating: Mutex::new(HashMap::new()),
            cache_dir,
        }
    }

    pub fn get(&self, path: &Path) -> Option<CachedPeaks> {
        let mut guard = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let (map, counter) = &mut *guard;
        *counter += 1;
        let entry = map.get_mut(path)?;
        entry.last_used = *counter;
        Some(entry.peaks.clone())
    }

    pub fn insert(&self, path: PathBuf, peaks: CachedPeaks) {
        let mut guard = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let (map, counter) = &mut *guard;
        *counter += 1;
        if map.len() >= MAX_CACHED_FILES && !map.contains_key(&path) {
            // Evict the least-recently-used entry to stay at the cap.
            if let Some(lru_key) = map
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            {
                map.remove(&lru_key);
            }
        }
        map.insert(path, LruEntry { peaks, last_used: *counter });
    }

    /// Evict a path from the in-memory cache and delete its .peaks file from disk.
    pub fn evict(&self, path: &Path) {
        {
            let mut guard = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            guard.0.remove(path);
        }
        let disk_path = cache_path(path, &self.cache_dir);
        let _ = fs::remove_file(disk_path);
    }

    /// Per-path generation lock: extract_peaks holds this across its disk-read
    /// + decode miss path so two concurrent requests for the same uncached
    /// file don't both run a full decode.
    fn generation_lock(&self, path: &Path) -> Arc<Mutex<()>> {
        let mut map = self.generating.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
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

    // Validate the claimed peak count against the actual file size before
    // allocating — a corrupt count could otherwise request gigabytes.
    // Header layout: magic(4) + version(1) + samples_per_peak(4) +
    // sample_rate(4) + total_samples(8) + total_peaks(4) + src_size(8) +
    // src_mtime(8) = 41 bytes, then 4 bytes per peak.
    const HEADER_LEN: u64 = 41;
    let file_len = file.metadata().ok()?.len();
    if HEADER_LEN + (total_peaks as u64) * 4 != file_len {
        return None;
    }

    // Read peak data in one bulk read (the old loop was one syscall per peak).
    let mut data = vec![0u8; total_peaks * 4];
    file.read_exact(&mut data).ok()?;
    let peaks: Vec<f32> = data
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

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
    // Stream-decode in ~32 KB chunks (any miniaudio format incl. .opus); the
    // old whole-file decode held a 2-hour stereo file as ~2.7 GB of f32 —
    // twice, counting the C→Rust copy — to produce a few hundred KB of peaks.
    let path_str = audio_path
        .to_str()
        .ok_or_else(|| format!("Non-UTF8 path: {}", audio_path.display()))?;
    let mut dec = crate::audio::ffi::StreamingDecoder::open(path_str)?;

    let channels = dec.channels.max(1) as usize;
    let sample_rate = dec.sample_rate;

    let mut acc = PeakAccumulator::new();
    while let Some(chunk) = dec.read_chunk()? {
        // Mix each frame to mono: average the per-channel samples.
        for frame in chunk.chunks_exact(channels) {
            let mono_sum: f32 = frame.iter().sum();
            acc.push(mono_sum / channels as f32);
        }
    }

    let (peaks, total_samples) = acc.finish();
    let cached = CachedPeaks {
        sample_rate,
        samples_per_peak: SAMPLES_PER_PEAK,
        total_samples,
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

    // Tiers 2/3 are expensive; serialize per-path so concurrent requests for
    // the same uncached file don't both hit disk / run a full decode.
    let gen_lock = peak_cache.generation_lock(audio_path);
    let _guard = gen_lock.lock().unwrap_or_else(|p| p.into_inner());

    // Re-check Tier 1: another thread may have populated it while we waited.
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

    #[test]
    fn read_cache_rejects_peak_count_inconsistent_with_file_size() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("a.mp3");
        std::fs::write(&audio, b"not really audio, just needs size/mtime").unwrap();
        let (src_size, src_mtime) = source_file_meta(&audio).unwrap();

        // Hand-craft a header claiming u32::MAX peaks but containing none.
        let cp = cache_path(&audio, dir.path());
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC);
        data.push(FORMAT_VERSION);
        data.extend_from_slice(&512u32.to_le_bytes());          // samples_per_peak
        data.extend_from_slice(&48000u32.to_le_bytes());        // sample_rate
        data.extend_from_slice(&1_000_000u64.to_le_bytes());    // total_samples
        data.extend_from_slice(&u32::MAX.to_le_bytes());        // total_peaks (lie)
        data.extend_from_slice(&src_size.to_le_bytes());
        data.extend_from_slice(&src_mtime.to_le_bytes());
        std::fs::write(&cp, &data).unwrap();

        assert!(read_cache(&audio, dir.path()).is_none());
    }

    #[test]
    fn opus_fixture_peaks_snapshot() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let dir = tempfile::tempdir().unwrap();
        let cached = generate_cache(&fixture, dir.path()).unwrap();
        // Values pinned from the pre-streaming (whole-file) decoder — the
        // streaming rewrite must reproduce them exactly.
        assert_eq!(cached.sample_rate, 48000 /* PIN */);
        assert_eq!(cached.total_samples, 5760648 /* PIN */);
        assert_eq!(cached.peaks.len(), 22503 /* PIN */);
        let sum: f32 = cached.peaks.iter().sum();
        assert!((sum - 8246.38 /* PIN */).abs() < 0.01, "peak sum {sum}");
        assert_eq!(cached.samples_per_peak, SAMPLES_PER_PEAK);
    }

    #[test]
    fn peak_cache_evicts_least_recently_used_beyond_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PeakCache::new(dir.path().to_path_buf());
        for i in 0..MAX_CACHED_FILES {
            cache.insert(PathBuf::from(format!("/f{i}.mp3")), make_test_peaks(vec![0.5]));
        }
        // Touch /f0 so it becomes most-recently-used…
        assert!(cache.get(Path::new("/f0.mp3")).is_some());
        // …then push past the cap: /f1 (now the LRU) must be evicted, /f0 kept.
        cache.insert(PathBuf::from("/overflow.mp3"), make_test_peaks(vec![0.5]));
        assert!(cache.get(Path::new("/f0.mp3")).is_some());
        assert!(cache.get(Path::new("/f1.mp3")).is_none());
        assert!(cache.get(Path::new("/overflow.mp3")).is_some());
    }

    #[test]
    fn generation_lock_is_shared_per_path() {
        let dir = tempfile::tempdir().unwrap();
        let cache = PeakCache::new(dir.path().to_path_buf());
        let a1 = cache.generation_lock(Path::new("/same.mp3"));
        let a2 = cache.generation_lock(Path::new("/same.mp3"));
        let b = cache.generation_lock(Path::new("/other.mp3"));
        assert!(std::sync::Arc::ptr_eq(&a1, &a2));
        assert!(!std::sync::Arc::ptr_eq(&a1, &b));
    }
}
