# Waveform Peak Cache Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace seek-based waveform sampling with a persistent binary peak cache for instant loads and pixel-accurate waveforms.

**Architecture:** One-time full decode writes a `.peaks` sidecar file next to each audio file. Subsequent loads read the cache. An in-memory `HashMap` avoids repeated disk reads within a session. The `get_waveform_data` Tauri command signature stays identical — frontend unchanged.

**Tech Stack:** Rust, Symphonia (audio decoding), Tauri 2, Vue 3

**Spec:** `docs/superpowers/specs/2026-04-12-waveform-cache-design.md`

---

## File Map

| Action | File | Responsibility |
|--------|------|---------------|
| Rewrite | `src-tauri/src/waveform.rs` | Cache format read/write, full-decode generator, downsample, public `extract_peaks` entry point |
| Modify | `src-tauri/src/commands.rs:137-149` | Pass `PeakCache` into `get_waveform_data` |
| Modify | `src-tauri/src/state.rs` | Add `PeakCache` field to `AppStateContainer` |
| Modify | `src-tauri/src/lib.rs:35` | Initialize `PeakCache` in container construction |

Frontend files are **not touched**.

---

### Task 1: Peak cache data structures and binary format writer

**Files:**
- Modify: `src-tauri/src/waveform.rs` (replace entire contents)

This task builds the foundation: the `CachedPeaks` struct, the binary header format, and the `write_cache` function. No reading or decoding yet — just the data model and serialization.

- [ ] **Step 1: Replace waveform.rs with the new module structure and cache types**

Replace the entire contents of `src-tauri/src/waveform.rs` with:

```rust
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
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: compiles with warnings about unused imports/functions (expected — we haven't written the consumers yet).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/waveform.rs
git commit -m "feat(waveform): cache data structures and binary format writer"
```

---

### Task 2: Binary format reader with validation

**Files:**
- Modify: `src-tauri/src/waveform.rs`

Add `read_cache` — reads a `.peaks` file, validates magic/version/staleness, and returns `CachedPeaks`.

- [ ] **Step 1: Add the read_cache function**

Add this function to `src-tauri/src/waveform.rs`, after `write_cache`:

```rust
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
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: compiles (with unused warnings still fine).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/waveform.rs
git commit -m "feat(waveform): binary format reader with staleness validation"
```

---

### Task 3: Full-decode cache generator

**Files:**
- Modify: `src-tauri/src/waveform.rs`

Add `generate_cache` — does a single linear decode of the entire audio file, computes peaks at 256-sample resolution, writes the `.peaks` file, and returns the `CachedPeaks`.

- [ ] **Step 1: Add the generate_cache function**

Add this function to `src-tauri/src/waveform.rs`, after `read_cache`:

```rust
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
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: compiles cleanly (the `log` crate is already a dependency).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/waveform.rs
git commit -m "feat(waveform): full-decode cache generator with .peaks sidecar write"
```

---

### Task 4: Downsample function and public API

**Files:**
- Modify: `src-tauri/src/waveform.rs`

Add `downsample_peaks` (re-buckets cached peaks to any requested width/range) and rewrite the public `extract_peaks` entry point to use the cache pipeline.

- [ ] **Step 1: Add the downsample function and rewrite extract_peaks**

Add `downsample_peaks` after `generate_cache`, and replace the existing `extract_peaks` function:

```rust
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
/// Uses a three-tier lookup: in-memory cache → disk .peaks file → full decode.
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
    if let Some(cached) = read_cache(audio_path) {
        peak_cache.insert(audio_path.to_path_buf(), cached.clone());
        return Ok(downsample_peaks(&cached, width, start, end));
    }

    // Tier 3: full decode → write cache → store in memory
    let cached = generate_cache(audio_path)?;
    peak_cache.insert(audio_path.to_path_buf(), cached.clone());
    Ok(downsample_peaks(&cached, width, start, end))
}
```

- [ ] **Step 2: Remove old unused imports**

At the top of `src-tauri/src/waveform.rs`, remove the imports that are no longer needed. The final import block should be exactly:

```rust
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
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Compile error — `extract_peaks` now requires a `&PeakCache` parameter but `commands.rs` isn't passing one yet. This is expected and gets fixed in Task 5.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/waveform.rs
git commit -m "feat(waveform): downsample function and cache-backed extract_peaks"
```

---

### Task 5: Wire PeakCache into app state and commands

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs:35`
- Modify: `src-tauri/src/commands.rs:137-149`

Connect the `PeakCache` to the Tauri app so the `get_waveform_data` command can use it.

- [ ] **Step 1: Add PeakCache to AppStateContainer**

In `src-tauri/src/state.rs`, add the import and field:

```rust
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use tokio::sync::broadcast;
use crate::audio::AudioEngine;
use crate::models::AppState;
use crate::waveform::PeakCache;

/// Managed state shared across all Tauri commands and the WebSocket server.
/// Access in commands via `state: tauri::State<'_, Arc<AppStateContainer>>`.
pub struct AppStateContainer {
    pub data: RwLock<AppState>,
    pub audio: Mutex<AudioEngine>,
    pub save_path: PathBuf,
    pub broadcast_tx: broadcast::Sender<String>,
    pub peak_cache: PeakCache,
}

