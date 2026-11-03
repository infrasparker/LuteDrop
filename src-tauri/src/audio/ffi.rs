//! # FFI contract with the C shim (`vendor/shim.c`)
//!
//! This module is the sole Rust boundary to the vendored miniaudio shim. Four
//! conventions govern every call across it:
//!
//! **Ownership.** Every `ld_*_create`/`ld_decoder_open` call is paired with
//! exactly one `ld_*_destroy`/`ld_decoder_close` call, and each pairing is
//! wrapped in a `Drop` impl in this module (`Engine`, `Group`, `Sound`,
//! `StreamingDecoder`, `Encoder`). Rust code elsewhere never calls the raw `ld_*_destroy`
//! functions directly.
//!
//! **Drop order.** The shim requires sounds to be destroyed before their
//! group, and groups before their engine (miniaudio sounds/groups hold a raw
//! pointer back to the engine; destroying it first is undefined behavior).
//! This is enforced by FIELD DECLARATION ORDER — Rust drops struct fields
//! top-to-bottom — in `Voice` (`current`/`fading_out` before `group`),
//! `AudioEngine` (`mod.rs`: `tracks`/`library`, and `broadcaster` (whose
//! encoder thread owns an `Encoder` holding the engine pointer) before
//! `engine`), and
//! `ManagedTrack` (`orchestrator.rs`: `transitions` before `voice`).
//! An `Encoder` made by `Engine::create_encoder` must likewise be dropped
//! before its `Engine`: it reads the engine's tap through a raw pointer.
//! Reordering any of those fields silently reintroduces the UB; there is no
//! compile-time check.
//!
//! **Threading.** Every `ld_engine_*`/`ld_group_*`/`ld_sound_*` call happens
//! while the caller holds `AppStateContainer.audio: Mutex<AudioEngine>` —
//! that serialization is the entire basis for the `unsafe impl Send` below on
//! `Engine`/`Group`/`Sound` (miniaudio's objects are not internally
//! thread-safe). The single exception is `EngineRef` (below), a `Send + Sync`
//! engine-pointer copy used ONLY by the loader thread (`audio/loader.rs`) to
//! create and pre-seek group-less sounds off the mutex; its safety rests on
//! miniaudio's internal spinlock synchronization of node init/attach/uninit
//! (miniaudio.h §7.2) and on the engine outliving the loader thread (the
//! `AudioEngine.loader` field is joined on drop, before the `engine` field).
//! A group-less, never-started preloaded `Sound` may therefore be created,
//! seeked, and dropped on the loader thread. `ld_decoder_*` (`StreamingDecoder`)
//! is an unrelated handle that never touches the engine or that mutex, and is
//! safe on any single thread as long as one `StreamingDecoder` isn't shared
//! across threads.
//!
//! **Error conventions.** `ld_*_create`/`ld_decoder_open` return NULL on
//! failure. `ld_sound_play` returns a non-zero `c_int` on failure (see
//! `Sound::play`). `ld_decoder_read` returns frames read (`>0`), `0` for
//! end-of-stream, or `-1` for a hard mid-stream decode error — `0` is NOT an
//! error and must not be treated as one.

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
    fn ld_decoder_open(path: *const u16, out_channels: *mut u32, out_sample_rate: *mut u32) -> *mut LdDecoder;
    fn ld_decoder_read(d: *mut LdDecoder, out: *mut f32, max_frames: u64) -> i64;
    fn ld_decoder_close(d: *mut LdDecoder);
    fn ld_file_duration_secs(path: *const u16) -> f64;

    fn ld_engine_create() -> *mut LdEngine;
    fn ld_engine_destroy(e: *mut LdEngine);
    fn ld_engine_set_volume(e: *mut LdEngine, linear: c_float);
    fn ld_engine_time_frames(e: *mut LdEngine) -> u64;
    fn ld_engine_sample_rate(e: *mut LdEngine) -> u32;
    fn ld_engine_channels(e: *mut LdEngine) -> u32;
    fn ld_tap_set_enabled(e: *mut LdEngine, enabled: c_int);
    fn ld_tap_read(e: *mut LdEngine, out: *mut f32, max_frames: u64) -> i64;

    fn ld_group_create(e: *mut LdEngine, on_program_bus: c_int) -> *mut LdGroup;
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
    fn ld_sound_schedule_stop_secs(s: *mut LdSound, secs_from_now: f64, fade_secs: f64);
    fn ld_sound_attach_to_group(s: *mut LdSound, g: *mut LdGroup);
    fn ld_sound_seek_secs(s: *mut LdSound, secs: f64);
    fn ld_sound_destroy(s: *mut LdSound);

    fn ld_encoder_create(e: *mut LdEngine, bitrate_bps: u32) -> *mut LdEncoder;
    fn ld_encoder_create_detached(in_rate: u32, in_channels: u32, bitrate_bps: u32) -> *mut LdEncoder;
    fn ld_encoder_header(enc: *mut LdEncoder, out: *mut *const u8) -> usize;
    fn ld_encoder_pump(enc: *mut LdEncoder, out: *mut u8, cap: usize) -> usize;
    fn ld_encoder_feed(enc: *mut LdEncoder, frames: *const f32, count: u64);
    fn ld_encoder_destroy(enc: *mut LdEncoder);
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
#[repr(C)]
pub struct LdDecoder {
    _private: [u8; 0],
}
#[repr(C)]
pub struct LdEncoder {
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

/// Where a sound group sends its audio.
///
/// Audio in miniaudio flows through a graph of nodes toward the "endpoint"
/// (the speakers). The program bus is one extra node on that path: all track
/// groups are summed there first, which gives a single place to tap "the mix".
/// Master volume is applied after the bus, at the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupRoute {
    /// Feed the program bus (the broadcastable mix of all tracks).
    ProgramBus,
    /// Feed the endpoint directly, skipping the bus (local-only audio).
    Endpoint,
}

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

