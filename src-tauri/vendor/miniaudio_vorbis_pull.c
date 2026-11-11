/* See miniaudio_vorbis_pull.h for why this exists alongside miniaudio's own stb_vorbis backend. */
#ifndef miniaudio_vorbis_pull_c
#define miniaudio_vorbis_pull_c

#include "miniaudio_vorbis_pull.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct
{
    ma_data_source_base ds;     /* MUST be first: miniaudio treats ld_vorbis* as ma_data_source*. */
    stb_vorbis* stb;
    unsigned char* data;        /* Whole compressed file (onInit path only); stb_vorbis reads from it. */
    ma_uint32 channels;
    ma_uint32 sampleRate;
    ma_uint64 cursor;           /* Next frame read() will return. */
    ma_uint64 length;           /* Total frames, from the last Ogg page's granule position. */
    ma_bool32 broken;           /* A failed seek left the position unknown: reads return nothing. */
} ld_vorbis;

static ma_result ld_vorbis_read(ma_data_source* pDataSource, void* pFramesOut, ma_uint64 frameCount, ma_uint64* pFramesRead)
{
    ld_vorbis* v = (ld_vorbis*)pDataSource;
    float* out = (float*)pFramesOut;
    ma_uint64 done = 0;

    if (v->broken) {
        if (pFramesRead != NULL) *pFramesRead = 0;
        return MA_AT_END;
    }
    /* stb_vorbis takes an int sample count, so read in bounded chunks. */
    while (done < frameCount) {
        ma_uint64 want = frameCount - done;
        if (want > 65536) want = 65536;
        int got = stb_vorbis_get_samples_float_interleaved(v->stb, (int)v->channels,
                      out + done * v->channels, (int)(want * v->channels));
        if (got <= 0) break;
        done += (ma_uint64)got;
    }

    v->cursor += done;
    if (pFramesRead != NULL) *pFramesRead = done;
    /* miniaudio's convention: a short read is fine, zero frames means the end. */
    return (done == 0 && frameCount > 0) ? MA_AT_END : MA_SUCCESS;
}

static ma_result ld_vorbis_seek(ma_data_source* pDataSource, ma_uint64 frameIndex)
{
    ld_vorbis* v = (ld_vorbis*)pDataSource;
    if (frameIndex > 0x7FFFFFFF) return MA_INVALID_ARGS;   /* stb_vorbis positions are 32-bit. */
    /* Pull-mode stb_vorbis_seek finds the page holding the target through the
       Ogg granule positions, then decodes and discards up to the exact sample. */
    if (!stb_vorbis_seek(v->stb, (unsigned int)frameIndex)) {
        /* A failed seek can leave the decoder anywhere. Go back to the start
           so the cursor matches what read() returns next; if even that fails,
           end the stream rather than hand out audio from an unknown place. */
        if (stb_vorbis_seek_start(v->stb)) {
            v->cursor = 0;
        } else {
            v->cursor = v->length;
            v->broken = MA_TRUE;
        }
        return MA_ERROR;
    }
    v->cursor = frameIndex;
    v->broken = MA_FALSE;
    return MA_SUCCESS;
}

static ma_result ld_vorbis_get_data_format(ma_data_source* pDataSource, ma_format* pFormat, ma_uint32* pChannels, ma_uint32* pSampleRate, ma_channel* pChannelMap, size_t channelMapCap)
{
    ld_vorbis* v = (ld_vorbis*)pDataSource;
    if (pFormat != NULL) *pFormat = ma_format_f32;     /* stb_vorbis decodes to float. */
    if (pChannels != NULL) *pChannels = v->channels;
    if (pSampleRate != NULL) *pSampleRate = v->sampleRate;
    /* Vorbis orders channels differently from WAV once there are more than two. */
    if (pChannelMap != NULL) ma_channel_map_init_standard(ma_standard_channel_map_vorbis, pChannelMap, channelMapCap, v->channels);
    return MA_SUCCESS;
}

static ma_result ld_vorbis_get_cursor(ma_data_source* pDataSource, ma_uint64* pCursor)
{
    *pCursor = ((ld_vorbis*)pDataSource)->cursor;
    return MA_SUCCESS;
}

static ma_result ld_vorbis_get_length(ma_data_source* pDataSource, ma_uint64* pLength)
{
    *pLength = ((ld_vorbis*)pDataSource)->length;
    return MA_SUCCESS;
}

static ma_data_source_vtable g_ld_vorbis_ds_vtable =
{
    ld_vorbis_read,
    ld_vorbis_seek,
    ld_vorbis_get_data_format,
    ld_vorbis_get_cursor,
    ld_vorbis_get_length,
    NULL,   /* onSetLooping */
    0       /* flags */
};

/* Wraps an opened stb_vorbis decoder in a heap-allocated data source.
   Takes ownership of `stb` and `data` (freed on failure). */
static ma_result ld_vorbis_wrap(stb_vorbis* stb, unsigned char* data, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    ld_vorbis* v = (ld_vorbis*)ma_malloc(sizeof(ld_vorbis), pAlloc);
    if (v == NULL) {
        stb_vorbis_close(stb);
        ma_free(data, pAlloc);
        return MA_OUT_OF_MEMORY;
    }
    memset(v, 0, sizeof(*v));

    stb_vorbis_info info = stb_vorbis_get_info(stb);
    v->stb = stb;
    v->data = data;
    v->channels = (ma_uint32)info.channels;
    v->sampleRate = info.sample_rate;
    v->length = stb_vorbis_stream_length_in_samples(stb);

    ma_data_source_config cfg = ma_data_source_config_init();
    cfg.vtable = &g_ld_vorbis_ds_vtable;
    ma_result r = ma_data_source_init(&cfg, &v->ds);
    if (r != MA_SUCCESS || v->channels == 0 || v->sampleRate == 0) {
        stb_vorbis_close(stb);
        ma_free(data, pAlloc);
        ma_free(v, pAlloc);
        return (r != MA_SUCCESS) ? r : MA_INVALID_FILE;
    }
    *ppBackend = (ma_data_source*)v;
    return MA_SUCCESS;
}

