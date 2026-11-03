#ifndef LD_SHIM_H
#define LD_SHIM_H
#include <stdint.h>
#include <stddef.h>
#include <wchar.h>

// Paths are passed as wide (UTF-16) strings and forwarded to miniaudio's `_w`
// file APIs. On Windows the narrow file APIs route through fopen and interpret
// path bytes in the ANSI/locale codepage, so non-ASCII UTF-8 paths fail to
// open; the wide APIs (_wfopen) handle full Unicode. Rust builds these from
// OsStr::encode_wide (see audio/ffi.rs).

// ---- Streaming decode ---------------------------------------------------
// Chunked decode for peak generation: open a decoder, pull interleaved f32
// frames a chunk at a time, close. Replaces the old whole-file decode API
// (which materialized the entire file in RAM).
typedef struct ld_decoder ld_decoder;

// Returns NULL if the file can't be opened / decoder init fails.
ld_decoder* ld_decoder_open(const wchar_t* path, uint32_t* out_channels, uint32_t* out_sample_rate);
// Reads up to max_frames interleaved f32 frames into out.
// Returns frames read (0 = end of stream), or -1 on a hard mid-stream decode error.
int64_t ld_decoder_read(ld_decoder*, float* out, uint64_t max_frames);
void ld_decoder_close(ld_decoder*);

// Cheap duration probe: inits a decoder (libopus backend registered, same as
// ld_decoder_open), reads the PCM frame count via ma_decoder_get_length_in_pcm_frames
// and divides by the output sample rate -- NO full decode. Returns the duration in
// seconds, or a negative value if the file can't be opened or its length is unknown
// (e.g. a streamed source with no reported length).
double ld_file_duration_secs(const wchar_t* path);

// ---- Playback layer ---------------------------------------------------------
// Opaque handles. The shim owns the underlying miniaudio structs.
typedef struct ld_engine ld_engine;
typedef struct ld_group  ld_group;
typedef struct ld_sound  ld_sound;

// Engine wraps ma_engine + a ma_resource_manager that registers the libopus
// backend (so streamed .opus works). Returns NULL on failure.
ld_engine* ld_engine_create(void);
void       ld_engine_destroy(ld_engine*);
void       ld_engine_set_volume(ld_engine*, float linear);   // master volume
uint64_t   ld_engine_time_frames(ld_engine*);                // global engine clock
uint32_t   ld_engine_sample_rate(ld_engine*);
uint32_t   ld_engine_channels(ld_engine*);

// Broadcast tap: a pass-through node between the program bus and the endpoint
// (bus -> tap -> endpoint). While enabled it copies every frame the bus carries
// (silence included) into a 2-second lock-free ring buffer; groups routed
// straight to the endpoint never reach it. If the reader falls behind, the
// newest frames are dropped -- the audio thread never waits.
// ld_tap_set_enabled: atomic flag the audio thread reads; 0 = copy nothing.
void       ld_tap_set_enabled(ld_engine*, int enabled);
// Pops up to max_frames interleaved f32 frames at the ENGINE's sample rate and
// channel count into out (which must hold max_frames * channels floats).
// Returns frames read (0 = nothing queued). Single consumer only: call from
// one thread at a time.
int64_t    ld_tap_read(ld_engine*, float* out, uint64_t max_frames);

// Group wraps ma_sound_group (a per-track / library sub-mix). Returns NULL on failure.
// on_program_bus != 0: the group feeds the engine's program bus (the mix of all
// tracks; the future tap point for broadcasting). 0: it feeds the endpoint
// directly, bypassing the bus (used by the library preview so it is never
// broadcast). Master volume is NOT applied at the bus; it stays on the engine
// (ld_engine_set_volume), downstream of the bus.
ld_group*  ld_group_create(ld_engine*, int on_program_bus);
void       ld_group_set_volume(ld_group*, float linear);
void       ld_group_destroy(ld_group*);

