// stb_vorbis (Ogg Vorbis decoder) is a single file: declarations first, the
// implementation further down. Its declarations must come before miniaudio.h,
// which switches on its own built-in Vorbis decoder when it sees them.
#define STB_VORBIS_HEADER_ONLY
#include "stb_vorbis.c"

#define MINIAUDIO_IMPLEMENTATION
#define MA_NO_GENERATION
#include "miniaudio.h"

// stb_vorbis's implementation, with its assert() calls compiled out: they
// check stream invariants a malformed .ogg can break, and a failed assert
// aborts the whole app. With them off, stb_vorbis reports the bad stream as a
// decode error instead. NDEBUG is what turns assert() off; <assert.h> re-reads
// it each time it is included, so asserts are switched back on (if they were
// on) for everything after this block.
#undef STB_VORBIS_HEADER_ONLY
#ifndef NDEBUG
#define LD_ASSERTS_WERE_ON
#define NDEBUG
#endif
#include "stb_vorbis.c"
#ifdef LD_ASSERTS_WERE_ON
#undef NDEBUG
#undef LD_ASSERTS_WERE_ON
#endif
#include <assert.h>
// stb_vorbis leaves these one-letter macros defined; Windows headers included
// after this point (Media Foundation's) use them as field names.
#undef L
#undef C
#undef R

#include "miniaudio_libopus.h"
#include "miniaudio_libopus.c"
#include "miniaudio_vorbis_pull.h"
#include "miniaudio_vorbis_pull.c"
#include "miniaudio_mf.h"
#include "miniaudio_mf.c"
#include "shim.h"
#include <stdlib.h>

// The custom decoding backends, in the order miniaudio tries them. miniaudio
// offers every file to these BEFORE its built-in WAV/FLAC/MP3/Vorbis decoders,
// moving to the next backend when one rejects the file:
//   1. libopus: Ogg Opus (.opus, and .ogg files that hold Opus).
//   2. stb_vorbis in pull mode: Ogg Vorbis, with exact length and fast seeks
//      (see miniaudio_vorbis_pull.h for why not miniaudio's built-in one).
//   3. Media Foundation: AAC, ALAC, WMA. Claims only MP4/ADTS/ASF content, so
//      MP3/WAV/FLAC still fall through to miniaudio's built-ins.
// Both Ogg backends check the codec header and reject the other codec, so an
// .ogg file reaches the right one whichever it contains.
//
// Both decoder setups below (ld_make_decoder for analysis, the resource
// manager for playback) must use this same list.
//
// NOTE: each `ld_decoding_backend_*` / `ma_decoding_backend_libopus` is declared
// as `extern ma_decoding_backend_vtable*` -- ALREADY a pointer, so the array
// holds it directly (NOT its address).
#define LD_BACKENDS { ma_decoding_backend_libopus, ld_decoding_backend_vorbis, ld_decoding_backend_mf }

// Path is a wide (UTF-16) string and we call miniaudio's `_w` file APIs. On
// Windows, miniaudio's narrow file APIs route the path through fopen, which
// interprets the bytes in the ANSI/locale codepage rather than UTF-8 -- so any
// non-ASCII UTF-8 path fails to open. The wide APIs go straight to _wfopen, so
// Unicode paths (smart quotes, accents, CJK, etc.) open correctly.
static ma_result ld_make_decoder(const wchar_t* path, ma_decoder* dec) {
    ma_decoding_backend_vtable* backends[] = LD_BACKENDS;
    ma_decoder_config cfg = ma_decoder_config_init(ma_format_f32, 0, 0);
    cfg.ppCustomBackendVTables = backends;
    cfg.customBackendCount = sizeof(backends) / sizeof(backends[0]);
    return ma_decoder_init_file_w(path, &cfg, dec);
}

typedef struct ld_decoder { ma_decoder dec; } ld_decoder;

ld_decoder* ld_decoder_open(const wchar_t* path, uint32_t* out_channels, uint32_t* out_sample_rate) {
    ld_decoder* d = (ld_decoder*)malloc(sizeof(ld_decoder));
    if (d == NULL) return NULL;
    if (ld_make_decoder(path, &d->dec) != MA_SUCCESS) { free(d); return NULL; }
    *out_channels = d->dec.outputChannels;
    *out_sample_rate = d->dec.outputSampleRate;
    return d;
}

int64_t ld_decoder_read(ld_decoder* d, float* out, uint64_t max_frames) {
    ma_uint64 framesRead = 0;
    ma_result r = ma_decoder_read_pcm_frames(&d->dec, out, max_frames, &framesRead);
    // MA_AT_END with a final partial chunk still returns those frames; the
    // NEXT call returns 0. A hard mid-stream error must not masquerade as EOF.
    if (r != MA_SUCCESS && r != MA_AT_END) return -1;
    return (int64_t)framesRead;
}

void ld_decoder_close(ld_decoder* d) {
    if (d == NULL) return;
    ma_decoder_uninit(&d->dec);
    free(d);
}

int ld_decoder_seek(ld_decoder* d, uint64_t frame) {
    if (d == NULL) return -1;
    return (ma_decoder_seek_to_pcm_frame(&d->dec, frame) == MA_SUCCESS) ? 0 : -1;
}

uint64_t ld_decoder_length(ld_decoder* d) {
    ma_uint64 total = 0;
    if (d == NULL || ma_decoder_get_length_in_pcm_frames(&d->dec, &total) != MA_SUCCESS) return 0;
    return total;
}

ld_decoder* ld_decoder_open_callbacks(const wchar_t* path, uint32_t* out_channels, uint32_t* out_sample_rate) {
    ld_decoder* d = (ld_decoder*)malloc(sizeof(ld_decoder));
    if (d == NULL) return NULL;
    ma_decoding_backend_vtable* backends[] = LD_BACKENDS;
    ma_decoder_config cfg = ma_decoder_config_init(ma_format_f32, 0, 0);
    cfg.ppCustomBackendVTables = backends;
    cfg.customBackendCount = sizeof(backends) / sizeof(backends[0]);
    // The call the resource manager makes (with its default VFS = NULL).
    if (ma_decoder_init_vfs_w(NULL, path, &cfg, &d->dec) != MA_SUCCESS) { free(d); return NULL; }
    *out_channels = d->dec.outputChannels;
    *out_sample_rate = d->dec.outputSampleRate;
    return d;
}