/// A `Send + Sync` copy of the raw engine pointer, handed to the loader thread
/// for SOUND CREATION ONLY (`create_sound_unattached`). It exists so a
/// background thread can open + pre-seek streaming sounds without holding the
/// `AudioEngine` mutex.
///
// SAFETY: This is the one place a miniaudio engine pointer is used off the
// audio mutex, so the argument is spelled out in full.
//
// (a) Internal synchronization. The engine operations reachable through an
//     `EngineRef` are: `ma_sound_init_from_file_w` (via `create_sound_unattached`);
//     `ma_sound_seek_to_second` and `ma_sound_get_length_in_seconds` (via
//     `Sound::seek_secs`/`Sound::length_secs`, called from `sound::preload` on
//     the loader thread to pre-seek and size the preloaded sound); and —
//     because a never-started, group-less `Sound` may be dropped on this
//     thread — `ma_sound_uninit`. All four mutate or read the miniaudio node
//     graph, and miniaudio synchronizes node attachment, detachment, and
//     uninitialization internally with spinlocks, crafted so that
//     `ma_node_graph_read_pcm_frames` on the audio thread stays lock-free
//     (miniaudio.h §7.2). Node uninit waits until the audio thread has
//     finished with the node before freeing it. `ma_sound_init_from_file_*`
//     also routes through the resource manager, whose multi-producer/
//     multi-consumer job queue is likewise internally synchronized (spinlock +
//     per-object serial execution counters, miniaudio.h §6.2.1, ~:1884-1898).
//     `ma_sound_seek_to_second` and `ma_sound_get_length_in_seconds` are safe
//     here for a stronger reason than synchronization alone: at the point
//     `sound::preload` calls them the sound is never-started — its node has
//     never entered the playing state, so the audio thread's graph read
//     (`ma_node_graph_read_pcm_frames`) skips it as a stopped node and never
//     concurrently touches its data source. The seek/length calls are
//     therefore not racing the audio thread over this node at all, let alone
//     relying on miniaudio's internal locking to arbitrate it. No other engine
//     state is touched through this handle: volume, clock, group, and every
//     already-created `Sound`/`Group` stay behind the `AudioEngine` mutex
//     exactly as before.
// (b) Lifetime. The pointer must outlive the loader thread. `AudioEngine` owns
//     both the loader thread (`loader` field, joined on drop) and the engine
//     (`engine` field, declared LAST so it drops after the loader is joined),
//     and `AudioEngine` lives inside `AppStateContainer`, whose `Arc` is held by
//     Tauri managed state for the process lifetime. The loader thread is joined
//     in `Loader::drop` before the `engine` field is dropped, so the pointer is
//     never dangling while the loader could dereference it.
// (c) Not for playback. `EngineRef` deliberately exposes no play/stop/volume/
//     clock method. A preloaded `Sound` is group-less and never started on the
//     loader thread; it only becomes audible after the poll tick attaches it to
//     a live group and starts it — under the mutex.
#[derive(Clone, Copy)]
pub struct EngineRef(*mut LdEngine);

