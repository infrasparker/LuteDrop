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
