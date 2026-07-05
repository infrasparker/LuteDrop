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

// ---- Playback layer ---------------------------------------------------------

struct ld_engine {
    ma_engine engine;
    ma_resource_manager resource_manager;
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
    return e;
}

void ld_engine_destroy(ld_engine* e) {
    if (e == NULL) return;
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

ld_group* ld_group_create(ld_engine* e) {
    if (e == NULL) return NULL;
    ld_group* g = (ld_group*)calloc(1, sizeof(ld_group));
    if (g == NULL) return NULL;
    if (ma_sound_group_init(&e->engine, 0, NULL, &g->group) != MA_SUCCESS) {
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

void ld_sound_destroy(ld_sound* s) {
    if (s == NULL) return;
    ma_sound_uninit(&s->sound);
    free(s);
}