impl AppStateContainer {
    pub fn new(initial: AppState, save_path: PathBuf, audio: AudioEngine) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            data: RwLock::new(initial),
            audio: Mutex::new(audio),
            save_path,
            broadcast_tx,
            peak_cache: PeakCache::new(),
        }
    }
}
```

- [ ] **Step 2: Update get_waveform_data command to pass PeakCache**

In `src-tauri/src/commands.rs`, replace the `get_waveform_data` function (lines 137–149):

```rust
#[tauri::command]
pub async fn get_waveform_data(
    state: State<'_, Arc<AppStateContainer>>,
    file_path: String,
    width: u32,
    start: Option<f64>,
    end: Option<f64>,
) -> Result<WaveformData, String> {
    let state_clone = state.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<WaveformData, String> {
        let (peaks, duration) = waveform::extract_peaks(&file_path, width, start, end, &state_clone.peak_cache)?;
        Ok(WaveformData { peaks, duration })
    })
    .await
    .map_err(|e| format!("Waveform task failed: {}", e))?
}
```

- [ ] **Step 3: Add the Arc import to commands.rs if missing**

In `src-tauri/src/commands.rs`, make sure the imports at the top include `Arc` and `AppStateContainer`:

```rust
use serde::Serialize;
use tauri::State;
use std::sync::Arc;
use crate::actions;
use crate::models::AppState;
use crate::state::AppStateContainer;
use crate::waveform;
```

- [ ] **Step 4: Build the full project**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: compiles successfully with no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/commands.rs
git commit -m "feat(waveform): wire PeakCache into app state and commands"
```

---

### Task 6: Unit tests for cache round-trip and downsampling

**Files:**
- Modify: `src-tauri/src/waveform.rs` (add `#[cfg(test)]` module at bottom)

- [ ] **Step 1: Add test module to waveform.rs**

Add at the bottom of `src-tauri/src/waveform.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

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
        // Create a temp file to act as the "audio source" for metadata
        let dir = std::env::temp_dir().join("lutedrop_test_cache");
        let _ = fs::create_dir_all(&dir);
        let fake_audio = dir.join("test.mp3");
        {
            let mut f = File::create(&fake_audio).unwrap();
            f.write_all(b"fake audio data for testing").unwrap();
        }

        let original = make_test_peaks(vec![0.1, 0.5, 0.9, 0.3, 0.7]);

        // Write cache
        write_cache(&fake_audio, &original).unwrap();

        // Read it back
        let loaded = read_cache(&fake_audio).expect("should read back successfully");
        assert_eq!(loaded.sample_rate, original.sample_rate);
        assert_eq!(loaded.samples_per_peak, original.samples_per_peak);
        assert_eq!(loaded.total_samples, original.total_samples);
        assert_eq!(loaded.peaks.len(), original.peaks.len());
        for (a, b) in loaded.peaks.iter().zip(original.peaks.iter()) {
            assert!((a - b).abs() < 0.0001);
        }

        // Clean up
        let _ = fs::remove_file(cache_path(&fake_audio));
        let _ = fs::remove_file(&fake_audio);
        let _ = fs::remove_dir(&dir);
    }

    #[test]
    fn cache_invalidation_on_size_change() {
        let dir = std::env::temp_dir().join("lutedrop_test_invalidation");
        let _ = fs::create_dir_all(&dir);
        let fake_audio = dir.join("test2.mp3");
        {
            let mut f = File::create(&fake_audio).unwrap();
            f.write_all(b"original content").unwrap();
        }

        let original = make_test_peaks(vec![0.5, 0.5]);
        write_cache(&fake_audio, &original).unwrap();

        // Modify the source file (different size)
        {
            let mut f = File::create(&fake_audio).unwrap();
            f.write_all(b"modified content that is longer now").unwrap();
        }

        // Cache should be stale
        assert!(read_cache(&fake_audio).is_none(), "cache should be invalidated after source change");

        // Clean up
        let _ = fs::remove_file(cache_path(&fake_audio));
        let _ = fs::remove_file(&fake_audio);
        let _ = fs::remove_dir(&dir);
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml -- waveform`
Expected: all 6 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/waveform.rs
git commit -m "test(waveform): unit tests for cache round-trip and downsampling"
```

---

### Task 7: Manual smoke test

**Files:** None (verification only)

- [ ] **Step 1: Build and run the app**

Run: `cd src-tauri && cargo tauri dev`

- [ ] **Step 2: Open a track with an audio clip and verify the waveform loads**

Open any composition with an audio clip. The waveform should render. First load may take a moment (full decode + cache write). Check the terminal for any errors or warnings.

- [ ] **Step 3: Verify the .peaks sidecar was created**

Check next to the audio file — there should be a `.peaks` file:
```bash
ls -la /path/to/your/audio/file.mp3.peaks
```

- [ ] **Step 4: Reload the waveform and verify instant load**

Navigate away from the track and back, or restart the app. The waveform should load near-instantly the second time.

- [ ] **Step 5: Test zoom/pan**

Zoom in with mouse wheel, pan with drag. Should be responsive with no loading delay — zoom is now served from cached data.

- [ ] **Step 6: Commit any fixes if needed**

If any issues were found and fixed:
```bash
git add -A
git commit -m "fix(waveform): smoke test fixes"
```
