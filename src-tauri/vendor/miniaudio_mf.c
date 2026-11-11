/* See miniaudio_mf.h for what this backend covers and how files reach it. */
#ifndef miniaudio_mf_c
#define miniaudio_mf_c

#include "miniaudio_mf.h"
#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>
#include <string.h>
#include <math.h>

/* ---- COM and Media Foundation startup ----------------------------------------
 *
 * COM (the Windows component model MF is built on) must be initialised on a
 * thread before that thread calls MF. Our calls arrive on several threads:
 * the resource manager's job thread, the playback loader thread, and Tokio
 * blocking threads running waveform/mood analysis.
 *
 * Per call, ld_mf_com_enter joins the thread to the multithreaded apartment
 * (MTA, COM's "any thread may call" mode) and ld_mf_com_leave undoes it, so the
 * thread is left as it was found. If the thread already chose the
 * single-threaded mode (STA, e.g. a UI thread), CoInitializeEx returns
 * RPC_E_CHANGED_MODE; that thread is already initialised and the MF objects
 * used here are free-threaded, so the call simply proceeds without the undo.
 *
 * Once per process: CoIncrementMTAUsage keeps the MTA alive even while no
 * thread is inside an enter/leave pair, so MF objects created inside one pair
 * stay valid between calls (a stream is opened in one call and read in many
 * later ones, possibly from another thread). MFStartup is likewise called once
 * and never paired with MFShutdown: decoders may still be open at any moment,
 * and the process exiting cleans up. */

static INIT_ONCE g_ld_mf_once = INIT_ONCE_STATIC_INIT;
static BOOL g_ld_mf_ready = FALSE;

static BOOL CALLBACK ld_mf_startup(PINIT_ONCE once, PVOID param, PVOID* ctx)
{
    (void)once; (void)param; (void)ctx;
    CO_MTA_USAGE_COOKIE cookie;
    if (FAILED(CoIncrementMTAUsage(&cookie))) return TRUE;   /* leaves g_ld_mf_ready FALSE */
    /* LITE: local playback only, skips MF's network stack. */
    g_ld_mf_ready = SUCCEEDED(MFStartup(MF_VERSION, MFSTARTUP_LITE));
    return TRUE;
}

static ma_bool32 ld_mf_ready(void)
{
    InitOnceExecuteOnce(&g_ld_mf_once, ld_mf_startup, NULL, NULL);
    return g_ld_mf_ready ? MA_TRUE : MA_FALSE;
}

/* Returns 1 if a matching ld_mf_com_leave must undo the init. */
static int ld_mf_com_enter(void)
{
    HRESULT hr = CoInitializeEx(NULL, COINIT_MULTITHREADED);
    return SUCCEEDED(hr) ? 1 : 0;   /* S_OK / S_FALSE count; RPC_E_CHANGED_MODE does not */
}

static void ld_mf_com_leave(int entered)
{
    if (entered) CoUninitialize();
}

/* ---- IStream over miniaudio's read/seek/tell callbacks ------------------------
 *
 * The resource manager hands backends only read/seek/tell callbacks, never a
 * path. MF can read from any COM IStream (via MFCreateMFByteStreamOnStream), so
 * this is the smallest IStream that forwards to those callbacks.
 *
 * A COM object in C is a struct whose first member points at a table of
 * function pointers (the "vtable"); callers only ever go through that table.
 * Reference counting (AddRef/Release) decides when it is freed, because MF
 * keeps its own references and may release them after our decoder is gone.
 *
 * MF reads ahead on its own worker threads, so Read/Seek can run on a thread
 * other than the one decoding, and may still arrive after the decoder closed.
 * The lock serialises them, and `detached` (set on uninit, before miniaudio
 * closes the underlying file) makes any late call fail instead of touching a
 * closed file. */

typedef struct ld_mf_stream
{
    IStream iface;              /* MUST be first: COM callers see this struct as an IStream*. */
    volatile LONG refs;
    SRWLOCK lock;
    ma_read_proc onRead;
    ma_seek_proc onSeek;
    ma_tell_proc onTell;
    void* ud;
    ma_uint64 size;             /* Total bytes, measured once at creation. */
    int detached;
} ld_mf_stream;

static HRESULT STDMETHODCALLTYPE ld_mfs_QueryInterface(IStream* This, REFIID riid, void** ppv)
{
    if (ppv == NULL) return E_POINTER;
    if (IsEqualIID(riid, &IID_IUnknown) || IsEqualIID(riid, &IID_ISequentialStream) || IsEqualIID(riid, &IID_IStream)) {
        *ppv = This;
        This->lpVtbl->AddRef(This);
        return S_OK;
    }
    *ppv = NULL;
    return E_NOINTERFACE;
}

static ULONG STDMETHODCALLTYPE ld_mfs_AddRef(IStream* This)
{
    return (ULONG)InterlockedIncrement(&((ld_mf_stream*)This)->refs);
}