/* Reads exactly n bytes, or fails. */
static ma_bool32 ld_vorbis_read_exact(ma_read_proc onRead, void* ud, void* buf, size_t n)
{
    size_t got = 0;
    return onRead(ud, buf, n, &got) == MA_SUCCESS && got == n;
}

/* True if the stream starts with an Ogg page whose first packet is a Vorbis
   identification header ("\x01vorbis"). Checked before reading the whole file
   into memory: in the resource manager every file is offered to the custom
   backends first, so without this a large WAV or an Opus file would be loaded
   in full only to be rejected. */
static ma_bool32 ld_vorbis_sniff(ma_read_proc onRead, void* ud)
{
    unsigned char page[27 + 255];
    unsigned char packet[7];
    if (!ld_vorbis_read_exact(onRead, ud, page, 27)) return MA_FALSE;
    if (memcmp(page, "OggS", 4) != 0) return MA_FALSE;
    /* Byte 26 is the segment count; the segment table follows, then the body. */
    if (!ld_vorbis_read_exact(onRead, ud, page + 27, page[26])) return MA_FALSE;
    if (!ld_vorbis_read_exact(onRead, ud, packet, 7)) return MA_FALSE;
    return memcmp(packet, "\x01vorbis", 7) == 0;
}

static ma_result ld_vorbis_backend_init(void* pUserData, ma_read_proc onRead, ma_seek_proc onSeek, ma_tell_proc onTell, void* pReadSeekTellUserData, const ma_decoding_backend_config* pConfig, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    (void)pUserData; (void)onTell; (void)pConfig;
    if (onRead == NULL || onSeek == NULL) return MA_INVALID_ARGS;

    if (!ld_vorbis_sniff(onRead, pReadSeekTellUserData)) return MA_INVALID_FILE;
    if (onSeek(pReadSeekTellUserData, 0, ma_seek_origin_start) != MA_SUCCESS) return MA_ERROR;

    /* Read to the end in growing chunks (no reliance on the stream knowing its size). */
    size_t cap = 1 << 20, len = 0;
    unsigned char* data = (unsigned char*)ma_malloc(cap, pAlloc);
    if (data == NULL) return MA_OUT_OF_MEMORY;
    for (;;) {
        if (len == cap) {
            if (cap > 0x3FFFFFFF) { ma_free(data, pAlloc); return MA_TOO_BIG; }  /* stb_vorbis lengths are int. */
            unsigned char* grown = (unsigned char*)ma_realloc(data, cap * 2, pAlloc);
            if (grown == NULL) { ma_free(data, pAlloc); return MA_OUT_OF_MEMORY; }
            data = grown;
            cap *= 2;
        }
        size_t got = 0;
        ma_result r = onRead(pReadSeekTellUserData, data + len, cap - len, &got);
        len += got;
        if (got == 0 || (r != MA_SUCCESS && r != MA_AT_END)) break;
    }

    stb_vorbis* stb = stb_vorbis_open_memory(data, (int)len, NULL, NULL);
    if (stb == NULL) {
        ma_free(data, pAlloc);
        return MA_INVALID_FILE;
    }
    return ld_vorbis_wrap(stb, data, pAlloc, ppBackend);
}

static ma_result ld_vorbis_backend_init_file_w(void* pUserData, const wchar_t* pFilePath, const ma_decoding_backend_config* pConfig, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    (void)pUserData; (void)pConfig;
    FILE* f = NULL;
    if (_wfopen_s(&f, pFilePath, L"rb") != 0 || f == NULL) return MA_DOES_NOT_EXIST;
    /* close_on_free = 1: stb_vorbis closes the FILE on stb_vorbis_close, and
       also when open fails, so it is never closed here. */
    stb_vorbis* stb = stb_vorbis_open_file(f, 1, NULL, NULL);
    if (stb == NULL) return MA_INVALID_FILE;
    return ld_vorbis_wrap(stb, NULL, pAlloc, ppBackend);
}

static void ld_vorbis_backend_uninit(void* pUserData, ma_data_source* pBackend, const ma_allocation_callbacks* pAlloc)
{
    (void)pUserData;
    ld_vorbis* v = (ld_vorbis*)pBackend;
    stb_vorbis_close(v->stb);           /* Closes the FILE too, on the file path. */
    ma_free(v->data, pAlloc);           /* NULL on the file path. */
    ma_data_source_uninit(&v->ds);
    ma_free(v, pAlloc);
}

static ma_decoding_backend_vtable g_ld_vorbis_backend_vtable =
{
    ld_vorbis_backend_init,
    NULL,   /* onInitFile: only wide paths are used (see shim.c) */
    ld_vorbis_backend_init_file_w,
    NULL,   /* onInitMemory */
    ld_vorbis_backend_uninit
};

ma_decoding_backend_vtable* ld_decoding_backend_vorbis = &g_ld_vorbis_backend_vtable;

#endif /* miniaudio_vorbis_pull_c */
