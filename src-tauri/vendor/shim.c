#define MINIAUDIO_IMPLEMENTATION
#define MA_NO_GENERATION
#include "miniaudio.h"
#include "miniaudio_libopus.h"
#include "miniaudio_libopus.c"
#include "shim.h"
#include <stdlib.h>

// Registers the libopus backend so .opus decodes alongside built-ins.
//
// NOTE: `ma_decoding_backend_libopus` is declared in miniaudio_libopus.h as
// `extern ma_decoding_backend_vtable*` -- it is ALREADY a pointer, so the
// backends array holds it directly (NOT its address).
// Path is a wide (UTF-16) string and we call miniaudio's `_w` file APIs. On
// Windows, miniaudio's narrow file APIs route the path through fopen, which
// interprets the bytes in the ANSI/locale codepage rather than UTF-8 -- so any
// non-ASCII UTF-8 path fails to open. The wide APIs go straight to _wfopen, so
// Unicode paths (smart quotes, accents, CJK, etc.) open correctly.
static ma_result ld_make_decoder(const wchar_t* path, ma_decoder* dec) {
    ma_decoding_backend_vtable* backends[] = { ma_decoding_backend_libopus };
    ma_decoder_config cfg = ma_decoder_config_init(ma_format_f32, 0, 0);
    cfg.ppCustomBackendVTables = backends;
    cfg.customBackendCount = 1;
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
    // The "program bus": a sound group every track's group feeds into. Think of
    // it as a mixing-desk submaster: all tracks are summed here before reaching
    // the speakers. It is parented to the endpoint, so what it carries still
    // gets master volume (applied by the engine) afterwards. A later change
    // taps this bus to broadcast the mix.
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

ld_engine* ld_engine_create(void) {
    ld_engine* e = (ld_engine*)calloc(1, sizeof(ld_engine));
    if (e == NULL) return NULL;

    // Register the libopus backend on the resource manager so streamed .opus
    // decodes natively. `ma_decoding_backend_libopus` is ALREADY a
    // ma_decoding_backend_vtable* (a pointer) -- store it directly (NO &).
    ma_decoding_backend_vtable* backends[] = { ma_decoding_backend_libopus };

    ma_resource_manager_config rmcfg = ma_resource_manager_config_init();
    rmcfg.decodedFormat = ma_format_f32;
    rmcfg.ppCustomDecodingBackendVTables = backends;
    rmcfg.customDecodingBackendCount = 1;
    if (ma_resource_manager_init(&rmcfg, &e->resource_manager) != MA_SUCCESS) {
        free(e);
        return NULL;
    }

    ma_engine_config ecfg = ma_engine_config_init();
    ecfg.pResourceManager = &e->resource_manager;
    if (ma_engine_init(&ecfg, &e->engine) != MA_SUCCESS) {
        ma_resource_manager_uninit(&e->resource_manager);
        free(e);
        return NULL;
    }

    // Tap node + its ring buffer, allocated once here (never on the audio thread).
    if (ld_tap_init(&e->engine, &e->tap) != MA_SUCCESS) {
        ma_engine_uninit(&e->engine);
        ma_resource_manager_uninit(&e->resource_manager);
        free(e);
        return NULL;
    }

    // Parent NULL = attach to the engine endpoint. Volume stays at the default
    // 1.0; master volume lives on the engine, not here.
    if (ma_sound_group_init(&e->engine, 0, NULL, &e->bus) != MA_SUCCESS) {
        ma_node_uninit(&e->tap, NULL);
        ma_pcm_rb_uninit(&e->tap.rb);
        ma_engine_uninit(&e->engine);
        ma_resource_manager_uninit(&e->resource_manager);
        free(e);
        return NULL;
    }

    // Splice the tap in: bus -> tap -> endpoint. A sound group IS a node, so the
    // cast is how miniaudio itself treats one. Attaching the bus's output bus
    // detaches it from the endpoint first (miniaudio.h 7.1).
    ma_node_attach_output_bus(&e->tap, 0, ma_engine_get_endpoint(&e->engine), 0);
    ma_node_attach_output_bus((ma_node*)&e->bus, 0, &e->tap, 0);
    return e;
}

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