static ULONG STDMETHODCALLTYPE ld_mfs_Release(IStream* This)
{
    LONG n = InterlockedDecrement(&((ld_mf_stream*)This)->refs);
    if (n == 0) free(This);
    return (ULONG)n;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Read(IStream* This, void* pv, ULONG cb, ULONG* pcbRead)
{
    ld_mf_stream* s = (ld_mf_stream*)This;
    HRESULT hr = S_OK;
    size_t total = 0;
    AcquireSRWLockExclusive(&s->lock);
    if (s->detached) {
        hr = E_FAIL;
    } else {
        /* A callback may return fewer bytes than asked; keep going until the
           request is filled or the stream ends. */
        while (total < cb) {
            size_t got = 0;
            ma_result r = s->onRead(s->ud, (ma_uint8*)pv + total, cb - total, &got);
            total += got;
            if (got == 0) {
                if (r != MA_SUCCESS && r != MA_AT_END) hr = E_FAIL;
                break;
            }
        }
    }
    ReleaseSRWLockExclusive(&s->lock);
    if (pcbRead != NULL) *pcbRead = (ULONG)total;
    if (FAILED(hr)) return hr;
    return (total == cb) ? S_OK : S_FALSE;   /* S_FALSE = short read, end of stream */
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Write(IStream* This, const void* pv, ULONG cb, ULONG* pcbWritten)
{
    (void)This; (void)pv; (void)cb;
    if (pcbWritten != NULL) *pcbWritten = 0;
    return STG_E_ACCESSDENIED;   /* read-only */
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Seek(IStream* This, LARGE_INTEGER move, DWORD origin, ULARGE_INTEGER* newPos)
{
    ld_mf_stream* s = (ld_mf_stream*)This;
    ma_seek_origin o = (origin == STREAM_SEEK_SET) ? ma_seek_origin_start
                     : (origin == STREAM_SEEK_END) ? ma_seek_origin_end
                     : ma_seek_origin_current;
    HRESULT hr = S_OK;
    ma_int64 pos = 0;
    AcquireSRWLockExclusive(&s->lock);
    if (s->detached || s->onSeek(s->ud, move.QuadPart, o) != MA_SUCCESS || s->onTell(s->ud, &pos) != MA_SUCCESS) {
        hr = STG_E_INVALIDFUNCTION;
    }
    ReleaseSRWLockExclusive(&s->lock);
    if (SUCCEEDED(hr) && newPos != NULL) newPos->QuadPart = (ULONGLONG)pos;
    return hr;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_SetSize(IStream* This, ULARGE_INTEGER size)
{
    (void)This; (void)size;
    return STG_E_ACCESSDENIED;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_CopyTo(IStream* This, IStream* dst, ULARGE_INTEGER cb, ULARGE_INTEGER* pcbRead, ULARGE_INTEGER* pcbWritten)
{
    (void)This; (void)dst; (void)cb; (void)pcbRead; (void)pcbWritten;
    return E_NOTIMPL;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Commit(IStream* This, DWORD flags)
{
    (void)This; (void)flags;
    return S_OK;   /* nothing is ever written */
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Revert(IStream* This)
{
    (void)This;
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_LockRegion(IStream* This, ULARGE_INTEGER off, ULARGE_INTEGER cb, DWORD type)
{
    (void)This; (void)off; (void)cb; (void)type;
    return STG_E_INVALIDFUNCTION;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_UnlockRegion(IStream* This, ULARGE_INTEGER off, ULARGE_INTEGER cb, DWORD type)
{
    (void)This; (void)off; (void)cb; (void)type;
    return STG_E_INVALIDFUNCTION;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Stat(IStream* This, STATSTG* st, DWORD flags)
{
    (void)flags;
    if (st == NULL) return E_POINTER;
    memset(st, 0, sizeof(*st));     /* pwcsName stays NULL: no name to report */
    st->type = STGTY_STREAM;
    st->cbSize.QuadPart = ((ld_mf_stream*)This)->size;
    st->grfMode = STGM_READ;
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Clone(IStream* This, IStream** out)
{
    (void)This;
    if (out != NULL) *out = NULL;
    return E_NOTIMPL;
}

static IStreamVtbl g_ld_mf_stream_vtbl =
{
    ld_mfs_QueryInterface, ld_mfs_AddRef, ld_mfs_Release,
    ld_mfs_Read, ld_mfs_Write,
    ld_mfs_Seek, ld_mfs_SetSize, ld_mfs_CopyTo, ld_mfs_Commit, ld_mfs_Revert,
    ld_mfs_LockRegion, ld_mfs_UnlockRegion, ld_mfs_Stat, ld_mfs_Clone
};

/* Returns a stream holding one reference, or NULL. */
static ld_mf_stream* ld_mf_stream_create(ma_read_proc onRead, ma_seek_proc onSeek, ma_tell_proc onTell, void* ud)
{
    /* Size up front: Stat must report it, and MF uses it to bound its reads. */
    ma_int64 size = 0;
    if (onSeek(ud, 0, ma_seek_origin_end) != MA_SUCCESS || onTell(ud, &size) != MA_SUCCESS || size <= 0) return NULL;
    if (onSeek(ud, 0, ma_seek_origin_start) != MA_SUCCESS) return NULL;

    ld_mf_stream* s = (ld_mf_stream*)calloc(1, sizeof(ld_mf_stream));
    if (s == NULL) return NULL;
    s->iface.lpVtbl = &g_ld_mf_stream_vtbl;
    s->refs = 1;
    InitializeSRWLock(&s->lock);
    s->onRead = onRead;
    s->onSeek = onSeek;
    s->onTell = onTell;
    s->ud = ud;
    s->size = (ma_uint64)size;
    return s;
}

/* Cuts the stream off from miniaudio's callbacks; later reads fail. */
static void ld_mf_stream_detach(ld_mf_stream* s)
{
    AcquireSRWLockExclusive(&s->lock);
    s->detached = 1;
    ReleaseSRWLockExclusive(&s->lock);
}

/* ---- Content sniffing ----------------------------------------------------------- */

typedef enum { LD_MF_NONE, LD_MF_MP4, LD_MF_ADTS, LD_MF_ASF } ld_mf_kind;

/* Reads exactly n bytes at offset off, or fails. Two sources: an MF byte
   stream (the file path), and the callback IStream (the playback path), which
   is sniffed before any MF object exists, so files this backend rejects
   (every MP3/WAV/FLAC played) cost only a few small reads. */
typedef ma_bool32 (*ld_read_at_fn)(void* src, QWORD off, BYTE* buf, ULONG n);

static ma_bool32 ld_mf_read_at(IMFByteStream* bs, QWORD off, BYTE* buf, ULONG n)
{
    ULONG got = 0;
    if (FAILED(bs->lpVtbl->SetCurrentPosition(bs, off))) return MA_FALSE;
    if (FAILED(bs->lpVtbl->Read(bs, buf, n, &got))) return MA_FALSE;
    return got == n;
}

static ma_bool32 ld_bytestream_read_at(void* src, QWORD off, BYTE* buf, ULONG n)
{
    return ld_mf_read_at((IMFByteStream*)src, off, buf, n);
}

static ma_bool32 ld_istream_read_at(void* src, QWORD off, BYTE* buf, ULONG n)
{
    IStream* st = (IStream*)src;
    LARGE_INTEGER pos;
    ULONG got = 0;
    pos.QuadPart = (LONGLONG)off;
    if (FAILED(st->lpVtbl->Seek(st, pos, STREAM_SEEK_SET, NULL))) return MA_FALSE;
    return st->lpVtbl->Read(st, buf, n, &got) == S_OK && got == n;
}

/* An ADTS frame header: 12 sync bits, then the 2 "layer" bits, always 00 for
   AAC. MP3 frames share the sync bits but have a nonzero layer, which is what
   keeps MP3 files away from this backend. */
static ma_bool32 ld_mf_is_adts_header(const BYTE* h)
{
    return h[0] == 0xFF && (h[1] & 0xF6) == 0xF0;
}

/* Also reports where the audio data starts (past any ID3 tags) in *pAudioStart. */
static ld_mf_kind ld_mf_sniff(ld_read_at_fn read_at, void* src, QWORD* pAudioStart)
{
    /* First 16 bytes of the ASF header object's GUID. */
    static const BYTE asf_guid[16] = { 0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11,
                                       0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C };
    BYTE h[16];
    QWORD off = 0;

    *pAudioStart = 0;
    if (!read_at(src, 0, h, sizeof(h))) return LD_MF_NONE;
    /* MP4 (.m4a/.mp4): the first box is "ftyp", its 4-byte size then the type. */
    if (memcmp(h + 4, "ftyp", 4) == 0) return LD_MF_MP4;
    if (memcmp(h, asf_guid, 16) == 0) return LD_MF_ASF;

    /* Raw AAC may start with ID3v2 tags, as MP3s do. Each is a 10-byte header
       ("ID3", version, flags, then a size in 4 bytes of 7 bits each) plus an
       optional 10-byte footer (flag 0x10). Skip them to reach the audio. */
    while (memcmp(h, "ID3", 3) == 0) {
        QWORD tag = ((QWORD)(h[6] & 0x7F) << 21) | ((QWORD)(h[7] & 0x7F) << 14) |
                    ((QWORD)(h[8] & 0x7F) << 7) | (QWORD)(h[9] & 0x7F);
        off += 10 + tag + ((h[5] & 0x10) ? 10 : 0);
        if (!read_at(src, off, h, sizeof(h))) return LD_MF_NONE;
    }

    if (ld_mf_is_adts_header(h)) {
        /* Confirm with the next frame's header: 13-bit frame length at bits 30..42. */
        QWORD len = ((QWORD)(h[3] & 0x03) << 11) | ((QWORD)h[4] << 3) | ((QWORD)h[5] >> 5);
        BYTE next[2];
        if (len >= 7 && read_at(src, off + len, next, 2) && ld_mf_is_adts_header(next)) {
            *pAudioStart = off;
            return LD_MF_ADTS;
        }
    }
    return LD_MF_NONE;
}

/* ---- Exact lengths the source reader doesn't provide ------------------------------
 *
 * AAC encoders add "priming" (warm-up) frames at the start of the stream and
 * pad the end up to a whole 1024-frame packet. An MP4 file records how much of
 * that to cut, but MF's source reader ignores it and returns every decoded
 * frame, while its reported duration does exclude them. Left alone, playback
 * would start ~23-50 ms late and a fully loaded sound would be cut short at the
 * declared length, so the trim is read from the file here and applied in the
 * decoder (see ld_mf_open). */

static ma_uint32 ld_be32(const BYTE* p) { return ((ma_uint32)p[0] << 24) | ((ma_uint32)p[1] << 16) | ((ma_uint32)p[2] << 8) | p[3]; }
static ma_uint64 ld_be64(const BYTE* p) { return ((ma_uint64)ld_be32(p) << 32) | ld_be32(p + 4); }

/* MP4 files are a tree of "boxes": a 4-byte big-endian size (the whole box,
   header included), a 4-character type, then the payload, which for container
   boxes is more boxes. Finds the next box of `type` among the boxes laid out
   in [*pFrom, end). On success sets [*pStart, *pEnd) to its payload and moves
   *pFrom past it, so repeated calls walk siblings of the same type. */
static ma_bool32 ld_mp4_next(IMFByteStream* bs, QWORD* pFrom, QWORD end, const char* type, QWORD* pStart, QWORD* pEnd)
{
    QWORD off = *pFrom;
    while (off + 8 <= end) {
        BYTE h[16];
        if (!ld_mf_read_at(bs, off, h, 8)) return MA_FALSE;
        QWORD size = ld_be32(h), hdr = 8;
        if (size == 1) {                    /* 64-bit size follows the type */
            if (!ld_mf_read_at(bs, off + 8, h + 8, 8)) return MA_FALSE;
            size = ld_be64(h + 8);
            hdr = 16;
        } else if (size == 0) {             /* runs to the end of the parent */
            size = end - off;
        }
        if (size < hdr || size > end - off) return MA_FALSE;
        if (memcmp(h + 4, type, 4) == 0) {
            *pStart = off + hdr;
            *pEnd = off + size;
            *pFrom = off + size;
            return MA_TRUE;
        }
        off += size;
    }
    return MA_FALSE;
}

static ma_bool32 ld_mp4_find(IMFByteStream* bs, QWORD from, QWORD end, const char* type, QWORD* pStart, QWORD* pEnd)
{
    return ld_mp4_next(bs, &from, end, type, pStart, pEnd);
}

/* Reads "mvhd"/"mdhd": a version byte and 3 flag bytes, then (version 1)
   64-bit creation/modification times or (version 0) 32-bit ones, then the
   32-bit timescale (ticks per second) and the duration in those ticks. */
static ma_bool32 ld_mp4_read_header(IMFByteStream* bs, QWORD start, ma_uint32* pTimescale, ma_uint64* pDuration)
{
    BYTE b[32];
    if (!ld_mf_read_at(bs, start, b, 1)) return MA_FALSE;
    if (b[0] == 1) {
        if (!ld_mf_read_at(bs, start, b, 32)) return MA_FALSE;
        *pTimescale = ld_be32(b + 20);
        *pDuration = ld_be64(b + 24);
    } else {
        if (!ld_mf_read_at(bs, start, b, 20)) return MA_FALSE;
        *pTimescale = ld_be32(b + 12);
        *pDuration = ld_be32(b + 16);
    }
    return *pTimescale != 0;
}

/* Apple's encoder records the trim in an iTunes metadata item named
   "iTunSMPB" instead of an edit list: hex text whose 2nd, 3rd and 4th fields
   are the priming frames, the padding frames and the real frame count. It
   lives at moov/udta/meta/ilst/"----", next to "name" and "data" children. */
static ma_bool32 ld_mp4_itunsmpb(IMFByteStream* bs, QWORD moovStart, QWORD moovEnd, ma_uint64* pSkip, ma_uint64* pFrames)
{
    QWORD a, b, c, d, e, f, from, itemStart, itemEnd;
    if (!ld_mp4_find(bs, moovStart, moovEnd, "udta", &a, &b)) return MA_FALSE;
    if (!ld_mp4_find(bs, a, b, "meta", &c, &d)) return MA_FALSE;
    /* "meta" has a version/flags word before its children. */
    if (!ld_mp4_find(bs, c + 4, d, "ilst", &e, &f)) return MA_FALSE;
    from = e;
    while (ld_mp4_next(bs, &from, f, "----", &itemStart, &itemEnd)) {
        QWORD ns, ne, ds, de;
        char name[8];
        char text[128] = { 0 };
        unsigned int zero, priming, padding;
        unsigned long long frames;
        /* "name" payload: version/flags (4), then the name. */
        if (!ld_mp4_find(bs, itemStart, itemEnd, "name", &ns, &ne) || ne - ns != 4 + 8) continue;
        if (!ld_mf_read_at(bs, ns + 4, (BYTE*)name, 8) || memcmp(name, "iTunSMPB", 8) != 0) continue;
        /* "data" payload: type (4), locale (4), then the text. */
        if (!ld_mp4_find(bs, itemStart, itemEnd, "data", &ds, &de) || de - ds <= 8) return MA_FALSE;
        ULONG n = (ULONG)((de - ds - 8 < sizeof(text) - 1) ? de - ds - 8 : sizeof(text) - 1);
        if (!ld_mf_read_at(bs, ds + 8, (BYTE*)text, n)) return MA_FALSE;
        if (sscanf_s(text, " %x %x %x %llx", &zero, &priming, &padding, &frames) != 4 || frames == 0) return MA_FALSE;
        *pSkip = priming;
        *pFrames = frames;
        return MA_TRUE;
    }
    return MA_FALSE;
}

/* Media time in `scale` ticks per second -> seconds. */
static double ld_mp4_secs(ma_uint64 t, ma_uint32 scale)
{
    return (double)t / (double)scale;
}

/* Finds the trim for the first sound track of an MP4 file, in seconds: how much
   to drop from the start (*pSkip) and how much real audio follows (*pLen).
   Seconds rather than frames because this runs before the decoder's output
   rate is known (it must finish before MF starts reading the same byte
   stream). Sources, in order of preference:
     - the track's edit list ("elst"): one entry giving the media time where
       playback starts and how long it lasts (ffmpeg and most tools);
     - iTunSMPB (Apple's encoder);
     - neither: no trim, length = the track's full duration ("mdhd").
   Returns MA_FALSE if the file can't be parsed; the caller then keeps MF's
   numbers. */
static ma_bool32 ld_mp4_trim(IMFByteStream* bs, double* pSkip, double* pLen)
{
    QWORD size = 0, moovStart, moovEnd, from, trakStart, trakEnd, s, e, s2, e2;
    ma_uint32 movieScale = 0, mediaScale = 0;
    ma_uint64 movieDur = 0, mediaDur = 0, skipTicks = 0, lenTicks = 0;
    double total, skip, len;

    if (FAILED(bs->lpVtbl->GetLength(bs, &size))) return MA_FALSE;
    if (!ld_mp4_find(bs, 0, size, "moov", &moovStart, &moovEnd)) return MA_FALSE;
    if (!ld_mp4_find(bs, moovStart, moovEnd, "mvhd", &s, &e) || !ld_mp4_read_header(bs, s, &movieScale, &movieDur)) return MA_FALSE;

    /* The first track whose handler ("hdlr") is "soun", i.e. audio. */
    from = moovStart;
    for (;;) {
        BYTE handler[4];
        if (!ld_mp4_next(bs, &from, moovEnd, "trak", &trakStart, &trakEnd)) return MA_FALSE;
        if (!ld_mp4_find(bs, trakStart, trakEnd, "mdia", &s, &e)) continue;
        /* hdlr payload: version/flags (4), pre_defined (4), handler type (4). */
        if (!ld_mp4_find(bs, s, e, "hdlr", &s2, &e2) || !ld_mf_read_at(bs, s2 + 8, handler, 4)) continue;
        if (memcmp(handler, "soun", 4) != 0) continue;
        if (!ld_mp4_find(bs, s, e, "mdhd", &s2, &e2) || !ld_mp4_read_header(bs, s2, &mediaScale, &mediaDur)) return MA_FALSE;
        break;
    }
    total = ld_mp4_secs(mediaDur, mediaScale);

    if (ld_mp4_find(bs, trakStart, trakEnd, "edts", &s, &e) && ld_mp4_find(bs, s, e, "elst", &s2, &e2)) {
        /* elst payload: version/flags (4), entry count (4), then entries of
           segment duration (movie timescale), media time (media timescale,
           -1 for an empty edit) and a 4-byte rate; 64-bit fields in version 1.
           Only the common single-entry form is handled. */
        BYTE b[20];
        if (ld_mf_read_at(bs, s2, b, 8) && ld_be32(b + 4) == 1) {
            ma_bool32 v1 = (b[0] == 1);
            if (ld_mf_read_at(bs, s2 + 8, b, v1 ? 16 : 8)) {
                ma_uint64 segDur = v1 ? ld_be64(b) : ld_be32(b);
                ma_int64 mediaTime = v1 ? (ma_int64)ld_be64(b + 8) : (ma_int64)(ma_int32)ld_be32(b + 4);
                if (mediaTime >= 0) {
                    skip = ld_mp4_secs((ma_uint64)mediaTime, mediaScale);
                    len = ld_mp4_secs(segDur, movieScale);
                    if (skip > total) skip = total;
                    /* Some writers leave the duration 0, meaning "to the end". */
                    if (len <= 0.0 || len > total - skip) len = total - skip;
                    *pSkip = skip;
                    *pLen = len;
                    return MA_TRUE;
                }
            }
        }
    }

    if (ld_mp4_itunsmpb(bs, moovStart, moovEnd, &skipTicks, &lenTicks)) {
        skip = ld_mp4_secs(skipTicks, mediaScale);
        len = ld_mp4_secs(lenTicks, mediaScale);
        if (skip <= total && len <= total - skip) {
            *pSkip = skip;
            *pLen = len;
            return MA_TRUE;
        }
    }

    *pSkip = 0.0;
    *pLen = total;
    return total > 0.0;
}

/* Raw ADTS AAC has no index or duration field; MF estimates the length from
   the bitrate. Counting the frame headers gives the exact figure: each header
   holds its frame's byte length and how many 1024-frame AAC blocks it
   carries. Reads in 64 KB chunks, so even a long file takes only a few ms.
   Returns the length in seconds, or 0 if the stream can't be walked. */
static double ld_adts_secs(IMFByteStream* bs, QWORD audioStart)
{
    static const ma_uint32 rates[16] = { 96000, 88200, 64000, 48000, 44100, 32000, 24000,
                                         22050, 16000, 12000, 11025, 8000, 7350, 0, 0, 0 };
    enum { CHUNK = 1 << 16 };
    BYTE* buf = (BYTE*)malloc(CHUNK);
    QWORD bufStart = 0, pos = audioStart;
    ULONG bufLen = 0;
    ma_uint64 blocks = 0;
    ma_uint32 coreRate = 0;
    if (buf == NULL) return 0;

    for (;;) {
        /* Keep the 7-byte header at `pos` inside the buffer. */
        if (pos < bufStart || pos + 7 > bufStart + bufLen) {
            if (FAILED(bs->lpVtbl->SetCurrentPosition(bs, pos)) ||
                FAILED(bs->lpVtbl->Read(bs, buf, CHUNK, &bufLen)) || bufLen < 7) break;
            bufStart = pos;
        }
        const BYTE* h = buf + (pos - bufStart);
        if (!ld_mf_is_adts_header(h)) break;     /* a trailing tag or junk ends the count */
        QWORD len = ((QWORD)(h[3] & 0x03) << 11) | ((QWORD)h[4] << 3) | ((QWORD)h[5] >> 5);
        if (len < 7) break;
        if (coreRate == 0) coreRate = rates[(h[2] >> 2) & 0x0F];
        blocks += (ma_uint64)(h[6] & 0x03) + 1;
        pos += len;
    }
    free(buf);
    if (coreRate == 0) return 0.0;
    /* Seconds, not frames: an HE-AAC decoder outputs at twice this rate. */
    return (double)blocks * 1024.0 / (double)coreRate;
}

/* ---- The decoder ------------------------------------------------------------------ */

typedef struct
{
    ma_data_source_base ds;     /* MUST be first: miniaudio treats ld_mf* as ma_data_source*. */
    IMFSourceReader* reader;
    ld_mf_stream* stream;       /* The callback stream (onInit path), else NULL. */
    ma_format format;
    ma_uint32 channels;
    ma_uint32 sampleRate;
    ma_uint32 bytesPerFrame;
    ma_uint64 length;           /* Total frames (sources: see the end of ld_mf_open). */
    ma_uint64 cursor;           /* Frame index of the next frame read() returns. */
    ma_int64 origin;            /* Timestamp, in frames, that counts as frame 0. */
    ma_int64 firstStart;        /* Where the file's first decoded chunk starts, relative to origin. */
    ma_bool32 exactTimestamps;  /* Container timestamps are sample-accurate (see seek). */
    ma_bool32 stopAtLength;     /* `length` is exact: never hand out frames past it. */
    ma_bool32 eof;
    /* The most recent decoded chunk, partly handed out: frames [pendPos, pendFrames). */
    ma_uint8* pend;
    size_t pendCap;             /* bytes */
    ma_uint64 pendFrames;
    ma_uint64 pendPos;
} ld_mf;

/* MF timestamps are in 100 ns units. Rounding to the nearest frame recovers
   the exact frame index whenever the timestamp was derived from one. */
static ma_int64 ld_mf_ts_to_frames(LONGLONG ts, ma_uint32 sampleRate)
{
    return (ma_int64)llround((double)ts * (double)sampleRate / 10000000.0);
}

static LONGLONG ld_mf_frames_to_ts(ma_int64 frames, ma_uint32 sampleRate)
{
    return (LONGLONG)llround((double)frames * 10000000.0 / (double)sampleRate);
}

/* Decodes the next chunk into `pend` (replacing what was there) and reports
   where it starts, in frames relative to `origin`, from its timestamp.
   MA_AT_END once the stream is exhausted. */
static ma_result ld_mf_fetch(ld_mf* m, ma_int64* pStart)
{
    for (;;) {
        DWORD flags = 0;
        LONGLONG ts = 0;
        IMFSample* sample = NULL;
        HRESULT hr = m->reader->lpVtbl->ReadSample(m->reader, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM,
                                                   0, NULL, &flags, &ts, &sample);
        if (FAILED(hr) || (flags & MF_SOURCE_READERF_ERROR)) {
            if (sample != NULL) sample->lpVtbl->Release(sample);
            return MA_ERROR;
        }
        if (flags & MF_SOURCE_READERF_ENDOFSTREAM) {
            if (sample != NULL) sample->lpVtbl->Release(sample);
            m->eof = MA_TRUE;
            m->pendFrames = m->pendPos = 0;
            return MA_AT_END;
        }
        /* The output format is fixed at open; a mid-stream change (rare, e.g. a
           channel count switch) can't be passed on to miniaudio. */
        if (flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED) {
            if (sample != NULL) sample->lpVtbl->Release(sample);
            return MA_ERROR;
        }
        if (sample == NULL) continue;   /* a gap marker with no audio */

        IMFMediaBuffer* buf = NULL;
        BYTE* data = NULL;
        DWORD bytes = 0;
        hr = sample->lpVtbl->ConvertToContiguousBuffer(sample, &buf);
        if (SUCCEEDED(hr)) hr = buf->lpVtbl->Lock(buf, &data, NULL, &bytes);
        if (FAILED(hr)) {
            if (buf != NULL) buf->lpVtbl->Release(buf);
            sample->lpVtbl->Release(sample);
            return MA_ERROR;
        }
        ma_uint64 frames = bytes / m->bytesPerFrame;
        size_t need = (size_t)(frames * m->bytesPerFrame);
        ma_result r = MA_SUCCESS;
        if (need > m->pendCap) {
            ma_uint8* grown = (ma_uint8*)realloc(m->pend, need);
            if (grown == NULL) {
                r = MA_OUT_OF_MEMORY;
            } else {
                m->pend = grown;
                m->pendCap = need;
            }
        }
        if (r == MA_SUCCESS) {
            memcpy(m->pend, data, need);
            m->pendFrames = frames;
            m->pendPos = 0;
        }
        buf->lpVtbl->Unlock(buf);
        buf->lpVtbl->Release(buf);
        sample->lpVtbl->Release(sample);
        if (r != MA_SUCCESS) return r;
        if (frames == 0) continue;
        *pStart = ld_mf_ts_to_frames(ts, m->sampleRate) - m->origin;
        return MA_SUCCESS;
    }
}

/* `pend` holds a fresh chunk that starts at frame `pos`. Decodes forward,
   discarding, until the next frame to hand out is `target`. Positions before 0
   (decoder warm-up the container marks as not part of the audio) are dropped
   the same way. Decoded chunks are treated as back to back: only the first
   chunk's timestamp is used. */
static ma_result ld_mf_skip_to(ld_mf* m, ma_int64 pos, ma_uint64 target)
{
    for (;;) {
        ma_int64 avail = (ma_int64)(m->pendFrames - m->pendPos);
        if (pos + avail > (ma_int64)target) {
            m->pendPos += (ma_uint64)((ma_int64)target - pos);
            m->cursor = target;
            return MA_SUCCESS;
        }
        pos += avail;
        m->pendPos = m->pendFrames;
        ma_int64 ignored;
        ma_result r = ld_mf_fetch(m, &ignored);
        if (r == MA_AT_END) {
            /* Target at or past the end: reads now return nothing. */
            m->cursor = target;
            return MA_SUCCESS;
        }
        if (r != MA_SUCCESS) return r;
    }
}

/* Repositions MF at the stream position `frame` (relative to origin) and
   decodes the first chunk there. Reports that chunk's start frame. */
static ma_result ld_mf_reposition(ld_mf* m, ma_int64 frame, ma_int64* pStart)
{
    PROPVARIANT pv;
    PropVariantInit(&pv);
    pv.vt = VT_I8;
    pv.hVal.QuadPart = ld_mf_frames_to_ts(frame + m->origin, m->sampleRate);
    HRESULT hr = m->reader->lpVtbl->SetCurrentPosition(m->reader, &GUID_NULL, &pv);
    PropVariantClear(&pv);
    if (FAILED(hr)) return MA_ERROR;
    m->eof = MA_FALSE;
    m->pendFrames = m->pendPos = 0;
    return ld_mf_fetch(m, pStart);
}

/* Back to the very start of the file (timestamp 0, before any priming that
   gets dropped), so the decoder rebuilds exactly the state it had at open. The
   first chunk is the same one decoded at open, so it gets the same position. */
static ma_result ld_mf_restart(ld_mf* m, ma_int64* pStart)
{
    ma_result r = ld_mf_reposition(m, -m->origin, pStart);
    if (r == MA_SUCCESS) *pStart = m->firstStart;
    return r;
}

static ma_result ld_mf_read(ma_data_source* pDataSource, void* pFramesOut, ma_uint64 frameCount, ma_uint64* pFramesRead)
{
    ld_mf* m = (ld_mf*)pDataSource;
    ma_uint64 done = 0;
    ma_result r = MA_SUCCESS;

    /* The MP4 trim's end: what follows is encoder padding. */
    if (m->stopAtLength) {
        ma_uint64 left = (m->cursor < m->length) ? m->length - m->cursor : 0;
        if (frameCount > left) frameCount = left;
        if (frameCount == 0) {
            if (pFramesRead != NULL) *pFramesRead = 0;
            return MA_AT_END;
        }
    }

    int com = ld_mf_com_enter();
    while (done < frameCount) {
        ma_uint64 avail = m->pendFrames - m->pendPos;
        if (avail > 0) {
            ma_uint64 n = frameCount - done;
            if (n > avail) n = avail;
            if (pFramesOut != NULL) {
                memcpy((ma_uint8*)pFramesOut + done * m->bytesPerFrame,
                       m->pend + m->pendPos * m->bytesPerFrame, (size_t)(n * m->bytesPerFrame));
            }
            m->pendPos += n;
            done += n;
            continue;
        }
        if (m->eof) break;
        ma_int64 ignored;
        r = ld_mf_fetch(m, &ignored);
        if (r == MA_AT_END) { r = MA_SUCCESS; break; }
        if (r != MA_SUCCESS) break;
    }

    ld_mf_com_leave(com);
    m->cursor += done;
    if (pFramesRead != NULL) *pFramesRead = done;
    /* Frames already decoded are delivered even if decoding then failed; the
       error surfaces on the next call. */
    if (done > 0) return MA_SUCCESS;
    if (r != MA_SUCCESS) return r;
    return (frameCount > 0) ? MA_AT_END : MA_SUCCESS;
}

/* Sample-accurate seek.
 *
 * MF's SetCurrentPosition lands on a packet at or before the requested time,
 * and lossy decoders (AAC, WMA) need the packet before the target as well to
 * rebuild their state, so the first output after a jump is not yet exact. So:
 * jump to a point LD_MF_SEEK_PREROLL_SECS before the target, read the timestamp of
 * the first decoded chunk to learn where we actually are, then decode forward
 * and discard up to the exact target frame.
 *
 * That relies on timestamps that map to exact frames. MP4 timestamps do (they
 * count samples in the track's own timescale). ASF only stores milliseconds
 * and raw ADTS has no timestamps at all (MF estimates them), so for those
 * containers a backward seek restarts from frame 0 and decodes forward
 * instead: slower for targets deep into a long file, but exact. Short forward
 * seeks in every container just decode forward from the current position. */
#define LD_MF_SEEK_PREROLL_SECS 0.25
#define LD_MF_DECODE_FORWARD_SECS 2

static ma_result ld_mf_seek(ma_data_source* pDataSource, ma_uint64 frameIndex)
{
    ld_mf* m = (ld_mf*)pDataSource;
    ma_result r;
    ma_int64 start = 0;
    int com = ld_mf_com_enter();

    ma_uint64 forwardLimit = (ma_uint64)m->sampleRate * LD_MF_DECODE_FORWARD_SECS;
    ma_bool32 forward = frameIndex >= m->cursor;

    if (forward && (frameIndex - m->cursor <= forwardLimit || !m->exactTimestamps)) {
        /* `pend` resumes exactly at the cursor. */
        r = ld_mf_skip_to(m, (ma_int64)m->cursor, frameIndex);
    } else if (m->exactTimestamps) {
        ma_int64 preroll = (ma_int64)(m->sampleRate * LD_MF_SEEK_PREROLL_SECS);
        ma_int64 jump = (ma_int64)frameIndex - preroll;
        if (jump < 0) jump = 0;
        r = (jump == 0) ? ld_mf_restart(m, &start) : ld_mf_reposition(m, jump, &start);
        /* Landed past the target (a container with sparse seek points): go
           back to the start, which is always exact. */
        if (r == MA_SUCCESS && start > (ma_int64)frameIndex) r = ld_mf_restart(m, &start);
        if (r == MA_AT_END) { m->cursor = frameIndex; r = MA_SUCCESS; }
        else if (r == MA_SUCCESS) r = ld_mf_skip_to(m, start, frameIndex);
    } else {
        r = ld_mf_restart(m, &start);
        if (r == MA_AT_END) { m->cursor = frameIndex; r = MA_SUCCESS; }
        else if (r == MA_SUCCESS) r = ld_mf_skip_to(m, start, frameIndex);
    }

    ld_mf_com_leave(com);
    return r;
}

static ma_result ld_mf_get_data_format(ma_data_source* pDataSource, ma_format* pFormat, ma_uint32* pChannels, ma_uint32* pSampleRate, ma_channel* pChannelMap, size_t channelMapCap)
{
    ld_mf* m = (ld_mf*)pDataSource;
    if (pFormat != NULL) *pFormat = m->format;
    if (pChannels != NULL) *pChannels = m->channels;
    if (pSampleRate != NULL) *pSampleRate = m->sampleRate;
    /* MF delivers channels in the standard WAV order. */
    if (pChannelMap != NULL) ma_channel_map_init_standard(ma_standard_channel_map_microsoft, pChannelMap, channelMapCap, m->channels);
    return MA_SUCCESS;
}

static ma_result ld_mf_get_cursor(ma_data_source* pDataSource, ma_uint64* pCursor)
{
    *pCursor = ((ld_mf*)pDataSource)->cursor;
    return MA_SUCCESS;
}

static ma_result ld_mf_get_length(ma_data_source* pDataSource, ma_uint64* pLength)
{
    *pLength = ((ld_mf*)pDataSource)->length;
    return MA_SUCCESS;
}

static ma_data_source_vtable g_ld_mf_ds_vtable =
{
    ld_mf_read,
    ld_mf_seek,
    ld_mf_get_data_format,
    ld_mf_get_cursor,
    ld_mf_get_length,
    NULL,   /* onSetLooping */
    0       /* flags */
};

static void ld_mf_free(ld_mf* m, const ma_allocation_callbacks* pAlloc)
{
    if (m->reader != NULL) m->reader->lpVtbl->Release(m->reader);
    if (m->stream != NULL) {
        ld_mf_stream_detach(m->stream);
        m->stream->iface.lpVtbl->Release(&m->stream->iface);
    }
    free(m->pend);
    ma_free(m, pAlloc);
}

/* Asks the reader for float output, falling back to integer PCM if a decoder
   only offers that (miniaudio converts either to the float the app uses). */
static ma_result ld_mf_choose_format(ld_mf* m)
{
    IMFSourceReader* rd = m->reader;
    IMFMediaType* want = NULL;
    IMFMediaType* got = NULL;
    HRESULT hr;
    UINT32 bits = 0, align = 0;
    GUID subtype;

    rd->lpVtbl->SetStreamSelection(rd, (DWORD)MF_SOURCE_READER_ALL_STREAMS, FALSE);
    hr = rd->lpVtbl->SetStreamSelection(rd, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, TRUE);
    if (FAILED(hr)) return MA_INVALID_FILE;   /* no audio stream (e.g. a video-only MP4) */

    if (FAILED(MFCreateMediaType(&want))) return MA_OUT_OF_MEMORY;
    want->lpVtbl->SetGUID(want, &MF_MT_MAJOR_TYPE, &MFMediaType_Audio);
    want->lpVtbl->SetGUID(want, &MF_MT_SUBTYPE, &MFAudioFormat_Float);
    /* Only the sample type is set, so rate and channel count stay the file's own. */
    hr = rd->lpVtbl->SetCurrentMediaType(rd, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, NULL, want);
    if (FAILED(hr)) {
        want->lpVtbl->SetGUID(want, &MF_MT_SUBTYPE, &MFAudioFormat_PCM);
        hr = rd->lpVtbl->SetCurrentMediaType(rd, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, NULL, want);
    }
    want->lpVtbl->Release(want);
    if (FAILED(hr)) return MA_INVALID_FILE;   /* no installed decoder for this codec */

    hr = rd->lpVtbl->GetCurrentMediaType(rd, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, &got);
    if (FAILED(hr)) return MA_INVALID_FILE;
    got->lpVtbl->GetGUID(got, &MF_MT_SUBTYPE, &subtype);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_NUM_CHANNELS, &m->channels);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_SAMPLES_PER_SECOND, &m->sampleRate);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_BITS_PER_SAMPLE, &bits);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_BLOCK_ALIGNMENT, &align);
    got->lpVtbl->Release(got);

    if (IsEqualGUID(&subtype, &MFAudioFormat_Float) && bits == 32) m->format = ma_format_f32;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 16) m->format = ma_format_s16;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 24) m->format = ma_format_s24;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 32) m->format = ma_format_s32;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 8)  m->format = ma_format_u8;
    else return MA_INVALID_FILE;

    if (m->channels == 0 || m->channels > MA_MAX_CHANNELS || m->sampleRate == 0) return MA_INVALID_FILE;
    m->bytesPerFrame = m->channels * (bits / 8);
    if (align != 0 && align != m->bytesPerFrame) return MA_INVALID_FILE;
    return MA_SUCCESS;
}

/* Builds the decoder on an opened byte stream whose content sniffed as `kind`.
   Consumes the caller's reference to `bs`; on success also takes over
   `stream`'s reference. */
static ma_result ld_mf_open(IMFByteStream* bs, ld_mf_kind kind, QWORD audioStart, ld_mf_stream* stream, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    ma_result r = MA_INVALID_FILE;
    ld_mf* m = NULL;
    ma_int64 start = 0;
    double trimSkip = 0.0, trimLen = 0.0, adtsLen = 0.0;
    ma_bool32 haveTrim = MA_FALSE;
    /* Container lengths, read now: once the reader exists it reads the same
       byte stream on its own threads, and the position would be shared. */
    if (kind == LD_MF_MP4) haveTrim = ld_mp4_trim(bs, &trimSkip, &trimLen);
    if (kind == LD_MF_ADTS) adtsLen = ld_adts_secs(bs, audioStart);

    m = (ld_mf*)ma_malloc(sizeof(ld_mf), pAlloc);
    if (m == NULL) {
        bs->lpVtbl->Release(bs);
        return MA_OUT_OF_MEMORY;
    }
    memset(m, 0, sizeof(*m));
    m->exactTimestamps = (kind == LD_MF_MP4);

    /* MF picks the container parser from the byte stream's MIME type (there is
       no file name to go on in the onInit path). Some Windows builds register
       a parser under only one of the common names, so try each in turn. */
    {
        static const wchar_t* mp4[]  = { L"audio/mp4", L"video/mp4", NULL };
        static const wchar_t* adts[] = { L"audio/aac", L"audio/vnd.dlna.adts", L"audio/aacp", NULL };
        static const wchar_t* asf[]  = { L"audio/x-ms-wma", L"video/x-ms-asf", NULL };
        const wchar_t** mimes = (kind == LD_MF_MP4) ? mp4 : (kind == LD_MF_ADTS) ? adts : asf;
        IMFAttributes* attrs = NULL;
        bs->lpVtbl->QueryInterface(bs, &IID_IMFAttributes, (void**)&attrs);
        for (int i = 0; mimes[i] != NULL && m->reader == NULL; i++) {
            if (attrs != NULL) attrs->lpVtbl->SetString(attrs, &MF_BYTESTREAM_CONTENT_TYPE, mimes[i]);
            bs->lpVtbl->SetCurrentPosition(bs, 0);
            if (FAILED(MFCreateSourceReaderFromByteStream(bs, NULL, &m->reader))) m->reader = NULL;
        }
        if (attrs != NULL) attrs->lpVtbl->Release(attrs);
    }
    bs->lpVtbl->Release(bs);    /* the reader keeps its own reference */
    if (m->reader == NULL) goto fail;

    r = ld_mf_choose_format(m);
    if (r != MA_SUCCESS) goto fail;

    /* Decode the first chunk now: proves a decoder is installed and working
       (Windows "N" editions lack some), and fixes the timestamp origin, i.e.
       which timestamp counts as frame 0:
         - an MP4 trim: the priming frames after the first chunk's start are
           cut, so frame 0 is that many frames in (skip_to drops them);
         - MF already gave the priming negative timestamps: keep origin 0 and
           let skip_to drop everything before it, with no trim on top;
         - otherwise the first chunk's start, so a file whose audio starts
           after timestamp 0 does not gain leading silence. */
    r = ld_mf_fetch(m, &start);
    if (r != MA_SUCCESS) { if (r == MA_AT_END) r = MA_INVALID_FILE; goto fail; }
    if (start >= 0) {
        ma_int64 skip = haveTrim ? (ma_int64)llround(trimSkip * m->sampleRate) : 0;
        m->origin = start + skip;
        start = -skip;
    }
    m->firstStart = start;
    r = ld_mf_skip_to(m, start, 0);
    if (r != MA_SUCCESS) goto fail;

    if (haveTrim) {
        /* Exact, and the end trim is enforced in read. */
        m->length = (ma_uint64)llround(trimLen * m->sampleRate);
        m->stopAtLength = MA_TRUE;
    } else if (adtsLen > 0.0) {
        m->length = (ma_uint64)llround(adtsLen * m->sampleRate);
    } else {
        /* The container's declared duration (100 ns units). For ASF this is
           stored to the millisecond and may come out a few ms short of what
           decodes; reads still run to the real end. */
        PROPVARIANT pv;
        PropVariantInit(&pv);
        if (SUCCEEDED(m->reader->lpVtbl->GetPresentationAttribute(m->reader, (DWORD)MF_SOURCE_READER_MEDIASOURCE,
                                                                  &MF_PD_DURATION, &pv)) && pv.vt == VT_UI8) {
            ma_int64 frames = ld_mf_ts_to_frames((LONGLONG)pv.uhVal.QuadPart, m->sampleRate) - m->origin;
            m->length = (frames > 0) ? (ma_uint64)frames : 0;
        }
        PropVariantClear(&pv);
    }

    {
        ma_data_source_config cfg = ma_data_source_config_init();
        cfg.vtable = &g_ld_mf_ds_vtable;
        r = ma_data_source_init(&cfg, &m->ds);
        if (r != MA_SUCCESS) goto fail;
    }
    m->stream = stream;
    *ppBackend = (ma_data_source*)m;
    return MA_SUCCESS;

fail:
    ld_mf_free(m, pAlloc);
    return (r == MA_SUCCESS) ? MA_INVALID_FILE : r;
}

static ma_result ld_mf_backend_init(void* pUserData, ma_read_proc onRead, ma_seek_proc onSeek, ma_tell_proc onTell, void* pReadSeekTellUserData, const ma_decoding_backend_config* pConfig, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    (void)pUserData; (void)pConfig;
    if (onRead == NULL || onSeek == NULL || onTell == NULL) return MA_INVALID_ARGS;

    ld_mf_stream* s = ld_mf_stream_create(onRead, onSeek, onTell, pReadSeekTellUserData);
    if (s == NULL) return MA_INVALID_FILE;
    QWORD audioStart = 0;
    ld_mf_kind kind = ld_mf_sniff(ld_istream_read_at, &s->iface, &audioStart);
    if (kind == LD_MF_NONE) {
        s->iface.lpVtbl->Release(&s->iface);    /* MF never saw it */
        return MA_INVALID_FILE;
    }

    int com = ld_mf_com_enter();
    ma_result r = MA_NOT_IMPLEMENTED;
    IMFByteStream* bs = NULL;
    if (!ld_mf_ready()) {
        r = MA_NOT_IMPLEMENTED;
    } else if (FAILED(MFCreateMFByteStreamOnStream(&s->iface, &bs))) {
        r = MA_ERROR;
    } else {
        r = ld_mf_open(bs, kind, audioStart, s, pAlloc, ppBackend);
    }
    /* On success the decoder owns `s`; otherwise drop it, cutting it off
       first in case MF still holds a reference. */
    if (r != MA_SUCCESS) {
        ld_mf_stream_detach(s);
        s->iface.lpVtbl->Release(&s->iface);
    }
    ld_mf_com_leave(com);
    return r;
}

static ma_result ld_mf_backend_init_file_w(void* pUserData, const wchar_t* pFilePath, const ma_decoding_backend_config* pConfig, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    (void)pUserData; (void)pConfig;
    int com = ld_mf_com_enter();
    ma_result r = MA_NOT_IMPLEMENTED;
    if (ld_mf_ready()) {
        IMFByteStream* bs = NULL;
        /* MFCreateFile takes a wide path, so Unicode names open correctly. */
        if (FAILED(MFCreateFile(MF_ACCESSMODE_READ, MF_OPENMODE_FAIL_IF_NOT_EXIST, MF_FILEFLAGS_NONE, pFilePath, &bs))) {
            r = MA_DOES_NOT_EXIST;
        } else {
            QWORD audioStart = 0;
            ld_mf_kind kind = ld_mf_sniff(ld_bytestream_read_at, bs, &audioStart);
            if (kind == LD_MF_NONE) {
                bs->lpVtbl->Release(bs);
                r = MA_INVALID_FILE;
            } else {
                r = ld_mf_open(bs, kind, audioStart, NULL, pAlloc, ppBackend);
            }
        }
    }
    ld_mf_com_leave(com);
    return r;
}

static void ld_mf_backend_uninit(void* pUserData, ma_data_source* pBackend, const ma_allocation_callbacks* pAlloc)
{
    (void)pUserData;
    ld_mf* m = (ld_mf*)pBackend;
    int com = ld_mf_com_enter();
    ma_data_source_uninit(&m->ds);
    ld_mf_free(m, pAlloc);
    ld_mf_com_leave(com);
}

static ma_decoding_backend_vtable g_ld_mf_backend_vtable =
{
    ld_mf_backend_init,
    NULL,   /* onInitFile: only wide paths are used (see shim.c) */
    ld_mf_backend_init_file_w,
    NULL,   /* onInitMemory */
    ld_mf_backend_uninit
};

ma_decoding_backend_vtable* ld_decoding_backend_mf = &g_ld_mf_backend_vtable;

#endif /* miniaudio_mf_c */