double ld_file_duration_secs(const wchar_t* path) {
    ma_decoder dec;
    if (ld_make_decoder(path, &dec) != MA_SUCCESS) return -1.0;
    ma_uint64 total = 0;
    ma_result r = ma_decoder_get_length_in_pcm_frames(&dec, &total);
    ma_uint32 sr = dec.outputSampleRate;
    ma_decoder_uninit(&dec);
    if (r != MA_SUCCESS || total == 0 || sr == 0) return -1.0;
    return (double)total / (double)sr;
}

// ---- Broadcast tap ----------------------------------------------------------
//
// A "pass-through node" is a stage in miniaudio's node graph (the chain of
// mixers/effects audio flows through on its way to the speakers) that hands its
// input to its output unchanged. Ours sits between the program bus and the
// endpoint, so everything the bus carries -- and nothing routed straight to the
// endpoint, like the library preview -- flows through it. When enabled, it also
// copies those frames aside into a ring buffer for the broadcast encoder.
//
// A ring buffer is a fixed-size block of memory used as a queue that wraps
// around: the producer (here, the audio thread) writes at one position, the
// consumer (the encoder thread, via ld_tap_read) reads at another, and both
// positions loop back to the start when they reach the end. miniaudio's
// ma_pcm_rb is single-producer/single-consumer and lock-free: each side only
// moves its own position with an atomic store, so neither ever waits for the
// other.
//
// Why the audio thread may never wait: it must deliver the next block of
// samples to the sound card within a few milliseconds. If it blocks on a lock,
// allocates memory (which can take a lock inside the allocator), or does I/O,
// a slow moment on another thread makes it miss that deadline and the speakers
// glitch. So the process callback does only a memcpy, an atomic load, and
// ring-buffer acquire/commit. If the encoder falls behind and the buffer is
// full, the newest frames are simply dropped rather than waiting for room.
typedef struct ld_tap {
    ma_node_base base;          // MUST be first: miniaudio treats ld_tap* as ma_node*
    ma_pcm_rb rb;               // LD_TAP_SECONDS of f32 frames at the engine rate/channels
    ma_atomic_bool32 enabled;   // written by ld_tap_set_enabled, read on the audio thread
    ma_uint32 channels;         // engine channel count (fixed at init)
} ld_tap;

#define LD_TAP_SECONDS 2

// Audio-thread callback. Copies input -> output (the pass-through), then, if
// enabled, pushes the same frames into the ring buffer.
static void ld_tap_process(ma_node* pNode, const float** ppFramesIn, ma_uint32* pFrameCountIn,
                           float** ppFramesOut, ma_uint32* pFrameCountOut) {
    ld_tap* tap = (ld_tap*)pNode;
    const float* in = (ppFramesIn != NULL) ? ppFramesIn[0] : NULL;
    ma_uint32 in_frames = (pFrameCountIn != NULL) ? *pFrameCountIn : 0;
    float* out = ppFramesOut[0];
    ma_uint32 frames = *pFrameCountOut;

    // In continuous mode (without MA_NODE_FLAG_ALLOW_NULL_INPUT) miniaudio
    // posts zeroed input when nothing upstream delivers data, so `in` should
    // always be valid. Guard anyway: no input means silence out.
    if (in == NULL || in_frames == 0) {
        ma_silence_pcm_frames(out, frames, ma_format_f32, tap->channels);
        if (pFrameCountIn != NULL) *pFrameCountIn = 0;
    } else {
        if (in_frames < frames) frames = in_frames;
        MA_COPY_MEMORY(out, in, (size_t)frames * tap->channels * sizeof(float));
        *pFrameCountIn = frames;
    }
    *pFrameCountOut = frames;

    if (!ma_atomic_bool32_get(&tap->enabled)) return;

    // acquire_write hands back at most the contiguous free space up to the end
    // of the buffer, so a write that straddles the wrap point takes two passes.
    // Once the buffer is full acquire returns 0 frames and the rest is dropped.
    ma_uint32 done = 0;
    while (done < frames) {
        ma_uint32 chunk = frames - done;
        void* dst = NULL;
        if (ma_pcm_rb_acquire_write(&tap->rb, &chunk, &dst) != MA_SUCCESS || chunk == 0) break;
        MA_COPY_MEMORY(dst, out + (size_t)done * tap->channels,
                       (size_t)chunk * tap->channels * sizeof(float));
        if (ma_pcm_rb_commit_write(&tap->rb, chunk) != MA_SUCCESS) break;
        done += chunk;
    }
}

static ma_node_vtable g_ld_tap_vtable = {
    ld_tap_process,
    NULL,   // onGetRequiredInputFrameCount: 1 frame in -> 1 frame out
    1,      // input buses
    1,      // output buses
    // Keep firing even when nothing plays, so silence is still tapped (the
    // encoder must keep emitting pages while the board is quiet).
    MA_NODE_FLAG_CONTINUOUS_PROCESSING
};

// Allocates the ring buffer and inits the (not yet wired) node. Runs once at
// engine creation, on the caller's thread -- never on the audio thread.
static ma_result ld_tap_init(ma_engine* engine, ld_tap* tap) {
    ma_uint32 channels = ma_engine_get_channels(engine);
    ma_uint32 frames = ma_engine_get_sample_rate(engine) * LD_TAP_SECONDS;
    tap->channels = channels;
    ma_atomic_bool32_set(&tap->enabled, MA_FALSE);

    ma_result r = ma_pcm_rb_init(ma_format_f32, channels, frames, NULL, NULL, &tap->rb);
    if (r != MA_SUCCESS) return r;

    ma_node_config cfg = ma_node_config_init();
    cfg.vtable = &g_ld_tap_vtable;
    cfg.pInputChannels = &channels;
    cfg.pOutputChannels = &channels;
    r = ma_node_init(ma_engine_get_node_graph(engine), &cfg, NULL, &tap->base);
    if (r != MA_SUCCESS) {
        ma_pcm_rb_uninit(&tap->rb);
        return r;
    }
    return MA_SUCCESS;
}

// ---- Playback layer ---------------------------------------------------------

struct ld_engine {
    ma_engine engine;
    ma_resource_manager resource_manager;
    // Set when the engine was created silent: a context restricted to the
    // null backend, so nothing reaches a real output device (tests, CI).
    ma_context context;
    int has_context;
    // The "program bus": a sound group every track's group feeds into. Think of
    // it as a mixing-desk submaster: all tracks are summed here before reaching
    // the speakers. It is parented to the endpoint, so what it carries still
    // gets master volume (applied by the engine) afterwards. The broadcast tap
    // (below) captures the mix right after this bus, before master volume.
    ma_sound_group bus;
    // Pass-through node spliced in after the bus: bus -> tap -> endpoint (see
    // "Broadcast tap" above).
    ld_tap tap;
};

