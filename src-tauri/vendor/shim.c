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
static ma_result ld_make_decoder(const char* path, ma_decoder* dec) {
    ma_decoding_backend_vtable* backends[] = { ma_decoding_backend_libopus };
    ma_decoder_config cfg = ma_decoder_config_init(ma_format_f32, 0, 0);
    cfg.ppCustomBackendVTables = backends;
    cfg.customBackendCount = 1;
    return ma_decoder_init_file(path, &cfg, dec);
}

int ld_decode_file(const char* path, float** out, uint64_t* out_frames,
                   uint32_t* out_channels, uint32_t* out_sample_rate) {
    ma_decoder dec;
    if (ld_make_decoder(path, &dec) != MA_SUCCESS) return 1;
    *out_channels = dec.outputChannels;
    *out_sample_rate = dec.outputSampleRate;

    ma_uint64 total = 0;
    // Best-effort: may be 0/unknown for some streamed sources; we grow the
    // buffer dynamically below regardless.
    ma_decoder_get_length_in_pcm_frames(&dec, &total);

    size_t cap_frames = total ? (size_t)total : 48000;
    size_t cap = cap_frames * dec.outputChannels;
    if (cap == 0) cap = 4096;
    float* buf = (float*)malloc(cap * sizeof(float));
    if (buf == NULL) { ma_decoder_uninit(&dec); return 2; }

    ma_uint64 framesRead, totalRead = 0;
    for (;;) {
        // Ensure room for at least one more 4096-frame chunk.
        if ((totalRead + 4096) * dec.outputChannels > cap) {
            cap *= 2;
            float* grown = (float*)realloc(buf, cap * sizeof(float));
            if (grown == NULL) { free(buf); ma_decoder_uninit(&dec); return 3; }
            buf = grown;
        }
        ma_result r = ma_decoder_read_pcm_frames(
            &dec, buf + totalRead * dec.outputChannels, 4096, &framesRead);
        totalRead += framesRead;
        if ((r != MA_SUCCESS && r != MA_AT_END) || framesRead == 0) break;
    }

    ma_decoder_uninit(&dec);
    *out = buf;
    *out_frames = totalRead;
    return 0;
}

void ld_free_decoded(float* p) { free(p); }

double ld_file_duration_secs(const char* path) {
    ma_decoder dec;
    if (ld_make_decoder(path, &dec) != MA_SUCCESS) return -1.0;
    ma_uint64 total = 0;
    ma_result r = ma_decoder_get_length_in_pcm_frames(&dec, &total);
    ma_uint32 sr = dec.outputSampleRate;
    ma_decoder_uninit(&dec);
    if (r != MA_SUCCESS || total == 0 || sr == 0) return -1.0;
    return (double)total / (double)sr;
}

// ---- Playback layer (Task 2) ------------------------------------------------

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

ld_sound* ld_sound_create(ld_engine* e, ld_group* g, const char* path, int stream) {
    if (e == NULL || path == NULL) return NULL;
    ld_sound* s = (ld_sound*)calloc(1, sizeof(ld_sound));
    if (s == NULL) return NULL;
    s->owner = e;

    ma_uint32 flags = 0;
    if (stream != 0) flags |= MA_SOUND_FLAG_STREAM;

    ma_sound_group* pGroup = (g != NULL) ? &g->group : NULL;
    if (ma_sound_init_from_file(&e->engine, path, flags, pGroup, NULL, &s->sound) != MA_SUCCESS) {
        free(s);
        return NULL;
    }
    return s;
}

int ld_sound_play(ld_sound* s, double start_secs, double fade_in_secs) {
    if (s == NULL) return 1;
    if (start_secs > 0.0) {
        // Seek in the data source's own time base (handles native vs device rate).
        ma_sound_seek_to_second(&s->sound, (float)start_secs);
    }
    if (fade_in_secs > 0.0) {
        // Fade from current (volumeBeg = -1) up to full over the given time.
        ma_sound_set_fade_in_milliseconds(&s->sound, 0.0f, 1.0f,
                                          (ma_uint64)(fade_in_secs * 1000.0));
    }
    return (ma_sound_start(&s->sound) == MA_SUCCESS) ? 0 : 2;
}

void ld_sound_stop(ld_sound* s, double fade_out_secs) {
    if (s == NULL) return;
    if (fade_out_secs > 0.0) {
        ma_uint32 sr = ma_engine_get_sample_rate(&s->owner->engine);
        ma_uint64 frames = (ma_uint64)(fade_out_secs * (double)sr);
        ma_sound_stop_with_fade_in_pcm_frames(&s->sound, frames);
    } else {
        ma_sound_stop(&s->sound);
    }
}

void ld_sound_pause(ld_sound* s) {
    if (s == NULL) return;
    ma_sound_stop(&s->sound);   // keeps cursor
}

void ld_sound_resume(ld_sound* s) {
    if (s == NULL) return;
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

void ld_sound_destroy(ld_sound* s) {
    if (s == NULL) return;
    ma_sound_uninit(&s->sound);
    free(s);
}
