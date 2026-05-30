use std::os::raw::{c_float, c_int};
use std::os::windows::ffi::OsStrExt;

/// Encode a path as a NUL-terminated UTF-16 buffer for miniaudio's wide (`_w`)
/// file APIs. On Windows `wchar_t` is 16-bit (matches `u16`), and the wide APIs
/// open paths via `_wfopen`, which handles full Unicode — unlike the narrow
/// APIs that misinterpret UTF-8 bytes through the ANSI/locale codepage. This is
/// the fix for non-ASCII audio paths (smart quotes, accents, CJK) failing to
/// decode/play. Windows is the only target, so this is unconditionally Windows.
fn to_wide(path: &str) -> Vec<u16> {
    std::ffi::OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

extern "C" {
    fn ld_decode_file(
        path: *const u16,
        out: *mut *mut c_float,
        frames: *mut u64,
        channels: *mut u32,
        sample_rate: *mut u32,
    ) -> c_int;
    fn ld_free_decoded(p: *mut c_float);
    fn ld_file_duration_secs(path: *const u16) -> f64;

    fn ld_engine_create() -> *mut LdEngine;
    fn ld_engine_destroy(e: *mut LdEngine);
    fn ld_engine_set_volume(e: *mut LdEngine, linear: c_float);
    fn ld_engine_time_frames(e: *mut LdEngine) -> u64;
    fn ld_engine_sample_rate(e: *mut LdEngine) -> u32;

    fn ld_group_create(e: *mut LdEngine) -> *mut LdGroup;
    fn ld_group_set_volume(g: *mut LdGroup, linear: c_float);
    fn ld_group_destroy(g: *mut LdGroup);

    fn ld_sound_create(
        e: *mut LdEngine,
        g: *mut LdGroup,
        path: *const u16,
        stream: c_int,
    ) -> *mut LdSound;
    fn ld_sound_play(s: *mut LdSound, start_secs: f64, fade_in_secs: f64) -> c_int;
    fn ld_sound_stop(s: *mut LdSound, fade_out_secs: f64);
    fn ld_sound_pause(s: *mut LdSound);
    fn ld_sound_resume(s: *mut LdSound);
    fn ld_sound_set_volume(s: *mut LdSound, linear: c_float);
    fn ld_sound_position_secs(s: *mut LdSound) -> f64;
    fn ld_sound_length_secs(s: *mut LdSound) -> f64;
    fn ld_sound_is_playing(s: *mut LdSound) -> c_int;
    fn ld_sound_at_end(s: *mut LdSound) -> c_int;
    fn ld_sound_schedule_start_frame(s: *mut LdSound, engine_frame: u64);
    fn ld_sound_destroy(s: *mut LdSound);
}

// Opaque C types. We never construct or dereference these in Rust; the shim
// owns the underlying miniaudio structs and only hands back pointers.
#[repr(C)]
pub struct LdEngine {
    _private: [u8; 0],
}
#[repr(C)]
pub struct LdGroup {
    _private: [u8; 0],
}
#[repr(C)]
pub struct LdSound {
    _private: [u8; 0],
}

/// Safe wrapper over `ma_engine` (+ its libopus-aware resource manager).
///
/// # Drop order
/// The audio object graph must be torn down innermost-first:
/// **`Sound`s, then `Group`s, then `Engine`**. miniaudio sounds/groups reference
/// the engine, so destroying the engine while sounds/groups are still alive is
/// undefined behavior. Callers that hold these in one struct must order their
/// fields so Rust drops sounds before groups before the engine (Rust drops
/// struct fields in declaration order), or drop them explicitly in that order.
pub struct Engine(*mut LdEngine);

/// Safe wrapper over `ma_sound_group` (a per-track / library sub-mix).
/// Must be dropped before the owning `Engine` (see [`Engine`] drop-order note).
pub struct Group {
    ptr: *mut LdGroup,
}

/// Safe wrapper over `ma_sound` (one playing/loaded clip).
/// Must be dropped before its `Group` and `Engine` (see [`Engine`] drop-order note).
pub struct Sound {
    ptr: *mut LdSound,
}

// SAFETY: The raw miniaudio pointers are not themselves thread-safe, but in
// LuteDrop every `Engine`/`Group`/`Sound` lives behind the `AudioEngine`
// `Mutex`, so all access is serialized to a single thread at a time. We never
// share these across threads without that lock.
unsafe impl Send for Engine {}
unsafe impl Send for Group {}
unsafe impl Send for Sound {}

impl Engine {
    /// Create the audio engine. Fails if no usable audio backend can init.
    pub fn new() -> Result<Engine, String> {
        let ptr = unsafe { ld_engine_create() };
        if ptr.is_null() {
            return Err("failed to create audio engine".to_string());
        }
        Ok(Engine(ptr))
    }

    /// Set master (engine) volume. Linear, typically 0.0..=1.0.
    pub fn set_master_volume(&self, linear: f32) {
        unsafe { ld_engine_set_volume(self.0, linear) }
    }

    /// Current global engine clock in PCM frames (for sample-accurate scheduling).
    pub fn time_frames(&self) -> u64 {
        unsafe { ld_engine_time_frames(self.0) }
    }

    /// Engine output sample rate (Hz).
    pub fn sample_rate(&self) -> u32 {
        unsafe { ld_engine_sample_rate(self.0) }
    }

    /// Create a sub-mix group attached to this engine.
    pub fn create_group(&self) -> Result<Group, String> {
        let ptr = unsafe { ld_group_create(self.0) };
        if ptr.is_null() {
            return Err("failed to create audio group".to_string());
        }
        Ok(Group { ptr })
    }

    /// Create a sound from `path` into `group` (or the engine endpoint if
    /// `group` is `None`). `stream` selects streaming vs fully-loaded.
    pub fn create_sound(
        &self,
        group: Option<&Group>,
        path: &str,
        stream: bool,
    ) -> Result<Sound, String> {
        let wide = to_wide(path);
        let gptr = group.map_or(std::ptr::null_mut(), |g| g.ptr);
        let ptr =
            unsafe { ld_sound_create(self.0, gptr, wide.as_ptr(), if stream { 1 } else { 0 }) };
        if ptr.is_null() {
            return Err(format!("failed to create sound for {path}"));
        }
        Ok(Sound { ptr })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { ld_engine_destroy(self.0) }
    }
}

impl Group {
    /// Set this group's volume (linear). Applies to all sounds routed through it.
    pub fn set_volume(&self, linear: f32) {
        unsafe { ld_group_set_volume(self.ptr, linear) }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        unsafe { ld_group_destroy(self.ptr) }
    }
}

impl Sound {
    /// Seek to `start_secs`, optionally fade in over `fade_in_secs`, then start.
    pub fn play(&self, start_secs: f64, fade_in_secs: f64) -> Result<(), String> {
        let rc = unsafe { ld_sound_play(self.ptr, start_secs, fade_in_secs) };
        if rc != 0 {
            return Err(format!("failed to play sound (code {rc})"));
        }
        Ok(())
    }

    /// Stop playback. With `fade_out_secs > 0`, fades out first. Cursor is kept.
    pub fn stop(&self, fade_out_secs: f64) {
        unsafe { ld_sound_stop(self.ptr, fade_out_secs) }
    }

    /// Pause (= stop without fade; cursor preserved).
    pub fn pause(&self) {
        unsafe { ld_sound_pause(self.ptr) }
    }

    /// Resume from the preserved cursor.
    pub fn resume(&self) {
        unsafe { ld_sound_resume(self.ptr) }
    }

    /// Set this sound's volume (linear).
    pub fn set_volume(&self, linear: f32) {
        unsafe { ld_sound_set_volume(self.ptr, linear) }
    }

    /// Current playback cursor in seconds.
    pub fn position_secs(&self) -> f64 {
        unsafe { ld_sound_position_secs(self.ptr) }
    }

    /// Total length in seconds (0.0 if unknown).
    pub fn length_secs(&self) -> f64 {
        unsafe { ld_sound_length_secs(self.ptr) }
    }

    /// Whether the sound is currently playing.
    pub fn is_playing(&self) -> bool {
        unsafe { ld_sound_is_playing(self.ptr) != 0 }
    }

    /// Whether playback has reached the end.
    pub fn at_end(&self) -> bool {
        unsafe { ld_sound_at_end(self.ptr) != 0 }
    }

    /// Schedule the sound to start at an absolute engine frame (sample-accurate).
    pub fn schedule_start_frame(&self, engine_frame: u64) {
        unsafe { ld_sound_schedule_start_frame(self.ptr, engine_frame) }
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        unsafe { ld_sound_destroy(self.ptr) }
    }
}

pub struct Decoded {
    pub samples: Vec<f32>,
    pub frames: u64,
    pub channels: u32,
    pub sample_rate: u32,
}

pub fn decode_file(path: &str) -> Result<Decoded, String> {
    let wide = to_wide(path);
    let (mut out, mut frames, mut channels, mut sr) =
        (std::ptr::null_mut(), 0u64, 0u32, 0u32);
    let rc = unsafe { ld_decode_file(wide.as_ptr(), &mut out, &mut frames, &mut channels, &mut sr) };
    if rc != 0 || out.is_null() {
        return Err(format!("decode failed for {path}"));
    }
    let n = (frames as usize) * (channels as usize);
    let samples = unsafe { std::slice::from_raw_parts(out, n) }.to_vec();
    unsafe { ld_free_decoded(out) };
    Ok(Decoded {
        samples,
        frames,
        channels,
        sample_rate: sr,
    })
}

/// Cheap duration probe via the shim: opens a decoder, reads the PCM frame
/// count, divides by the sample rate (no full decode). Returns `None` if the
/// file can't be opened or its length is unknown.
pub fn file_duration_secs(path: &str) -> Option<f64> {
    let wide = to_wide(path);
    let secs = unsafe { ld_file_duration_secs(wide.as_ptr()) };
    if secs > 0.0 {
        Some(secs)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn decodes_opus_fixture() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let d = decode_file(p.to_str().unwrap()).expect("decode");
        assert_eq!(d.sample_rate, 48000);
        assert_eq!(d.channels, 2);
        // ~120s fixture at 48k; allow one packet of slack.
        assert!((d.frames as i64 - 5_760_000).abs() < 10_000, "frames={}", d.frames);
        let peak = d.samples.iter().fold(0f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.01, "silent (peak {peak})");
    }

    #[test]
    fn decodes_non_ascii_path() {
        // Copy the fixture to a temp path with non-ASCII chars (smart quote,
        // accents, CJK) and decode it. This proves the wide (UTF-16) file-open
        // fix without needing the real library: the old narrow CString path
        // failed to open such paths on Windows.
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let dst = std::env::temp_dir().join("sämple_é’カ_ld_test.opus");
        let _ = std::fs::remove_file(&dst); // best-effort clean from a prior run
        std::fs::copy(&src, &dst).expect("copy fixture to unicode temp path");

        let result = decode_file(dst.to_str().unwrap());

        // Clean up before asserting so a failure doesn't leak the temp file.
        let _ = std::fs::remove_file(&dst);

        let d = result.expect("decode of non-ASCII path");
        assert!(d.frames > 0, "decoded zero frames from unicode path");
        assert_eq!(d.sample_rate, 48000);
    }

    #[test]
    fn engine_creates_and_loads_fixture() {
        let eng = Engine::new().expect("engine");
        let g = eng.create_group().expect("group");
        let p =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let s = eng
            .create_sound(Some(&g), p.to_str().unwrap(), true)
            .expect("sound");
        assert!(s.position_secs() >= 0.0);
        // ~120s fixture; streaming length should be reported via the decoder.
        let len = s.length_secs();
        assert!((len - 120.0).abs() < 2.0, "length_secs={len}");
        // Drop order: sound, then group, then engine (enforced here by scope end:
        // s drops first, then g, then eng — innermost-first as miniaudio requires).
    }
}