struct ld_group {
    ma_sound_group group;
};

struct ld_sound {
    ma_sound sound;
    ld_engine* owner;   // for sample-rate conversions
};

static ld_engine* ld_engine_create_impl(int silent) {
    ld_engine* e = (ld_engine*)calloc(1, sizeof(ld_engine));
    if (e == NULL) return NULL;

    if (silent) {
        // The null backend behaves like a device (it keeps a clock and pulls
        // frames) but is wired to nothing, so a test run plays no sound.
        ma_backend backends_null[] = { ma_backend_null };
        if (ma_context_init(backends_null, 1, NULL, &e->context) != MA_SUCCESS) {
            free(e);
            return NULL;
        }
        e->has_context = 1;
    }

    // Same custom backends as ld_make_decoder (see LD_BACKENDS). The resource
    // manager opens files through ma_decoder_init_vfs_w, which only calls each
    // backend's onInit (read/seek/tell callbacks), never onInitFileW.
    // miniaudio copies this array, so a stack array is fine.
    ma_decoding_backend_vtable* backends[] = LD_BACKENDS;

    ma_resource_manager_config rmcfg = ma_resource_manager_config_init();
    rmcfg.decodedFormat = ma_format_f32;
    rmcfg.ppCustomDecodingBackendVTables = backends;
    rmcfg.customDecodingBackendCount = sizeof(backends) / sizeof(backends[0]);
    if (ma_resource_manager_init(&rmcfg, &e->resource_manager) != MA_SUCCESS) {
        if (e->has_context) ma_context_uninit(&e->context);
        free(e);
        return NULL;
    }

    ma_engine_config ecfg = ma_engine_config_init();
    ecfg.pResourceManager = &e->resource_manager;
    if (e->has_context) ecfg.pContext = &e->context;
    if (ma_engine_init(&ecfg, &e->engine) != MA_SUCCESS) {
        ma_resource_manager_uninit(&e->resource_manager);
        if (e->has_context) ma_context_uninit(&e->context);
        free(e);
        return NULL;
    }

    // Tap node + its ring buffer, allocated once here (never on the audio thread).
    if (ld_tap_init(&e->engine, &e->tap) != MA_SUCCESS) {
        ma_engine_uninit(&e->engine);
        ma_resource_manager_uninit(&e->resource_manager);
        if (e->has_context) ma_context_uninit(&e->context);
        free(e);
        return NULL;
    }

    // Parent NULL = attach to the engine endpoint. Volume stays at the default
    // 1.0; master volume lives on the engine, not here.
    // The bus is a plain pass-through sub-mix: it only sums the track groups.
    // NO_SPATIALIZATION turns off miniaudio's 3D positioning/panning for it.
    // (Before, it was transparent only because it sat at distance 0 from the
    // listener; this makes that explicit instead of accidental.)
    if (ma_sound_group_init(&e->engine, MA_SOUND_FLAG_NO_SPATIALIZATION, NULL, &e->bus) != MA_SUCCESS) {
        ma_node_uninit(&e->tap, NULL);
        ma_pcm_rb_uninit(&e->tap.rb);
        ma_engine_uninit(&e->engine);
        ma_resource_manager_uninit(&e->resource_manager);
        if (e->has_context) ma_context_uninit(&e->context);
        free(e);
        return NULL;
    }

    // Splice the tap in: bus -> tap -> endpoint. A sound group IS a node, so the
    // cast is how miniaudio itself treats one. Attaching the bus's output bus
    // detaches it from the endpoint first (miniaudio.h 7.1).
    // A failed attach would leave track audio silently disconnected, so treat
    // it like any other init failure.
    if (ma_node_attach_output_bus(&e->tap, 0, ma_engine_get_endpoint(&e->engine), 0) != MA_SUCCESS ||
        ma_node_attach_output_bus((ma_node*)&e->bus, 0, &e->tap, 0) != MA_SUCCESS) {
        ma_sound_group_uninit(&e->bus);
        ma_node_uninit(&e->tap, NULL);
        ma_pcm_rb_uninit(&e->tap.rb);
        ma_engine_uninit(&e->engine);
        ma_resource_manager_uninit(&e->resource_manager);
        if (e->has_context) ma_context_uninit(&e->context);
        free(e);
        return NULL;
    }
    return e;
}

ld_engine* ld_engine_create(void) { return ld_engine_create_impl(0); }

/// An engine on miniaudio's null backend: same behaviour, no audio output.
ld_engine* ld_engine_create_silent(void) { return ld_engine_create_impl(1); }

void ld_engine_destroy(ld_engine* e) {
    if (e == NULL) return;
    // The bus and tap are nodes inside the engine's graph, so they go first,
    // upstream to downstream; the ring buffer only after the tap node that
    // writes into it is gone.
    ma_sound_group_uninit(&e->bus);
    ma_node_uninit(&e->tap, NULL);
    ma_pcm_rb_uninit(&e->tap.rb);
    ma_engine_uninit(&e->engine);
    ma_resource_manager_uninit(&e->resource_manager);
    if (e->has_context) ma_context_uninit(&e->context);
    free(e);
}

void ld_engine_set_volume(ld_engine* e, float linear) {
    if (e == NULL) return;
    ma_engine_set_volume(&e->engine, linear);
}

uint64_t ld_engine_time_frames(ld_engine* e) {
    if (e == NULL) return 0;
    return ma_engine_get_time_in_pcm_frames(&e->engine);
}

uint32_t ld_engine_sample_rate(ld_engine* e) {
    if (e == NULL) return 0;
    return ma_engine_get_sample_rate(&e->engine);
}

uint32_t ld_engine_channels(ld_engine* e) {
    if (e == NULL) return 0;
    return ma_engine_get_channels(&e->engine);
}

void ld_tap_set_enabled(ld_engine* e, int enabled) {
    if (e == NULL) return;
    ma_atomic_bool32_set(&e->tap.enabled, enabled ? MA_TRUE : MA_FALSE);
}

