# Waveform Peak Cache Design

## Problem

The current waveform implementation uses seek-based sampling — it seeks to 1200 evenly-spaced positions in the audio file and decodes 8 packets at each. This has two problems:

1. **Inaccuracy**: Transients between sample points are missed. The waveform is a sparse approximation, not a true representation.
2. **Slow loading**: Every load (and every zoom) re-opens the file, re-probes, creates a decoder, and performs hundreds of seeks. Seeking in compressed formats like MP3 is expensive.

## Solution

Adopt the Audacity/DAW approach: one-time full decode of the audio file, storing pre-computed peaks in a compact binary sidecar cache file (`{audio_path}.peaks`). Subsequent loads read the cache file — instant. Zoom is served by re-bucketing cached data — no decoding at all.

## Binary Cache Format

```
Header (41 bytes, little-endian):
  magic:             [u8; 4]   — "LDPK"
  version:           u8        — 1
  samples_per_peak:  u32       — 256
  sample_rate:       u32       — from audio file
  num_channels:      u16       — from audio file
  total_samples:     u64       — total mono samples decoded
  total_peaks:       u32       — length of the peak data array
  source_file_size:  u64       — for invalidation
  source_file_mtime: u64       — seconds since epoch, for invalidation

Data:
  peaks: [f32; total_peaks]    — peak absolute values, little-endian
```

### Size examples

At 256 samples per peak, 44.1kHz:

| Duration | Peaks   | Cache size |
|----------|---------|------------|
| 3 min    | ~31K    | ~124 KB    |
| 10 min   | ~103K   | ~413 KB    |
| 1 hour   | ~620K   | ~2.4 MB    |

### Invalidation

On cache load, compare `source_file_size` and `source_file_mtime` from the header against the actual audio file. If either differs, regenerate the cache.

## Architecture

### Command flow

`get_waveform_data(filePath, width, start?, end?)` keeps its exact current signature. The frontend does not change.

```
get_waveform_data(filePath, width, start?, end?)
  │
  ├─ Check in-memory cache (HashMap<PathBuf, CachedPeaks>)
  │    └─ Hit → downsample and return
  │
  ├─ Try read "{filePath}.peaks" from disk
  │    ├─ Valid (magic + size/mtime match) → load into memory cache → downsample and return
  │    └─ Missing or stale → fall through
  │
  └─ Full decode → write .peaks file → store in memory cache → downsample and return
```

### Full decode (cache generation)

Single linear pass through the audio file using Symphonia. Decode every packet, mix to mono, accumulate the max absolute sample value per 256-sample bucket. Write the resulting peaks to the `.peaks` sidecar file.

This replaces the current `extract_peaks_seek` function. The existing `extract_peaks_full_decode` function is close to what's needed — it gets repurposed as the cache generator, modified to write peaks at the fixed 256-sample resolution rather than at `width` resolution.

### Downsampling (serving a request)

Given the full-resolution cached peaks and a request for `width` bins over an optional `[start, end]` time range:

1. Convert `start`/`end` seconds to peak indices using `sample_rate` and `samples_per_peak`
2. Slice the relevant portion of the cached peaks array
3. Divide into `width` equal buckets, take the max value in each bucket
4. Normalize to 0.0–1.0

This is pure array math — microseconds regardless of file length.

### In-memory cache

A `HashMap<PathBuf, CachedPeaks>` held in the Tauri app state (behind `Arc<RwLock<...>>`). Once a `.peaks` file is read from disk, the peak data stays in memory for the session. This avoids re-reading the disk cache on every zoom/pan interaction.

For typical usage (a handful of active audio files), this adds a few MB of RAM at most.

## What changes

### Rust side

- `waveform.rs`: Replace seek-based logic with cache-backed approach
  - Delete `extract_peaks_seek`, `seek_and_sample`, `PACKETS_PER_SAMPLE`
  - Repurpose `extract_peaks_full_decode` as the cache generator (fixed 256-sample resolution, writes `.peaks` file)
  - New: `read_cache` / `write_cache` functions for the binary format
  - New: `downsample_peaks` function for serving requests from cached data
  - New: `CachedPeaks` struct holding the peak data + metadata
- `state.rs` or `commands.rs`: Add in-memory peak cache (`HashMap`) to app state
- `commands.rs`: `get_waveform_data` passes the memory cache into `waveform::extract_peaks`

### Frontend side

No changes. `WaveformEditor.vue` continues calling `invoke('get_waveform_data', ...)` with the same signature. The "Loading waveform..." overlay already handles the async wait.

### What gets deleted

- `extract_peaks_seek` function (~100 lines)
- `seek_and_sample` function (~60 lines)  
- `PACKETS_PER_SAMPLE` constant

Net result should be simpler code than what exists today.

## Performance expectations

| Scenario | Current | After |
|----------|---------|-------|
| First load (3 min MP3) | ~0.5s (seek) | ~0.5s (full decode + write cache) |
| First load (1 hr MP3) | ~2-3s (seek) | ~3-5s (full decode + write cache) |
| Subsequent load (any size) | Same as first | <10ms (read cache) |
| Zoom/pan | ~0.5-2s (re-seek) | <1ms (rebucket array) |

First load of long files is slightly slower, but it only happens once per file ever. Every subsequent interaction is effectively instant.