unsafe impl Send for EngineRef {}
unsafe impl Sync for EngineRef {}

impl Engine {
    /// A sound-creation-only handle to this engine, safe to move to the loader
    /// thread (see [`EngineRef`]).
    pub fn engine_ref(&self) -> EngineRef {
        EngineRef(self.0)
    }
}

impl EngineRef {
    /// Create a **group-less** streaming sound routed to the engine endpoint,
    /// without starting it. Runs on the loader thread. The poll tick re-parents
    /// it to a live group (`Sound::attach_to_group`) and starts it under the mutex.
    pub fn create_sound_unattached(&self, path: &str) -> Result<Sound, String> {
        let wide = to_wide(path);
        // group = NULL -> engine endpoint (shim `ld_sound_create` accepts NULL).
        let ptr = unsafe { ld_sound_create(self.0, std::ptr::null_mut(), wide.as_ptr(), 1) };
        if ptr.is_null() {
            return Err(format!("failed to preload sound for {path}"));
        }
        Ok(Sound { ptr })
    }
}

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

    /// Engine output channel count (2 = stereo). Tap frames are interleaved
    /// with this many samples per frame.
    pub fn channels(&self) -> u32 {
        unsafe { ld_engine_channels(self.0) }
    }

    /// Turn the broadcast tap on or off.
    ///
    /// The tap is a "pass-through" stage spliced in after the program bus: the
    /// audio flows through it to the speakers unchanged, and while enabled it
    /// also copies those samples into a ring buffer (a fixed-size queue that
    /// wraps around) for [`Engine::tap_read`] to collect. Only groups created
    /// with [`GroupRoute::ProgramBus`] pass through it; the library preview
    /// (`GroupRoute::Endpoint`) never does. The flag is a single atomic the
    /// audio thread reads each block, so this never blocks playback.
    // The encoder reads the tap directly in C (ld_encoder_create/pump), so
    // outside tests nothing calls this wrapper.
    #[allow(dead_code)]
    pub fn tap_set_enabled(&self, enabled: bool) {
        unsafe { ld_tap_set_enabled(self.0, enabled as c_int) }
    }

    /// Pop tapped audio into `out`: interleaved f32 at the engine's
    /// [`sample_rate`](Engine::sample_rate) and [`channels`](Engine::channels).
    /// Returns whole frames read (0 when nothing is queued); only the first
    /// `frames * channels` samples of `out` are written.
    ///
    /// The ring buffer holds 2 seconds. The audio thread may never wait (it has
    /// a few milliseconds to hand the sound card each block), so if this is not
    /// called often enough the newest audio is dropped instead.
    ///
    /// Takes `&self` with no mutex because miniaudio's ring buffer is
    /// single-producer/single-consumer: the audio thread is the one producer.
    /// Caller contract: **only one thread reads**. While an [`Encoder`] made by
    /// [`Engine::create_encoder`] exists it is that reader, so do not call this.
    // The encoder reads the tap directly in C (ld_encoder_create/pump), so
    // outside tests nothing calls this wrapper.
    #[allow(dead_code)]
    pub fn tap_read(&self, out: &mut [f32]) -> usize {
        let ch = self.channels() as usize;
        if ch == 0 {
            return 0;
        }
        let max_frames = (out.len() / ch) as u64;
        if max_frames == 0 {
            return 0;
        }
        let n = unsafe { ld_tap_read(self.0, out.as_mut_ptr(), max_frames) };
        n.max(0) as usize
    }

    /// Create a sub-mix group attached to this engine, feeding `route`.
    pub fn create_group(&self, route: GroupRoute) -> Result<Group, String> {
        let on_program_bus = matches!(route, GroupRoute::ProgramBus) as c_int;
        let ptr = unsafe { ld_group_create(self.0, on_program_bus) };
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

    /// Create a broadcast encoder that reads this engine's tap (see
    /// [`Encoder`]). Turns the tap on and discards any stale audio it still
    /// held from an earlier broadcast; dropping the encoder turns it off.
    ///
    /// The tap has a single reader, and creation already reads it. Creating
    /// the encoder on one thread and pumping it on another is fine as long as
    /// ownership is handed over by spawning the pumping thread with the
    /// encoder moved into it (as `Broadcaster::start` does): a thread spawn
    /// orders everything the creating thread did before everything the new
    /// thread does, so the two never read the tap at the same time and there
    /// is still exactly one reader at any moment.
    pub fn create_encoder(&self, bitrate_bps: u32) -> Result<Encoder, String> {
        let ptr = unsafe { ld_encoder_create(self.0, bitrate_bps) };
        if ptr.is_null() {
            return Err("failed to create broadcast encoder".to_string());
        }
        Ok(Encoder { ptr, in_channels: self.channels() })
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

    /// Start from the current cursor WITHOUT seeking (the cursor was pre-set by
    /// `seek_secs` on the loader thread), applying `fade_in_secs`. Relies on
    /// `ld_sound_play` skipping its internal seek when `start_secs <= 0`.
    pub fn play_no_seek(&self, fade_in_secs: f64) -> Result<(), String> {
        let rc = unsafe { ld_sound_play(self.ptr, 0.0, fade_in_secs) };
        if rc != 0 {
            return Err(format!("failed to start preloaded sound (code {rc})"));
        }
        Ok(())
    }

    /// Stop playback. With `fade_out_secs > 0`, fades out first. Cursor is kept.
    pub fn stop(&self, fade_out_secs: f64) {
        unsafe { ld_sound_stop(self.ptr, fade_out_secs) }
    }

    /// Pause with a short (~30 ms) de-click fade; cursor preserved (see shim.c).
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

    /// Schedule a faded stop `secs_from_now` seconds ahead (engine clock) with a
    /// `fade_secs` ramp to silence ending at the stop point. Used to stop a clip
    /// exactly at its `trim_end` without overshoot or a hard-cut click.
    pub fn schedule_stop_secs(&self, secs_from_now: f64, fade_secs: f64) {
        unsafe { ld_sound_schedule_stop_secs(self.ptr, secs_from_now, fade_secs) }
    }

    /// Re-parent this sound to `group`'s sub-mix (from the engine endpoint a
    /// preloaded sound was created against). Must be called under the audio
    /// mutex, with `group` provably alive (see the loader handoff in mod.rs).
    pub fn attach_to_group(&self, group: &Group) {
        unsafe { ld_sound_attach_to_group(self.ptr, group.ptr) }
    }

    /// Seek to `secs` without starting. Called on the LOADER thread only (may do
    /// file I/O); never under the audio mutex.
    pub fn seek_secs(&self, secs: f64) {
        unsafe { ld_sound_seek_secs(self.ptr, secs) }
    }
}

impl Drop for Sound {
    fn drop(&mut self) {
        unsafe { ld_sound_destroy(self.ptr) }
    }
}

/// Ogg/Opus encoder for the broadcast stream (implemented in `shim.c`).
///
/// It turns PCM into bytes a browser `<audio>` element can play:
/// - **Opus** is the codec. It compresses audio in fixed 20 ms blocks (960
///   samples per channel at 48 kHz, its recommended default: long enough to
///   compress well, short enough to keep latency low) into small packets.
/// - **Ogg** is the container. It wraps packets into *pages*, each with a
///   short header carrying the stream's serial number, a page sequence number
///   and a *granule position*: the number of 48 kHz samples decoded by the end
///   of that page, which players use as a timestamp to seek and stay in sync.
/// - The stream opens with two *header pages*: OpusHead (format, channel
///   count, and *pre-skip*: how many warm-up samples the decoder should drop,
///   since the encoder's first output is lookahead rather than real audio)
///   and OpusTags (vendor "LuteDrop"). Every listener needs these first, then
///   can start at any audio page boundary.
///
/// Output is always 48 kHz stereo; other input rates and channel counts are
/// converted inside the shim. Two sources, fixed at creation: the engine's
/// tap ([`Engine::create_encoder`], production) or frames pushed in by hand
/// ([`Encoder::create_detached`] + [`Encoder::feed`], tests). Never used on
/// the audio thread. Dropping it frees the encoder and, for a tap source,
/// disables the tap; drop it before the `Engine` it reads from.
pub struct Encoder {
    ptr: *mut LdEncoder,
    /// Samples per input frame, so `feed` can turn a sample slice into the
    /// frame count the shim expects.
    in_channels: u32,
}

// SAFETY: An `Encoder` is owned and used by one thread at a time (the
// broadcast encoder thread; `&mut self` on `pump`/`feed` gives exclusive
// access), and the shim's encoder state is plain heap memory with no thread
// affinity. A tap-source encoder also holds the engine pointer, mirroring
// `EngineRef` (b): the owning `AudioEngine` joins the encoder thread before
// its `engine` field drops, so the pointer is never dangling while the
// encoder can use it. Through it the encoder touches only the tap's atomic
// enable flag and the read side of its single-producer/single-consumer ring
// buffer, both built for use off the audio mutex; this encoder is that
// buffer's one consumer.
unsafe impl Send for Encoder {}

impl Encoder {
    /// Create a feed-source encoder (no engine): input arrives through
    /// [`feed`](Encoder::feed) as interleaved f32 frames at `in_rate` Hz with
    /// `in_channels` samples per frame. Used by tests.
    // Only the tests construct a feed-source encoder; production uses the tap.
    #[allow(dead_code)]
    pub fn create_detached(in_rate: u32, in_channels: u32, bitrate_bps: u32) -> Result<Encoder, String> {
        let ptr = unsafe { ld_encoder_create_detached(in_rate, in_channels, bitrate_bps) };
        if ptr.is_null() {
            return Err(format!("failed to create encoder for {in_rate} Hz / {in_channels} ch input"));
        }
        Ok(Encoder { ptr, in_channels })
    }

    /// The two header pages (OpusHead, OpusTags) every listener must receive
    /// before any audio page. Copied out of the shim.
    pub fn header(&self) -> Vec<u8> {
        let mut p: *const u8 = std::ptr::null();
        let len = unsafe { ld_encoder_header(self.ptr, &mut p) };
        if p.is_null() || len == 0 {
            return Vec::new();
        }
        // SAFETY: the shim keeps the header buffer alive for the encoder's
        // lifetime, and `self` stays borrowed for the duration of this copy.
        unsafe { std::slice::from_raw_parts(p, len) }.to_vec()
    }

    /// Encode whatever input is pending (for a tap source, everything the tap
    /// collected since the last call) and append the finished Ogg pages to
    /// `out`. Returns bytes appended; 0 means nothing is ready yet. What is
    /// appended always starts and ends on a page boundary.
    pub fn pump(&mut self, out: &mut Vec<u8>) -> usize {
        // 64 KiB holds any Ogg page (at most 65 307 bytes). Pages that do not
        // fit in one call stay queued in the shim, so call until it is dry.
        let mut scratch = vec![0u8; 64 * 1024];
        let mut total = 0;
        loop {
            let n = unsafe { ld_encoder_pump(self.ptr, scratch.as_mut_ptr(), scratch.len()) };
            if n == 0 {
                break;
            }
            out.extend_from_slice(&scratch[..n]);
            total += n;
        }
        total
    }

    /// Feed-source only: queue interleaved frames at the `in_rate` /
    /// `in_channels` given to [`create_detached`](Encoder::create_detached).
    /// A trailing partial frame is ignored. On a tap-source encoder the shim
    /// logs a warning and ignores the call.
    // Only the tests feed frames by hand; production uses the tap.
    #[allow(dead_code)]
    pub fn feed(&mut self, frames: &[f32]) {
        let count = (frames.len() / self.in_channels.max(1) as usize) as u64;
        if count == 0 {
            return;
        }
        unsafe { ld_encoder_feed(self.ptr, frames.as_ptr(), count) }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe { ld_encoder_destroy(self.ptr) }
    }
}

/// Frames per `read_chunk` call. 4096 frames × channels × 4 bytes ≈ 32 KB for
/// stereo — the whole point versus the old whole-file decode.
pub const DECODE_CHUNK_FRAMES: usize = 4096;

/// Chunked streaming decoder for offline analysis (peak generation, duration
/// probing). NOT for playback — playback streams through `ma_sound` directly.
/// Owns the C decoder; closed on drop. Used within a single thread (created
/// and consumed inside one call/spawn_blocking closure) — deliberately !Send.
pub struct StreamingDecoder {
    ptr: *mut LdDecoder,
    pub channels: u32,
    pub sample_rate: u32,
    buf: Vec<f32>,
}

impl StreamingDecoder {
    pub fn open(path: &str) -> Result<Self, String> {
        let wide = to_wide(path);
        let (mut channels, mut sample_rate) = (0u32, 0u32);
        let ptr = unsafe { ld_decoder_open(wide.as_ptr(), &mut channels, &mut sample_rate) };
        if ptr.is_null() {
            return Err(format!("Failed to open audio decoder for {path}"));
        }
        let ch = channels.max(1) as usize;
        Ok(Self {
            ptr,
            channels,
            sample_rate,
            buf: vec![0.0; DECODE_CHUNK_FRAMES * ch],
        })
    }

    /// Next chunk of interleaved samples; `Ok(None)` at end of stream.
    /// The returned slice borrows this decoder's internal buffer and is valid
    /// until the next call.
    pub fn read_chunk(&mut self) -> Result<Option<&[f32]>, String> {
        let n = unsafe { ld_decoder_read(self.ptr, self.buf.as_mut_ptr(), DECODE_CHUNK_FRAMES as u64) };
        if n < 0 {
            return Err("Audio decode failed mid-stream (corrupt file?)".to_string());
        }
        if n == 0 {
            return Ok(None);
        }
        let samples = (n as usize) * self.channels.max(1) as usize;
        Ok(Some(&self.buf[..samples]))
    }
}

impl Drop for StreamingDecoder {
    fn drop(&mut self) {
        unsafe { ld_decoder_close(self.ptr) }
    }
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
        let mut dec = StreamingDecoder::open(p.to_str().unwrap()).expect("open decoder");
        assert_eq!(dec.sample_rate, 48000);
        assert_eq!(dec.channels, 2);
        let mut total_frames: u64 = 0;
        while let Some(chunk) = dec.read_chunk().expect("read chunk") {
            total_frames += chunk.len() as u64 / dec.channels as u64;
        }
        // ~120s fixture at 48k; allow one packet of slack.
        assert!((total_frames as i64 - 5_760_000).abs() < 10_000, "frames={total_frames}");
        assert!(total_frames > 0, "decoded zero frames");
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

        let result = StreamingDecoder::open(dst.to_str().unwrap());
        let read_result = result.map(|mut dec| {
            let chunk_len = dec.read_chunk().expect("read one chunk").map(|c| c.len()).unwrap_or(0);
            (dec.sample_rate, chunk_len)
        });

        // Clean up before asserting so a failure doesn't leak the temp file.
        let _ = std::fs::remove_file(&dst);

        let (sample_rate, chunk_len) = read_result.expect("decode of non-ASCII path");
        assert!(chunk_len > 0, "decoded zero samples from unicode path");
        assert_eq!(sample_rate, 48000);
    }

    #[test]
    fn decode_garbage_file_errors_instead_of_silent_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garbage.mp3");
        std::fs::write(&path, [0xDEu8, 0xAD, 0xBE, 0xEF].repeat(1024)).unwrap();
        assert!(StreamingDecoder::open(path.to_str().unwrap()).is_err());
    }

    #[test]
    fn groups_can_route_to_bus_or_endpoint() {
        let eng = Engine::new().expect("engine");
        let on_bus = eng.create_group(GroupRoute::ProgramBus).expect("bus group");
        let direct = eng.create_group(GroupRoute::Endpoint).expect("endpoint group");
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let a = eng.create_sound(Some(&on_bus), p.to_str().unwrap(), true).expect("sound on bus");
        let b = eng.create_sound(Some(&direct), p.to_str().unwrap(), true).expect("sound direct");
        assert!(a.length_secs() > 0.0 && b.length_secs() > 0.0);
    }

    /// Pull everything currently queued in the tap's ring buffer.
    fn drain(eng: &Engine) -> Vec<f32> {
        let ch = eng.channels() as usize;
        let mut scratch = vec![0f32; 4800 * ch];
        let mut all = Vec::new();
        loop {
            let frames = eng.tap_read(&mut scratch);
            if frames == 0 {
                break;
            }
            all.extend_from_slice(&scratch[..frames * ch]);
        }
        all
    }

    /// Largest absolute sample value (0.0 for silence or an empty slice).
    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn tap_captures_bus_but_not_endpoint_groups() {
        let eng = Engine::new().expect("engine");
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");

        // Preview-style group: must never reach the tap.
        let direct = eng.create_group(GroupRoute::Endpoint).unwrap();
        let s = eng.create_sound(Some(&direct), p.to_str().unwrap(), true).unwrap();
        eng.tap_set_enabled(true);
        s.play(10.0, 0.0).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let captured = drain(&eng);
        assert!(!captured.is_empty(), "tap produced no frames while enabled");
        assert!(peak(&captured) < 1e-6, "endpoint-routed audio leaked into the tap");
        s.stop(0.0);

        // Track-style group: must be captured.
        let on_bus = eng.create_group(GroupRoute::ProgramBus).unwrap();
        let t = eng.create_sound(Some(&on_bus), p.to_str().unwrap(), true).unwrap();
        t.play(10.0, 0.0).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(peak(&drain(&eng)) > 0.01, "bus-routed audio did not reach the tap");

        // Disabled tap yields nothing.
        eng.tap_set_enabled(false);
        drain(&eng);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(drain(&eng).is_empty());
    }

    /// One Ogg page as seen by [`ogg_pages`].
    struct OggPage {
        granule: u64,
        serial: u32,
        seq: u32,
        header_type: u8,
        body: Vec<u8>,
    }

    /// Minimal Ogg page walker for the encoder tests. Each page is a 27-byte
    /// fixed header ("OggS", version, header_type, granule u64 LE, serial u32
    /// LE, sequence u32 LE, CRC u32, segment count), then a segment table of
    /// that many length bytes, then a body whose length is their sum. The CRC
    /// is not checked; the decode tests cover stream validity.
    fn ogg_pages(bytes: &[u8]) -> Vec<OggPage> {
        let mut pages = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let h = &bytes[at..];
            assert!(h.len() >= 27, "truncated page header at byte {at}");
            assert_eq!(&h[0..4], b"OggS", "missing capture pattern at byte {at}");
            assert_eq!(h[4], 0, "Ogg version");
            let header_type = h[5];
            let granule = u64::from_le_bytes(h[6..14].try_into().unwrap());
            let serial = u32::from_le_bytes(h[14..18].try_into().unwrap());
            let seq = u32::from_le_bytes(h[18..22].try_into().unwrap());
            // h[22..26] is the CRC, skipped.
            let n_segments = h[26] as usize;
            let table = &h[27..27 + n_segments];
            let body_len: usize = table.iter().map(|&b| b as usize).sum();
            let body_start = 27 + n_segments;
            let body = h[body_start..body_start + body_len].to_vec();
            pages.push(OggPage { granule, serial, seq, header_type, body });
            at += body_start + body_len;
        }
        pages
    }

    #[test]
    fn encoder_header_is_opus_head_then_tags() {
        let eng = Engine::new().unwrap();
        let enc = eng.create_encoder(128_000).unwrap();
        let pages = ogg_pages(&enc.header());
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].header_type & 0x02, 0x02, "first page is beginning-of-stream");
        assert!(pages[0].body.starts_with(b"OpusHead"));
        assert_eq!(pages[0].body[9], 2, "channel count");
        assert!(pages[1].body.starts_with(b"OpusTags"));
        assert_eq!(pages[0].serial, pages[1].serial);
    }

    #[test]
    fn encoder_pages_have_monotonic_granules_and_same_serial() {
        let eng = Engine::new().unwrap();
        let mut enc = eng.create_encoder(128_000).unwrap();
        let serial = ogg_pages(&enc.header())[0].serial;
        let mut out = Vec::new();
        for _ in 0..20 {
            // ~0.4 s of silence from the (enabled) tap
            std::thread::sleep(std::time::Duration::from_millis(20));
            enc.pump(&mut out);
        }
        let pages = ogg_pages(&out);
        assert!(pages.len() >= 2, "got {} pages", pages.len());
        let mut last = 0u64;
        let mut seq = pages[0].seq;
        for p in &pages {
            assert_eq!(p.serial, serial);
            assert!(p.granule >= last);
            last = p.granule;
            assert_eq!(p.seq, seq);
            seq += 1;
        }
    }

    #[test]
    fn encoder_output_decodes_and_carries_bus_audio() {
        let eng = Engine::new().unwrap();
        let on_bus = eng.create_group(GroupRoute::ProgramBus).unwrap();
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.opus");
        let s = eng.create_sound(Some(&on_bus), p.to_str().unwrap(), true).unwrap();
        let mut enc = eng.create_encoder(128_000).unwrap();
        s.play(10.0, 0.0).unwrap();
        let mut out = enc.header();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            enc.pump(&mut out);
        }
        s.stop(0.0);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("captured.opus");
        std::fs::write(&path, &out).unwrap();
        // opusfile tolerates a stream with no end-of-stream page, which is what a live capture is.
        let mut dec = StreamingDecoder::open(path.to_str().unwrap()).expect("captured stream decodes");
        assert_eq!(dec.sample_rate, 48000);
        assert_eq!(dec.channels, 2);
        let mut peak = 0f32;
        while let Some(chunk) = dec.read_chunk().unwrap() {
            for v in chunk {
                peak = peak.max(v.abs());
            }
        }
        assert!(peak > 0.01, "decoded stream is silent (peak {peak})");
    }

    #[test]
    fn encoder_accepts_44100_mono_input() {
        let mut enc = Encoder::create_detached(44_100, 1, 128_000).unwrap();
        // 0.5 s of a 440 Hz tone, mono, 44.1 kHz.
        let tone: Vec<f32> = (0..22_050)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.5)
            .collect();
        enc.feed(&tone);
        let mut out = enc.header();
        enc.pump(&mut out);
        let pages = ogg_pages(&out);
        assert!(pages.len() > 2);
        assert_eq!(pages[0].body[9], 2, "output is stereo regardless of input");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("tone.opus");
        std::fs::write(&path, &out).unwrap();
        let mut dec = StreamingDecoder::open(path.to_str().unwrap()).unwrap();
        assert_eq!(dec.sample_rate, 48000);
        let mut frames = 0u64;
        while let Some(c) = dec.read_chunk().unwrap() {
            frames += c.len() as u64 / 2;
        }
        assert!((frames as i64 - 24_000).abs() < 2_000, "≈0.5 s at 48 kHz, got {frames}");
    }

    #[test]
    fn engine_creates_and_loads_fixture() {
        let eng = Engine::new().expect("engine");
        let g = eng.create_group(GroupRoute::ProgramBus).expect("group");
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