int64_t ld_tap_read(ld_engine* e, float* out, uint64_t max_frames) {
    if (e == NULL || out == NULL) return 0;
    ma_uint32 channels = e->tap.channels;
    uint64_t done = 0;
    // Like the write side, a read may straddle the wrap point, so loop until
    // max_frames are copied or the buffer has nothing more to give.
    while (done < max_frames) {
        uint64_t want = max_frames - done;
        ma_uint32 chunk = (want > 0xFFFFFFFFu) ? 0xFFFFFFFFu : (ma_uint32)want;
        void* src = NULL;
        if (ma_pcm_rb_acquire_read(&e->tap.rb, &chunk, &src) != MA_SUCCESS || chunk == 0) break;
        MA_COPY_MEMORY(out + done * channels, src, (size_t)chunk * channels * sizeof(float));
        if (ma_pcm_rb_commit_read(&e->tap.rb, chunk) != MA_SUCCESS) break;
        done += chunk;
    }
    return (int64_t)done;
}

ld_group* ld_group_create(ld_engine* e, int on_program_bus) {
    if (e == NULL) return NULL;
    ld_group* g = (ld_group*)calloc(1, sizeof(ld_group));
    if (g == NULL) return NULL;
    ma_sound_group* parent = on_program_bus ? &e->bus : NULL;
    if (ma_sound_group_init(&e->engine, 0, parent, &g->group) != MA_SUCCESS) {
        free(g);
        return NULL;
    }
    return g;
}

void ld_group_set_volume(ld_group* g, float linear) {
    if (g == NULL) return;
    ma_sound_group_set_volume(&g->group, linear);
}

void ld_group_destroy(ld_group* g) {
    if (g == NULL) return;
    ma_sound_group_uninit(&g->group);
    free(g);
}

ld_sound* ld_sound_create(ld_engine* e, ld_group* g, const wchar_t* path, int stream) {
    if (e == NULL || path == NULL) return NULL;
    ld_sound* s = (ld_sound*)calloc(1, sizeof(ld_sound));
    if (s == NULL) return NULL;
    s->owner = e;

    ma_uint32 flags = 0;
    if (stream != 0) flags |= MA_SOUND_FLAG_STREAM;

    ma_sound_group* pGroup = (g != NULL) ? &g->group : NULL;
    // Wide path -> ma_sound_init_from_file_w so Unicode paths open on Windows
    // (the narrow variant misinterprets UTF-8 bytes via the ANSI codepage).
    if (ma_sound_init_from_file_w(&e->engine, path, flags, pGroup, NULL, &s->sound) != MA_SUCCESS) {
        free(s);
        return NULL;
    }
    return s;
}

// --- De-click ramp floors (milliseconds) -----------------------------------
//
// Starting or stopping playback abruptly at a non-zero sample produces a step
// discontinuity in the waveform -> an audible click/pop. The old kira engine
// (the audio library this engine replaced) avoided this because every
// start/stop/pause/resume ramped through zero with a short tween. miniaudio
// does NOT ramp by default, so we enforce a minimum fade on every transition
// here. These are FLOORS, not replacements: a longer
// musical fade requested by the caller always wins (we take the max).
#define LD_MIN_FADE_MS         4    // start / seek / stop (de-click floor; ~190 samples @48k)
#define LD_MIN_PAUSE_FADE_MS   30   // pause / resume (slightly longer feels smoother)

static ma_uint64 ld_max_u64(ma_uint64 a, ma_uint64 b) { return a > b ? a : b; }

int ld_sound_play(ld_sound* s, double start_secs, double fade_in_secs) {
    if (s == NULL) return 1;
    if (start_secs > 0.0) {
        // Seek in the data source's own time base (handles native vs device rate).
        ma_sound_seek_to_second(&s->sound, (float)start_secs);
    }
    // Always ramp in from silence so a sound that begins mid-waveform (e.g. a
    // seek into the middle of a song, or a freshly recreated sound) does not cut
    // in at a non-zero sample. Floor at LD_MIN_FADE_MS; a longer requested
    // fade-in wins. volumeBeg=0, volumeEnd=1 are fader MULTIPLIERS of the
    // sound's own volume (set via ma_sound_set_volume), so this fades 0 -> the
    // sound's configured volume regardless of what that volume is.
    ma_uint64 ms = ld_max_u64((ma_uint64)(fade_in_secs * 1000.0), LD_MIN_FADE_MS);
    // Clear any stale scheduled stop / fade (harmless on a fresh sound) so a
    // re-started sound is not killed by a previously scheduled faded stop, then
    // apply our fade-in floor.
    ma_sound_reset_stop_time_and_fade(&s->sound);
    ma_sound_set_fade_in_milliseconds(&s->sound, 0.0f, 1.0f, ms);
    return (ma_sound_start(&s->sound) == MA_SUCCESS) ? 0 : 2;
}

void ld_sound_stop(ld_sound* s, double fade_out_secs) {
    if (s == NULL) return;
    // Always fade out before stopping (floor LD_MIN_FADE_MS) so we ramp to zero
    // instead of cutting off mid-waveform. A longer requested fade wins. This
    // covers immediate/"quick" stops too: the Rust side may pass 0.0, but we
    // never stop abruptly.
    ma_uint64 ms = ld_max_u64((ma_uint64)(fade_out_secs * 1000.0), LD_MIN_FADE_MS);
    ma_sound_stop_with_fade_in_milliseconds(&s->sound, ms);
}

void ld_sound_pause(ld_sound* s) {
    if (s == NULL) return;
    // Pause = short fade-out then stop, preserving the cursor. ma_sound_stop()
    // alone cuts abruptly -> click. stop_with_fade schedules the node stop after
    // the fade window, so the cursor is retained for resume.
    ma_sound_stop_with_fade_in_milliseconds(&s->sound, LD_MIN_PAUSE_FADE_MS);
}

void ld_sound_resume(ld_sound* s) {
    if (s == NULL) return;
    // A faded pause left a scheduled stop time and a (-1 -> 0) fade on the
    // sound. ma_sound_start does NOT clear those, so without this reset the
    // resumed sound would re-trigger the scheduled stop / stay silent. Clear
    // them, then ramp back in from 0 -> current volume over the pause floor.
    ma_sound_reset_stop_time_and_fade(&s->sound);
    ma_sound_set_fade_in_milliseconds(&s->sound, 0.0f, 1.0f, LD_MIN_PAUSE_FADE_MS);
    ma_sound_start(&s->sound);  // resumes from preserved cursor
}