// Sound wraps ma_sound. stream != 0 => streaming (use for playback);
// group may be NULL (routes to the engine endpoint). Returns NULL on failure.
ld_sound*  ld_sound_create(ld_engine*, ld_group*, const wchar_t* path, int stream);
int        ld_sound_play(ld_sound*, double start_secs, double fade_in_secs);
void       ld_sound_stop(ld_sound*, double fade_out_secs);
void       ld_sound_pause(ld_sound*);    // = stop (keeps cursor)
void       ld_sound_resume(ld_sound*);   // = start (cursor preserved)
void       ld_sound_set_volume(ld_sound*, float linear);
double     ld_sound_position_secs(ld_sound*);
double     ld_sound_length_secs(ld_sound*);   // total length in seconds (0 if unknown)
int        ld_sound_is_playing(ld_sound*);
int        ld_sound_at_end(ld_sound*);
void       ld_sound_schedule_start_frame(ld_sound*, uint64_t engine_frame);
// Schedule a faded stop `secs_from_now` seconds ahead (computed on the engine
// clock) with a `fade_secs` ramp to silence ending at the stop point, so a clip
// with a trim_end stops exactly there without overshoot or a hard-cut click.
// Floored at LD_MIN_FADE_MS and clamped so the fade never starts in the past.
void       ld_sound_schedule_stop_secs(ld_sound*, double secs_from_now, double fade_secs);
// Re-parent an already-created sound to `g` (or to the engine endpoint if g is
// NULL). Wraps ma_node_attach_output_bus (the sound node's output bus 0 -> the
// group's input bus 0), which is spinlock-synchronized against the audio thread
// (miniaudio.h 7.2). Used at preload handoff to attach a group-less preloaded
// sound to its target Voice's group, under the audio mutex.
void       ld_sound_attach_to_group(ld_sound*, ld_group* g);
// Seek to `secs` in the data source's own time base WITHOUT starting playback.
// Streaming seeks can do file I/O (decode-skip for opus/mp3), so this runs on
// the loader thread before handoff, never under the audio mutex.
void       ld_sound_seek_secs(ld_sound*, double secs);
void       ld_sound_destroy(ld_sound*);

// ---- Broadcast encoder ------------------------------------------------------
// Turns audio into an Ogg/Opus byte stream a browser can play. Opus is the
// codec (it squeezes 20 ms blocks of PCM into small "packets"); Ogg is the
// container (it wraps packets into "pages", each with a small header carrying
// a stream serial number, a sequence number and a timestamp, so a listener can
// pick up the stream at any page boundary). Output is always 48 kHz stereo at
// the requested bitrate, whatever the input rate/channel count.
//
// Two sources, fixed at creation:
//   tap  -- production: reads the engine's broadcast tap (ld_tap_read).
//   feed -- tests: frames are pushed in by hand with ld_encoder_feed.
// An encoder is used from ONE thread (the encoder thread) and never from the
// audio thread. For a tap source that thread is also the tap's single reader:
// create/pump/destroy must all happen on it.
typedef struct ld_encoder ld_encoder;

// Tap source: input rate/channels come from the engine. Enables the tap and
// discards whatever stale audio it was still holding, so a new broadcast never
// starts with old frames. NULL on failure.
ld_encoder* ld_encoder_create(ld_engine*, uint32_t bitrate_bps);
// Feed source: no engine involved; input arrives via ld_encoder_feed at
// in_rate / in_channels. NULL on failure.
ld_encoder* ld_encoder_create_detached(uint32_t in_rate, uint32_t in_channels, uint32_t bitrate_bps);
// The two header pages (OpusHead, OpusTags) every listener must receive before
// any audio page. *out is valid for the encoder's lifetime. Returns its length.
size_t      ld_encoder_header(ld_encoder*, const uint8_t** out);
// Pulls pending input (tap source: drains the tap first), encodes every
// complete 20 ms block, and copies whole Ogg pages into out while they fit.
// Pages that do not fit wait for the next call; out never ends mid-page, so
// every chunk returned starts at a page boundary. cap should be at least
// 65307 bytes (the largest possible Ogg page) or a large page can never be
// delivered. Returns bytes written; 0 means nothing ready yet.
size_t      ld_encoder_pump(ld_encoder*, uint8_t* out, size_t cap);
// Feed source only: queue `count` interleaved frames at in_rate/in_channels.
// On a tap-source encoder this logs a warning and does nothing.
void        ld_encoder_feed(ld_encoder*, const float* frames, uint64_t count);
// Tap source: disables the tap. Frees everything. NULL is a no-op.
void        ld_encoder_destroy(ld_encoder*);

#endif