void ld_sound_set_volume(ld_sound* s, float linear) {
    if (s == NULL) return;
    ma_sound_set_volume(&s->sound, linear);
}

double ld_sound_position_secs(ld_sound* s) {
    if (s == NULL) return 0.0;
    // Uses the data source's own sample rate, so it stays correct even when the
    // engine/device runs at a different rate than the file.
    float cursor = 0.0f;
    if (ma_sound_get_cursor_in_seconds(&s->sound, &cursor) != MA_SUCCESS) return 0.0;
    return (double)cursor;
}

double ld_sound_length_secs(ld_sound* s) {
    if (s == NULL) return 0.0;
    float length = 0.0f;
    if (ma_sound_get_length_in_seconds(&s->sound, &length) != MA_SUCCESS) return 0.0;
    return (double)length;
}

int ld_sound_is_playing(ld_sound* s) {
    if (s == NULL) return 0;
    return ma_sound_is_playing(&s->sound) ? 1 : 0;
}

int ld_sound_at_end(ld_sound* s) {
    if (s == NULL) return 0;
    return ma_sound_at_end(&s->sound) ? 1 : 0;
}

void ld_sound_schedule_start_frame(ld_sound* s, uint64_t engine_frame) {
    if (s == NULL) return;
    ma_sound_set_start_time_in_pcm_frames(&s->sound, engine_frame);
}

// Schedule a sample-accurate, faded stop `secs_from_now` seconds in the future
// (i.e. when the clip reaches its trim_end), ramping `fade_secs` to silence so
// it hits the stop point already at zero -> no overshoot, no click.
//
// The stop time is computed on the ENGINE/device clock (engine sample rate), the
// same clock ma_sound_is_playing / node state is evaluated against, so once it
// fires the sound reports not-playing and the Rust poll loop / transition state
// machine sees it as Stopped and advances normally. We floor the fade at
// LD_MIN_FADE_MS for de-click and clamp it to the lead so we never schedule the
// fade to start in the past. miniaudio's _with_fade applies a (-1 -> 0) fade,
// i.e. from the sound's CURRENT volume down to silence, ending exactly at the
// stop frame.
void ld_sound_schedule_stop_secs(ld_sound* s, double secs_from_now, double fade_secs) {
    if (s == NULL || s->owner == NULL) return;
    ma_uint32 sr = ma_engine_get_sample_rate(&s->owner->engine);
    if (sr == 0) return;
    if (secs_from_now < 0.0) secs_from_now = 0.0;

    ma_uint64 lead_frames = (ma_uint64)(secs_from_now * (double)sr);
    ma_uint64 now = ma_engine_get_time_in_pcm_frames(&s->owner->engine);
    ma_uint64 stop_frame = now + lead_frames;

    ma_uint64 fade_frames =
        ld_max_u64((ma_uint64)(fade_secs * (double)sr),
                   (LD_MIN_FADE_MS * (ma_uint64)sr) / 1000);
    // Never start the fade before the sound is even audible.
    if (fade_frames > lead_frames) fade_frames = lead_frames;

    ma_sound_set_stop_time_with_fade_in_pcm_frames(&s->sound, stop_frame, fade_frames);
}

void ld_sound_attach_to_group(ld_sound* s, ld_group* g) {
    if (s == NULL || s->owner == NULL) return;
    // g == NULL routes back to the engine endpoint (the group-less default a
    // preloaded sound was created with); the normal handoff always passes a group.
    ma_node* target = (g != NULL)
        ? (ma_node*)&g->group
        : ma_engine_get_endpoint(&s->owner->engine);
    // Re-parents output bus 0; ma_node_attach_output_bus detaches any prior
    // attachment first, so no explicit detach is needed (miniaudio.h 7.1).
    ma_node_attach_output_bus((ma_node*)&s->sound, 0, target, 0);
}

void ld_sound_seek_secs(ld_sound* s, double secs) {
    if (s == NULL) return;
    if (secs < 0.0) secs = 0.0;
    // Same time base as ld_sound_play's internal seek (ld_sound_play seeks only
    // when start_secs > 0, so a preloaded sound started with start_secs == 0
    // begins from THIS cursor without re-seeking).
    ma_sound_seek_to_second(&s->sound, (float)secs);
}

void ld_sound_destroy(ld_sound* s) {
    if (s == NULL) return;
    ma_sound_uninit(&s->sound);
    free(s);
}

// ---- Broadcast encoder ------------------------------------------------------
//
// Pipeline, run on the encoder thread by ld_encoder_pump (never on the audio
// thread, so it may allocate freely):
//
//   input frames (engine rate/channels, from the tap or ld_encoder_feed)
//     -> channel converter (to stereo; skipped if the input is already stereo)
//     -> resampler (to 48 kHz; skipped if the input is already 48 kHz)
//     -> staging FIFO of 48 kHz stereo frames
//     -> Opus encoder, one 960-frame (20 ms) block at a time -> packets
//     -> Ogg stream, packets wrapped into pages -> pending page queue -> out
//
// Why 48 kHz stereo: Opus works internally at 48 kHz, and every browser that
// plays Opus handles stereo, so one fixed output format keeps the stream simple
// regardless of what rate the sound card happens to run at.
//
// Why 20 ms frames: Opus encodes audio in fixed-size blocks ("frames"); 20 ms
// (960 samples per channel at 48 kHz) is its recommended default, the sweet
// spot between compression efficiency (longer is better) and latency (shorter
// is better). Leftover input shorter than a block waits in the staging FIFO
// for the next pump.
//
// What an Ogg page is: Ogg is a container -- a way of chopping a stream of
// codec packets into self-delimiting chunks called pages. Each page starts
// with the capture pattern "OggS", then the stream's serial number (constant
// for the whole stream), a page sequence number (up by one per page, so a gap
// reveals loss), a granule position (a timestamp, see below) and a CRC. A
// listener who connects mid-stream is sent the two header pages first, then
// can start at any audio page boundary. That is why pump only ever hands out
// whole pages.
//
// What granule position means: for Opus (RFC 7845), a page's granule position
// is the total number of 48 kHz samples (per channel) the encoder has output
// up to the end of the last packet that finishes on that page. That total
// INCLUDES the pre-skip warm-up samples, because they are real encoder output
// too. So the playback position of real audio is granule - pre-skip. Players
// use it to know where in time each page sits, which is what lets them seek
// and keep audio in sync.
//
// What pre-skip is: the Opus encoder has a small built-in delay ("lookahead",
// a few milliseconds): the first samples it outputs are warm-up, not real
// audio. OpusHead records that many samples as "pre-skip" so the decoder
// throws them away and playback starts exactly at the first real sample.

#include <stdio.h>
#include <string.h>
#include <time.h>
#include <opus.h>
#include <ogg/ogg.h>

#define LD_ENC_RATE        48000
#define LD_ENC_CHANNELS    2
#define LD_ENC_FRAME       960      // 20 ms at 48 kHz, per channel
#define LD_ENC_MAX_PACKET  4000     // bytes; a 20 ms packet at 128 kbps is ~320
#define LD_ENC_FLUSH_EVERY 2        // force a page every 2 packets (~40 ms)
#define LD_ENC_TAP_CHUNK   4096     // frames per ld_tap_read call while draining

// A growable buffer of interleaved f32 frames (malloc'd; encoder thread only).
typedef struct ld_fbuf {
    float* data;
    size_t frames;     // frames currently held
    size_t cap;        // frames of capacity
} ld_fbuf;

// Makes room for `extra` more frames of `channels` samples. 0 on success.
static int ld_fbuf_reserve(ld_fbuf* b, size_t extra, ma_uint32 channels) {
    size_t need = b->frames + extra;
    if (need <= b->cap) return 0;
    size_t cap = b->cap ? b->cap : 1024;
    while (cap < need) cap *= 2;
    float* p = (float*)realloc(b->data, cap * channels * sizeof(float));
    if (p == NULL) return -1;
    b->data = p;
    b->cap = cap;
    return 0;
}

struct ld_encoder {
    ld_engine* engine;          // tap source; NULL for a feed source
    ma_uint32 in_rate;
    ma_uint32 in_channels;
    int convert;                // in_channels != 2: converter initialized
    int resample;               // in_rate != 48000: resampler initialized
    ma_channel_converter converter;
    ma_resampler resampler;

    ld_fbuf input;              // raw frames at in_rate / in_channels
    ld_fbuf stereo;             // scratch: input converted to stereo, still at in_rate
    ld_fbuf stage;              // 48 kHz stereo frames waiting to be encoded

    OpusEncoder* opus;
    int pre_skip;               // encoder lookahead, in 48 kHz samples
    ogg_stream_state os;
    ogg_int64_t packetno;       // next Ogg packet number (0 and 1 are the headers)
    ogg_int64_t samples_out;    // 48 kHz samples per channel encoded so far
    int since_flush;            // audio packets since the last forced page

    uint8_t* header;            // OpusHead + OpusTags pages, built once
    size_t header_len;

    // Finished pages pump has not handed out yet: their bytes back to back,
    // plus each page's length, so pump can copy whole pages only.
    uint8_t* pend;
    size_t pend_len, pend_cap;
    size_t* page_lens;
    size_t n_pages, pages_cap;

    int warned_feed;            // ld_encoder_feed misuse already logged
};

static void ld_put_u16(uint8_t* p, ma_uint32 v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static void ld_put_u32(uint8_t* p, ma_uint32 v) {
    p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); p[2] = (uint8_t)(v >> 16); p[3] = (uint8_t)(v >> 24);
}

// Appends one finished Ogg page (header then body) to a growable byte buffer.
// 0 on success.
static int ld_append_page(uint8_t** buf, size_t* len, size_t* cap, const ogg_page* og) {
    size_t n = (size_t)og->header_len + (size_t)og->body_len;
    if (*len + n > *cap) {
        size_t c = *cap ? *cap : 8192;
        while (c < *len + n) c *= 2;
        uint8_t* p = (uint8_t*)realloc(*buf, c);
        if (p == NULL) return -1;
        *buf = p;
        *cap = c;
    }
    memcpy(*buf + *len, og->header, (size_t)og->header_len);
    memcpy(*buf + *len + og->header_len, og->body, (size_t)og->body_len);
    *len += n;
    return 0;
}

// Queues a page for pump to hand out, remembering its length.
static int ld_enc_queue_page(ld_encoder* enc, const ogg_page* og) {
    if (enc->n_pages == enc->pages_cap) {
        size_t c = enc->pages_cap ? enc->pages_cap * 2 : 64;
        size_t* p = (size_t*)realloc(enc->page_lens, c * sizeof(size_t));
        if (p == NULL) return -1;
        enc->page_lens = p;
        enc->pages_cap = c;
    }
    if (ld_append_page(&enc->pend, &enc->pend_len, &enc->pend_cap, og) != 0) return -1;
    enc->page_lens[enc->n_pages++] = (size_t)og->header_len + (size_t)og->body_len;
    return 0;
}

// Builds the two header packets and flushes each onto its own page (RFC 7845:
// OpusHead alone on the first page, OpusTags starting the second).
static int ld_enc_build_header(ld_encoder* enc) {
    // OpusHead, 19 bytes, integers little-endian:
    //   0  "OpusHead"            magic
    //   8  version         u8    always 1
    //   9  channels        u8    2 (output is always stereo)
    //   10 pre-skip        u16   48 kHz samples the decoder must discard
    //   12 input rate      u32   informational only; 48000
    //   16 output gain     i16   0 dB
    //   18 mapping family  u8    0 = mono/stereo, no channel mapping table
    uint8_t head[19];
    memcpy(head, "OpusHead", 8);
    head[8] = 1;
    head[9] = LD_ENC_CHANNELS;
    ld_put_u16(head + 10, (ma_uint32)enc->pre_skip);
    ld_put_u32(head + 12, LD_ENC_RATE);
    ld_put_u16(head + 16, 0);
    head[18] = 0;

    // OpusTags: "OpusTags", vendor length u32 + vendor string, comment count u32.
    static const char vendor[] = "LuteDrop";
    uint8_t tags[8 + 4 + (sizeof(vendor) - 1) + 4];
    memcpy(tags, "OpusTags", 8);
    ld_put_u32(tags + 8, (ma_uint32)(sizeof(vendor) - 1));
    memcpy(tags + 12, vendor, sizeof(vendor) - 1);
    ld_put_u32(tags + 12 + (sizeof(vendor) - 1), 0);

    size_t cap = 0;
    ogg_page og;
    ogg_packet op;

    memset(&op, 0, sizeof(op));
    op.packet = head;
    op.bytes = (long)sizeof(head);
    op.b_o_s = 1;               // beginning of stream: flags the first page
    op.granulepos = 0;
    op.packetno = enc->packetno++;
    if (ogg_stream_packetin(&enc->os, &op) != 0) return -1;
    while (ogg_stream_flush(&enc->os, &og) != 0) {
        if (ld_append_page(&enc->header, &enc->header_len, &cap, &og) != 0) return -1;
    }

    memset(&op, 0, sizeof(op));
    op.packet = tags;
    op.bytes = (long)sizeof(tags);
    op.granulepos = 0;
    op.packetno = enc->packetno++;
    if (ogg_stream_packetin(&enc->os, &op) != 0) return -1;
    while (ogg_stream_flush(&enc->os, &og) != 0) {
        if (ld_append_page(&enc->header, &enc->header_len, &cap, &og) != 0) return -1;
    }
    return 0;
}

void ld_encoder_destroy(ld_encoder* enc) {
    if (enc == NULL) return;
    if (enc->engine != NULL) ld_tap_set_enabled(enc->engine, 0);
    if (enc->convert) ma_channel_converter_uninit(&enc->converter, NULL);
    if (enc->resample) ma_resampler_uninit(&enc->resampler, NULL);
    if (enc->opus != NULL) opus_encoder_destroy(enc->opus);
    ogg_stream_clear(&enc->os);  // safe on a zeroed, never-initialized state
    free(enc->input.data);
    free(enc->stereo.data);
    free(enc->stage.data);
    free(enc->header);
    free(enc->pend);
    free(enc->page_lens);
    free(enc);
}

// Setup shared by both sources. Does not touch the tap.
static ld_encoder* ld_encoder_new(uint32_t in_rate, uint32_t in_channels, uint32_t bitrate_bps) {
    if (in_rate == 0 || in_channels == 0 || in_channels > MA_MAX_CHANNELS) return NULL;
    ld_encoder* enc = (ld_encoder*)calloc(1, sizeof(ld_encoder));
    if (enc == NULL) return NULL;
    enc->in_rate = in_rate;
    enc->in_channels = in_channels;

    int err = 0;
    opus_int32 lookahead = 0;
    int serial = 0;

    if (in_channels != LD_ENC_CHANNELS) {
        // Rectangular mixing: each output channel averages the inputs on its
        // side of the speaker layout (mono simply goes to both left and right).
        ma_channel_converter_config cc = ma_channel_converter_config_init(
            ma_format_f32, in_channels, NULL, LD_ENC_CHANNELS, NULL, ma_channel_mix_mode_rectangular);
        if (ma_channel_converter_init(&cc, NULL, &enc->converter) != MA_SUCCESS) goto fail;
        enc->convert = 1;
    }
    if (in_rate != LD_ENC_RATE) {
        // Linear resampling: cheap, and ample for a listen-along stream. Runs
        // after channel conversion, so it is always 2 channels.
        ma_resampler_config rc = ma_resampler_config_init(
            ma_format_f32, LD_ENC_CHANNELS, in_rate, LD_ENC_RATE, ma_resample_algorithm_linear);
        if (ma_resampler_init(&rc, NULL, &enc->resampler) != MA_SUCCESS) goto fail;
        enc->resample = 1;
    }

    enc->opus = opus_encoder_create(LD_ENC_RATE, LD_ENC_CHANNELS, OPUS_APPLICATION_AUDIO, &err);
    if (enc->opus == NULL || err != OPUS_OK) goto fail;
    // VBR is libopus's default; only the target bitrate needs setting.
    if (opus_encoder_ctl(enc->opus, OPUS_SET_BITRATE((opus_int32)bitrate_bps)) != OPUS_OK) goto fail;
    if (opus_encoder_ctl(enc->opus, OPUS_GET_LOOKAHEAD(&lookahead)) != OPUS_OK) goto fail;
    enc->pre_skip = (int)lookahead;

    // Any nonzero serial works; it only has to stay the same for the whole
    // stream. Mixing the clock with this allocation's address makes two
    // encoders created back to back differ.
    serial = (int)((ma_uint32)time(NULL) ^ (ma_uint32)(uintptr_t)enc);
    if (serial == 0) serial = 1;
    if (ogg_stream_init(&enc->os, serial) != 0) goto fail;

    if (ld_enc_build_header(enc) != 0) goto fail;
    return enc;

fail:
    ld_encoder_destroy(enc);
    return NULL;
}

// Threading contract: the tap's ring buffer has a single reader, and this
// function already reads it (to discard stale frames). Pump on this same
// thread, or hand the encoder to the pumping thread by spawning that thread
// with it (a thread spawn orders this create before every later pump), so the
// two never read the tap at the same time. See shim.h.
ld_encoder* ld_encoder_create(ld_engine* e, uint32_t bitrate_bps) {
    if (e == NULL) return NULL;
    ld_encoder* enc = ld_encoder_new(ld_engine_sample_rate(e), ld_engine_channels(e), bitrate_bps);
    if (enc == NULL) return NULL;
    if (ld_fbuf_reserve(&enc->input, LD_ENC_TAP_CHUNK, enc->in_channels) != 0) {
        ld_encoder_destroy(enc);
        return NULL;
    }
    enc->engine = e;
    ld_tap_set_enabled(e, 1);

    // The tap keeps up to LD_TAP_SECONDS of frames across disable/enable, so a
    // previous broadcast's tail may still be queued. Throw it away so this
    // stream starts with live audio.
    while (ld_tap_read(e, enc->input.data, LD_ENC_TAP_CHUNK) > 0) {}
    return enc;
}

ld_encoder* ld_encoder_create_detached(uint32_t in_rate, uint32_t in_channels, uint32_t bitrate_bps) {
    return ld_encoder_new(in_rate, in_channels, bitrate_bps);
}

size_t ld_encoder_header(ld_encoder* enc, const uint8_t** out) {
    if (enc == NULL || out == NULL) return 0;
    *out = enc->header;
    return enc->header_len;
}

void ld_encoder_feed(ld_encoder* enc, const float* frames, uint64_t count) {
    if (enc == NULL || frames == NULL || count == 0) return;
    if (enc->engine != NULL) {
        if (!enc->warned_feed) {
            fprintf(stderr, "ld_encoder_feed: ignored on a tap-source encoder\n");
            enc->warned_feed = 1;
        }
        return;
    }
    if (ld_fbuf_reserve(&enc->input, (size_t)count, enc->in_channels) != 0) return;
    memcpy(enc->input.data + enc->input.frames * enc->in_channels, frames,
           (size_t)count * enc->in_channels * sizeof(float));
    enc->input.frames += (size_t)count;
}

// Moves everything queued in the tap into the input buffer (tap source only).
static int ld_enc_drain_tap(ld_encoder* enc) {
    for (;;) {
        if (ld_fbuf_reserve(&enc->input, LD_ENC_TAP_CHUNK, enc->in_channels) != 0) return -1;
        int64_t n = ld_tap_read(enc->engine, enc->input.data + enc->input.frames * enc->in_channels,
                                LD_ENC_TAP_CHUNK);
        if (n <= 0) return 0;
        enc->input.frames += (size_t)n;
    }
}

// input (in_rate, in_channels) -> stage (48 kHz stereo). Empties input.
static int ld_enc_convert_input(ld_encoder* enc) {
    size_t frames = enc->input.frames;
    if (frames == 0) return 0;

    // Step 1: to stereo, still at the input rate.
    const float* stereo = enc->input.data;
    if (enc->convert) {
        enc->stereo.frames = 0;
        if (ld_fbuf_reserve(&enc->stereo, frames, LD_ENC_CHANNELS) != 0) return -1;
        if (ma_channel_converter_process_pcm_frames(&enc->converter, enc->stereo.data,
                                                    enc->input.data, frames) != MA_SUCCESS) {
            return -1;
        }
        stereo = enc->stereo.data;
    }

    // Step 2: to 48 kHz, appended to the staging FIFO.
    if (!enc->resample) {
        if (ld_fbuf_reserve(&enc->stage, frames, LD_ENC_CHANNELS) != 0) return -1;
        memcpy(enc->stage.data + enc->stage.frames * LD_ENC_CHANNELS, stereo,
               frames * LD_ENC_CHANNELS * sizeof(float));
        enc->stage.frames += frames;
    } else {
        // The resampler keeps a little history internally, so it may consume
        // all the input yet emit slightly fewer frames than the ratio implies;
        // the rest comes out on the next call. Loop until the input is used up.
        size_t done = 0;
        while (done < frames) {
            ma_uint64 in_count = frames - done;
            ma_uint64 expect = 0;
            ma_resampler_get_expected_output_frame_count(&enc->resampler, in_count, &expect);
            size_t room = (size_t)expect + 16;
            if (ld_fbuf_reserve(&enc->stage, room, LD_ENC_CHANNELS) != 0) return -1;
            ma_uint64 out_count = room;
            if (ma_resampler_process_pcm_frames(&enc->resampler,
                    stereo + done * LD_ENC_CHANNELS, &in_count,
                    enc->stage.data + enc->stage.frames * LD_ENC_CHANNELS, &out_count) != MA_SUCCESS) {
                return -1;
            }
            enc->stage.frames += (size_t)out_count;
            done += (size_t)in_count;
            if (in_count == 0 && out_count == 0) break;   // no progress: never spin
        }
    }
    enc->input.frames = 0;
    return 0;
}

// Encodes every complete 20 ms block in the staging FIFO and queues the
// resulting Ogg pages.
static int ld_enc_encode_staged(ld_encoder* enc) {
    unsigned char packet[LD_ENC_MAX_PACKET];
    size_t off = 0;
    int rc = 0;
    while (rc == 0 && enc->stage.frames - off >= LD_ENC_FRAME) {
        opus_int32 n = opus_encode_float(enc->opus, enc->stage.data + off * LD_ENC_CHANNELS,
                                         LD_ENC_FRAME, packet, LD_ENC_MAX_PACKET);
        off += LD_ENC_FRAME;
        if (n < 0) { rc = -1; break; }
        enc->samples_out += LD_ENC_FRAME;

        ogg_packet op;
        memset(&op, 0, sizeof(op));
        op.packet = packet;
        op.bytes = n;
        // Granule = every sample the encoder has output so far, this packet
        // included. samples_out already counts the pre-skip warm-up (it is
        // part of the encoder's output), so nothing is added here; a player
        // finds the real-audio position as granule - pre-skip.
        op.granulepos = enc->samples_out;
        op.packetno = enc->packetno++;
        // e_o_s stays 0: a live stream never ends.
        if (ogg_stream_packetin(&enc->os, &op) != 0) { rc = -1; break; }

        // pageout alone only emits a page once ~4 KB has built up, which is
        // hundreds of ms of audio. Forcing a flush every 2nd packet caps each
        // page at ~40 ms. Each page costs ~28 bytes of header against ~640
        // bytes of audio (about 4 %), a fair price: the player's buffer has
        // to cover at least one page interval, so shorter pages let it be
        // smaller and the listener hears things sooner.
        ogg_page og;
        if (++enc->since_flush >= LD_ENC_FLUSH_EVERY) {
            enc->since_flush = 0;
            while (rc == 0 && ogg_stream_flush(&enc->os, &og) != 0) rc = ld_enc_queue_page(enc, &og);
        } else {
            while (rc == 0 && ogg_stream_pageout(&enc->os, &og) != 0) rc = ld_enc_queue_page(enc, &og);
        }
    }
    // Shift the leftover partial block to the front of the FIFO.
    size_t left = enc->stage.frames - off;
    if (off > 0 && left > 0) {
        memmove(enc->stage.data, enc->stage.data + off * LD_ENC_CHANNELS,
                left * LD_ENC_CHANNELS * sizeof(float));
    }
    enc->stage.frames = left;
    return rc;
}

size_t ld_encoder_pump(ld_encoder* enc, uint8_t* out, size_t cap) {
    if (enc == NULL || out == NULL) return 0;
    // A failure here (allocation or encoder error) only means fewer new pages
    // this round; pages already queued are still handed out below.
    if (enc->engine == NULL || ld_enc_drain_tap(enc) == 0) {
        if (ld_enc_convert_input(enc) == 0) ld_enc_encode_staged(enc);
    }

    // Copy whole pages from the front of the pending queue while they fit.
    size_t written = 0, taken = 0;
    while (taken < enc->n_pages && written + enc->page_lens[taken] <= cap) {
        written += enc->page_lens[taken];
        taken++;
    }
    if (written == 0) return 0;
    memcpy(out, enc->pend, written);
    memmove(enc->pend, enc->pend + written, enc->pend_len - written);
    enc->pend_len -= written;
    memmove(enc->page_lens, enc->page_lens + taken, (enc->n_pages - taken) * sizeof(size_t));
    enc->n_pages -= taken;
    return written;
}
