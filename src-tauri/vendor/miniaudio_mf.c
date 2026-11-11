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

/* ---- Byte source ----------------------------------------------------------------
 *
 * Where the file's bytes come from: a Win32 file handle (the path open) or
 * miniaudio's read/seek callbacks (the resource manager, which never passes a
 * path). Everything reads through ld_src_read at an explicit offset, so the
 * container parsing below and any number of MF readers can share one source.
 *
 * MF reads ahead on its own worker threads, so reads can arrive on a thread
 * other than the one decoding, and may still arrive after the decoder closed.
 * The lock serialises them, and `detached` (set on uninit, before miniaudio
 * closes its file and before our handle is closed) makes any late read fail
 * instead of touching a closed file. */

typedef struct ld_mf_src
{
    volatile LONG refs;
    SRWLOCK lock;
    int detached;
    ma_uint64 size;             /* Total bytes, measured once at creation. */
    HANDLE file;                /* The path open; INVALID_HANDLE_VALUE for callbacks. */
    ma_read_proc onRead;
    ma_seek_proc onSeek;
    void* ud;
} ld_mf_src;

static ld_mf_src* ld_src_alloc(ma_uint64 size)
{
    ld_mf_src* s = (ld_mf_src*)calloc(1, sizeof(ld_mf_src));
    if (s == NULL) return NULL;
    s->refs = 1;
    InitializeSRWLock(&s->lock);
    s->size = size;
    s->file = INVALID_HANDLE_VALUE;
    return s;
}

static ld_mf_src* ld_src_from_file(const wchar_t* path)
{
    /* Wide path: Unicode names open correctly. Sharing everything matches what
       other readers of the library (and the user's tools) expect. */
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                           NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    LARGE_INTEGER size;
    if (h == INVALID_HANDLE_VALUE) return NULL;
    if (!GetFileSizeEx(h, &size) || size.QuadPart <= 0) { CloseHandle(h); return NULL; }
    ld_mf_src* s = ld_src_alloc((ma_uint64)size.QuadPart);
    if (s == NULL) { CloseHandle(h); return NULL; }
    s->file = h;
    return s;
}

static ld_mf_src* ld_src_from_callbacks(ma_read_proc onRead, ma_seek_proc onSeek, ma_tell_proc onTell, void* ud)
{
    ma_int64 size = 0;
    if (onSeek(ud, 0, ma_seek_origin_end) != MA_SUCCESS || onTell(ud, &size) != MA_SUCCESS || size <= 0) return NULL;
    if (onSeek(ud, 0, ma_seek_origin_start) != MA_SUCCESS) return NULL;
    ld_mf_src* s = ld_src_alloc((ma_uint64)size);
    if (s == NULL) return NULL;
    s->onRead = onRead;
    s->onSeek = onSeek;
    s->ud = ud;
    return s;
}

static void ld_src_addref(ld_mf_src* s) { InterlockedIncrement(&s->refs); }

static void ld_src_release(ld_mf_src* s)
{
    if (InterlockedDecrement(&s->refs) != 0) return;
    if (s->file != INVALID_HANDLE_VALUE) CloseHandle(s->file);
    free(s);
}

/* Cuts the source off from the file; later reads fail. */
static void ld_src_detach(ld_mf_src* s)
{
    AcquireSRWLockExclusive(&s->lock);
    s->detached = 1;
    if (s->file != INVALID_HANDLE_VALUE) {
        CloseHandle(s->file);
        s->file = INVALID_HANDLE_VALUE;
    }
    ReleaseSRWLockExclusive(&s->lock);
}

/* Reads up to n bytes at `off` (fewer at the end of the file). */
static HRESULT ld_src_read(ld_mf_src* s, QWORD off, void* buf, ULONG n, ULONG* pGot)
{
    HRESULT hr = S_OK;
    ULONG total = 0;
    if (off >= s->size) n = 0;
    else if (n > s->size - off) n = (ULONG)(s->size - off);

    AcquireSRWLockExclusive(&s->lock);
    if (s->detached) {
        hr = E_FAIL;
    } else if (n > 0 && s->file != INVALID_HANDLE_VALUE) {
        /* OVERLAPPED here only carries the offset; the handle is synchronous. */
        OVERLAPPED ov;
        memset(&ov, 0, sizeof(ov));
        ov.Offset = (DWORD)off;
        ov.OffsetHigh = (DWORD)(off >> 32);
        DWORD got = 0;
        if (!ReadFile(s->file, buf, n, &got, &ov) && GetLastError() != ERROR_HANDLE_EOF) hr = E_FAIL;
        total = got;
    } else if (n > 0) {
        if (s->onSeek(s->ud, (ma_int64)off, ma_seek_origin_start) != MA_SUCCESS) {
            hr = E_FAIL;
        } else {
            /* A callback may return fewer bytes than asked; keep going until
               the request is filled or the stream ends. */
            while (total < n) {
                size_t got = 0;
                ma_result r = s->onRead(s->ud, (ma_uint8*)buf + total, n - total, &got);
                total += (ULONG)got;
                if (got == 0) {
                    if (r != MA_SUCCESS && r != MA_AT_END) hr = E_FAIL;
                    break;
                }
            }
        }
    }
    ReleaseSRWLockExclusive(&s->lock);
    *pGot = total;
    return hr;
}

static ma_bool32 ld_src_read_exact(ld_mf_src* s, QWORD off, void* buf, ULONG n)
{
    ULONG got = 0;
    return SUCCEEDED(ld_src_read(s, off, buf, n, &got)) && got == n;
}

/* ---- IStream view of a byte source ------------------------------------------------
 *
 * MF can read from any COM IStream (via MFCreateMFByteStreamOnStream). A view
 * presents the source from byte `base` onward as a whole stream, which is how
 * an ADTS seek starts a fresh reader at a frame boundary (see ld_mf_seek).
 *
 * A COM object in C is a struct whose first member points at a table of
 * function pointers (the "vtable"); callers only ever go through that table.
 * Reference counting (AddRef/Release) decides when it is freed, because MF
 * keeps its own references and may release them after our decoder is gone. */

typedef struct ld_mf_view
{
    IStream iface;              /* MUST be first: COM callers see this struct as an IStream*. */
    volatile LONG refs;
    SRWLOCK lock;               /* guards pos */
    ld_mf_src* src;             /* holds a reference */
    QWORD base;
    QWORD pos;                  /* relative to base */
} ld_mf_view;

static QWORD ld_view_size(ld_mf_view* v) { return v->src->size - v->base; }

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
    return (ULONG)InterlockedIncrement(&((ld_mf_view*)This)->refs);
}

static ULONG STDMETHODCALLTYPE ld_mfs_Release(IStream* This)
{
    ld_mf_view* v = (ld_mf_view*)This;
    LONG n = InterlockedDecrement(&v->refs);
    if (n == 0) {
        ld_src_release(v->src);
        free(v);
    }
    return (ULONG)n;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Read(IStream* This, void* pv, ULONG cb, ULONG* pcbRead)
{
    ld_mf_view* v = (ld_mf_view*)This;
    ULONG got = 0;
    AcquireSRWLockExclusive(&v->lock);
    HRESULT hr = ld_src_read(v->src, v->base + v->pos, pv, cb, &got);
    v->pos += got;
    ReleaseSRWLockExclusive(&v->lock);
    if (pcbRead != NULL) *pcbRead = got;
    if (FAILED(hr)) return hr;
    return (got == cb) ? S_OK : S_FALSE;   /* S_FALSE = short read, end of stream */
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Write(IStream* This, const void* pv, ULONG cb, ULONG* pcbWritten)
{
    (void)This; (void)pv; (void)cb;
    if (pcbWritten != NULL) *pcbWritten = 0;
    return STG_E_ACCESSDENIED;   /* read-only */
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Seek(IStream* This, LARGE_INTEGER move, DWORD origin, ULARGE_INTEGER* newPos)
{
    ld_mf_view* v = (ld_mf_view*)This;
    HRESULT hr = S_OK;
    AcquireSRWLockExclusive(&v->lock);
    LONGLONG from = (origin == STREAM_SEEK_SET) ? 0
                  : (origin == STREAM_SEEK_END) ? (LONGLONG)ld_view_size(v)
                  : (LONGLONG)v->pos;
    LONGLONG to = from + move.QuadPart;
    if (to < 0) hr = STG_E_INVALIDFUNCTION;
    else v->pos = (QWORD)to;
    if (SUCCEEDED(hr) && newPos != NULL) newPos->QuadPart = v->pos;
    ReleaseSRWLockExclusive(&v->lock);
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
    st->cbSize.QuadPart = ld_view_size((ld_mf_view*)This);
    st->grfMode = STGM_READ;
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE ld_mfs_Clone(IStream* This, IStream** out)
{
    (void)This;
    if (out != NULL) *out = NULL;
    return E_NOTIMPL;
}

static IStreamVtbl g_ld_mf_view_vtbl =
{
    ld_mfs_QueryInterface, ld_mfs_AddRef, ld_mfs_Release,
    ld_mfs_Read, ld_mfs_Write,
    ld_mfs_Seek, ld_mfs_SetSize, ld_mfs_CopyTo, ld_mfs_Commit, ld_mfs_Revert,
    ld_mfs_LockRegion, ld_mfs_UnlockRegion, ld_mfs_Stat, ld_mfs_Clone
};

/* Returns a view holding one reference, or NULL. */
static ld_mf_view* ld_view_create(ld_mf_src* src, QWORD base)
{
    if (base >= src->size) return NULL;
    ld_mf_view* v = (ld_mf_view*)calloc(1, sizeof(ld_mf_view));
    if (v == NULL) return NULL;
    v->iface.lpVtbl = &g_ld_mf_view_vtbl;
    v->refs = 1;
    InitializeSRWLock(&v->lock);
    ld_src_addref(src);
    v->src = src;
    v->base = base;
    return v;
}

/* ---- Content sniffing --------------------------------------------------------------
 *
 * Done on the byte source before any MF object exists, so files this backend
 * rejects (every MP3/WAV/FLAC played) cost only a few small reads. */

typedef enum { LD_MF_NONE, LD_MF_MP4, LD_MF_ADTS, LD_MF_ASF } ld_mf_kind;

/* An ADTS frame header: 12 sync bits, then the 2 "layer" bits, always 00 for
   AAC. MP3 frames share the sync bits but have a nonzero layer, which is what
   keeps MP3 files away from this backend. */
static ma_bool32 ld_mf_is_adts_header(const BYTE* h)
{
    return h[0] == 0xFF && (h[1] & 0xF6) == 0xF0;
}

/* Byte length of the ADTS frame whose header is at h (13 bits at bits 30..42). */
static QWORD ld_adts_frame_len(const BYTE* h)
{
    return ((QWORD)(h[3] & 0x03) << 11) | ((QWORD)h[4] << 3) | ((QWORD)h[5] >> 5);
}

/* Also reports where the audio data starts (past any ID3 tags) in *pAudioStart. */
static ld_mf_kind ld_mf_sniff(ld_mf_src* src, QWORD* pAudioStart)
{
    /* First 16 bytes of the ASF header object's GUID. */
    static const BYTE asf_guid[16] = { 0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11,
                                       0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C };
    BYTE h[16];
    QWORD off = 0;

    *pAudioStart = 0;
    if (!ld_src_read_exact(src, 0, h, sizeof(h))) return LD_MF_NONE;
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
        if (!ld_src_read_exact(src, off, h, sizeof(h))) return LD_MF_NONE;
    }

    if (ld_mf_is_adts_header(h)) {
        /* Confirm with the next frame's header. */
        QWORD len = ld_adts_frame_len(h);
        BYTE next[2];
        if (len >= 7 && ld_src_read_exact(src, off + len, next, 2) && ld_mf_is_adts_header(next)) {
            *pAudioStart = off;
            return LD_MF_ADTS;
        }
    }
    return LD_MF_NONE;
}

/* ---- MP4: the trim and length the source reader doesn't provide ---------------------
 *
 * AAC encoders add "priming" (warm-up) frames at the start of the stream and
 * pad the end up to a whole 1024-frame packet. An MP4 file records how much of
 * that to cut, but MF's source reader ignores it and returns every decoded
 * frame. Left alone, playback would start ~23-50 ms late and lengths would
 * disagree with what decodes, so the trim is read from the file here and
 * applied in the decoder (see ld_mf_open). */

static ma_uint32 ld_be32(const BYTE* p) { return ((ma_uint32)p[0] << 24) | ((ma_uint32)p[1] << 16) | ((ma_uint32)p[2] << 8) | p[3]; }
static ma_uint64 ld_be64(const BYTE* p) { return ((ma_uint64)ld_be32(p) << 32) | ld_be32(p + 4); }

/* MP4 files are a tree of "boxes": a 4-byte big-endian size (the whole box,
   header included), a 4-character type, then the payload, which for container
   boxes is more boxes. Finds the next box of `type` among the boxes laid out
   in [*pFrom, end). On success sets [*pStart, *pEnd) to its payload and moves
   *pFrom past it, so repeated calls walk siblings of the same type. */
static ma_bool32 ld_mp4_next(ld_mf_src* src, QWORD* pFrom, QWORD end, const char* type, QWORD* pStart, QWORD* pEnd)
{
    QWORD off = *pFrom;
    while (off + 8 <= end) {
        BYTE h[16];
        if (!ld_src_read_exact(src, off, h, 8)) return MA_FALSE;
        QWORD size = ld_be32(h), hdr = 8;
        if (size == 1) {                    /* 64-bit size follows the type */
            if (!ld_src_read_exact(src, off + 8, h + 8, 8)) return MA_FALSE;
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

static ma_bool32 ld_mp4_find(ld_mf_src* src, QWORD from, QWORD end, const char* type, QWORD* pStart, QWORD* pEnd)
{
    return ld_mp4_next(src, &from, end, type, pStart, pEnd);
}

/* A duration field with no real value. Fragmented MP4 files (written for
   streaming) keep their samples in later "moof" boxes and leave the header
   durations 0, or all ones ("unknown", e.g. ffmpeg's ismv output). */
static ma_bool32 ld_mp4_dur_unknown(ma_uint64 dur, ma_bool32 v1)
{
    return dur == 0 || dur == (v1 ? 0xFFFFFFFFFFFFFFFFull : 0xFFFFFFFFull);
}

/* Reads "mvhd"/"mdhd": a version byte and 3 flag bytes, then (version 1)
   64-bit creation/modification times or (version 0) 32-bit ones, then the
   32-bit timescale (ticks per second) and the duration in those ticks.
   *pDuration is 0 when the field holds no real value. */
static ma_bool32 ld_mp4_read_header(ld_mf_src* src, QWORD start, ma_uint32* pTimescale, ma_uint64* pDuration)
{
    BYTE b[32];
    if (!ld_src_read_exact(src, start, b, 1)) return MA_FALSE;
    ma_bool32 v1 = (b[0] == 1);
    if (v1) {
        if (!ld_src_read_exact(src, start, b, 32)) return MA_FALSE;
        *pTimescale = ld_be32(b + 20);
        *pDuration = ld_be64(b + 24);
    } else {
        if (!ld_src_read_exact(src, start, b, 20)) return MA_FALSE;
        *pTimescale = ld_be32(b + 12);
        *pDuration = ld_be32(b + 16);
    }
    if (ld_mp4_dur_unknown(*pDuration, v1)) *pDuration = 0;
    return *pTimescale != 0;
}

/* A fragmented file may still state its total duration, in movie ticks, in
   moov/mvex/mehd (version/flags word, then 32 or 64 bits). 0 if absent. */
static ma_uint64 ld_mp4_mehd(ld_mf_src* src, QWORD moovStart, QWORD moovEnd)
{
    QWORD s, e, s2, e2;
    BYTE b[12];
    if (!ld_mp4_find(src, moovStart, moovEnd, "mvex", &s, &e)) return 0;
    if (!ld_mp4_find(src, s, e, "mehd", &s2, &e2) || !ld_src_read_exact(src, s2, b, 4)) return 0;
    ma_bool32 v1 = (b[0] == 1);
    if (!ld_src_read_exact(src, s2 + 4, b + 4, v1 ? 8 : 4)) return 0;
    ma_uint64 dur = v1 ? ld_be64(b + 4) : ld_be32(b + 4);
    return ld_mp4_dur_unknown(dur, v1) ? 0 : dur;
}

/* Apple's encoder records the trim in an iTunes metadata item named
   "iTunSMPB" instead of an edit list: hex text whose 2nd, 3rd and 4th fields
   are the priming frames, the padding frames and the real frame count. It
   lives at moov/udta/meta/ilst/"----", next to "name" and "data" children. */
static ma_bool32 ld_mp4_itunsmpb(ld_mf_src* src, QWORD moovStart, QWORD moovEnd, ma_uint64* pSkip, ma_uint64* pFrames)
{
    QWORD a, b, c, d, e, f, from, itemStart, itemEnd;
    if (!ld_mp4_find(src, moovStart, moovEnd, "udta", &a, &b)) return MA_FALSE;
    if (!ld_mp4_find(src, a, b, "meta", &c, &d)) return MA_FALSE;
    /* "meta" has a version/flags word before its children. */
    if (!ld_mp4_find(src, c + 4, d, "ilst", &e, &f)) return MA_FALSE;
    from = e;
    while (ld_mp4_next(src, &from, f, "----", &itemStart, &itemEnd)) {
        QWORD ns, ne, ds, de;
        char name[8];
        char text[128] = { 0 };
        unsigned int zero, priming, padding;
        unsigned long long frames;
        /* "name" payload: version/flags (4), then the name. */
        if (!ld_mp4_find(src, itemStart, itemEnd, "name", &ns, &ne) || ne - ns != 4 + 8) continue;
        if (!ld_src_read_exact(src, ns + 4, name, 8) || memcmp(name, "iTunSMPB", 8) != 0) continue;
        /* "data" payload: type (4), locale (4), then the text. */
        if (!ld_mp4_find(src, itemStart, itemEnd, "data", &ds, &de) || de - ds <= 8) return MA_FALSE;
        ULONG n = (ULONG)((de - ds - 8 < sizeof(text) - 1) ? de - ds - 8 : sizeof(text) - 1);
        if (!ld_src_read_exact(src, ds + 8, text, n)) return MA_FALSE;
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

/* The trim for the first sound track of an MP4 file, in seconds. Seconds
   rather than frames because this runs before the decoder's output rate is
   known. */
typedef struct
{
    double skip;    /* drop this much from the start */
    double len;     /* real audio after that; 0 = unknown, read to the end */
} ld_mp4_trim_t;

/* Sources, in order of preference:
     - the track's edit list ("elst"): one entry giving the media time where
       playback starts and how long it lasts (ffmpeg and most tools);
     - iTunSMPB (Apple's encoder);
     - neither: no trim, length = the track's full duration.
   The track's full duration comes from "mdhd", or for a fragmented file from
   "mehd"; if neither states it, the length stays unknown and only the start
   trim applies. Returns MA_FALSE if the file can't be parsed; the caller then
   keeps MF's numbers. */
static ma_bool32 ld_mp4_trim(ld_mf_src* src, ld_mp4_trim_t* t)
{
    QWORD moovStart, moovEnd, from, trakStart, trakEnd, s, e, s2, e2;
    ma_uint32 movieScale = 0, mediaScale = 0;
    ma_uint64 movieDur = 0, mediaDur = 0, skipTicks = 0, lenTicks = 0;
    double total, skip, len;

    t->skip = 0.0;
    t->len = 0.0;
    if (!ld_mp4_find(src, 0, src->size, "moov", &moovStart, &moovEnd)) return MA_FALSE;
    if (!ld_mp4_find(src, moovStart, moovEnd, "mvhd", &s, &e) || !ld_mp4_read_header(src, s, &movieScale, &movieDur)) return MA_FALSE;

    /* The first track whose handler ("hdlr") is "soun", i.e. audio. */
    from = moovStart;
    for (;;) {
        BYTE handler[4];
        if (!ld_mp4_next(src, &from, moovEnd, "trak", &trakStart, &trakEnd)) return MA_FALSE;
        if (!ld_mp4_find(src, trakStart, trakEnd, "mdia", &s, &e)) continue;
        /* hdlr payload: version/flags (4), pre_defined (4), handler type (4). */
        if (!ld_mp4_find(src, s, e, "hdlr", &s2, &e2) || !ld_src_read_exact(src, s2 + 8, handler, 4)) continue;
        if (memcmp(handler, "soun", 4) != 0) continue;
        if (!ld_mp4_find(src, s, e, "mdhd", &s2, &e2) || !ld_mp4_read_header(src, s2, &mediaScale, &mediaDur)) return MA_FALSE;
        break;
    }
    total = ld_mp4_secs(mediaDur, mediaScale);
    if (total <= 0.0) total = ld_mp4_secs(ld_mp4_mehd(src, moovStart, moovEnd), movieScale);
    /* total <= 0 from here on means "unknown". */

    if (ld_mp4_find(src, trakStart, trakEnd, "edts", &s, &e) && ld_mp4_find(src, s, e, "elst", &s2, &e2)) {
        /* elst payload: version/flags (4), entry count (4), then entries of
           segment duration (movie timescale), media time (media timescale,
           -1 for an empty edit) and a 4-byte rate; 64-bit fields in version 1.
           Only the common single-entry form is handled. */
        BYTE b[20];
        if (ld_src_read_exact(src, s2, b, 8) && ld_be32(b + 4) == 1) {
            ma_bool32 v1 = (b[0] == 1);
            if (ld_src_read_exact(src, s2 + 8, b, v1 ? 16 : 8)) {
                ma_uint64 segDur = v1 ? ld_be64(b) : ld_be32(b);
                ma_int64 mediaTime = v1 ? (ma_int64)ld_be64(b + 8) : (ma_int64)(ma_int32)ld_be32(b + 4);
                if (mediaTime >= 0) {
                    skip = ld_mp4_secs((ma_uint64)mediaTime, mediaScale);
                    /* A zero (or "unknown") segment duration means "to the end". */
                    len = ld_mp4_dur_unknown(segDur, v1) ? 0.0 : ld_mp4_secs(segDur, movieScale);
                    if (total > 0.0) {
                        if (skip > total) skip = total;
                        if (len <= 0.0 || len > total - skip) len = total - skip;
                    }
                    t->skip = skip;
                    t->len = len;
                    return MA_TRUE;
                }
            }
        }
    }

    if (ld_mp4_itunsmpb(src, moovStart, moovEnd, &skipTicks, &lenTicks)) {
        skip = ld_mp4_secs(skipTicks, mediaScale);
        len = ld_mp4_secs(lenTicks, mediaScale);
        if (total <= 0.0 || (skip <= total && len <= total - skip)) {
            t->skip = skip;
            t->len = len;
            return MA_TRUE;
        }
    }

    t->len = (total > 0.0) ? total : 0.0;
    return MA_TRUE;
}

/* ---- ADTS: exact length and a seek index ---------------------------------------------
 *
 * Raw ADTS AAC has no index or duration field. MF estimates both from the
 * bitrate, so its length is approximate and, measured, its seeks land up to
 * ~130 ms from where its timestamps claim. Walking the frame headers once at
 * open gives the exact length and, every LD_ADTS_MARK_EVERY frames, a "mark":
 * a byte offset where a frame starts plus the sample position it starts at.
 * Every ADTS frame is self-contained, so a fresh reader can start decoding at
 * any mark; that is how ADTS seeks are made exact (see ld_mf_seek). */

#define LD_ADTS_MARK_EVERY 8

typedef struct
{
    QWORD off;          /* byte offset of the frame header */
    ma_uint64 pos;      /* first sample of that frame, at the stream's own (core) rate */
} ld_adts_mark;

typedef struct
{
    ld_adts_mark* marks;
    size_t count;
    ma_uint64 total;    /* samples at the core rate */
    ma_uint32 coreRate; /* the rate in the headers; an HE-AAC decoder outputs twice this */
} ld_adts_index;

/* Reads in 64 KB chunks, so even a long file takes only a few ms. Returns
   MA_FALSE if not even one frame could be walked. */
static ma_bool32 ld_adts_scan(ld_mf_src* src, QWORD audioStart, ld_adts_index* idx)
{
    static const ma_uint32 rates[16] = { 96000, 88200, 64000, 48000, 44100, 32000, 24000,
                                         22050, 16000, 12000, 11025, 8000, 7350, 0, 0, 0 };
    enum { CHUNK = 1 << 16 };
    BYTE* buf = (BYTE*)malloc(CHUNK);
    QWORD bufStart = 0, pos = audioStart;
    ULONG bufLen = 0;
    size_t cap = 0, frame = 0;
    memset(idx, 0, sizeof(*idx));
    if (buf == NULL) return MA_FALSE;

    for (;; frame++) {
        /* Keep the 7-byte header at `pos` inside the buffer. */
        if (pos < bufStart || pos + 7 > bufStart + bufLen) {
            if (FAILED(ld_src_read(src, pos, buf, CHUNK, &bufLen)) || bufLen < 7) break;
            bufStart = pos;
        }
        const BYTE* h = buf + (pos - bufStart);
        if (!ld_mf_is_adts_header(h)) break;     /* a trailing tag or junk ends the walk */
        QWORD len = ld_adts_frame_len(h);
        if (len < 7) break;
        if (idx->coreRate == 0) idx->coreRate = rates[(h[2] >> 2) & 0x0F];
        if (frame % LD_ADTS_MARK_EVERY == 0) {
            if (idx->count == cap) {
                size_t grown = cap ? cap * 2 : 256;
                ld_adts_mark* p = (ld_adts_mark*)realloc(idx->marks, grown * sizeof(ld_adts_mark));
                if (p == NULL) break;
                idx->marks = p;
                cap = grown;
            }
            idx->marks[idx->count].off = pos;
            idx->marks[idx->count].pos = idx->total;
            idx->count++;
        }
        /* Each frame carries 1-4 raw blocks of 1024 samples. */
        idx->total += ((ma_uint64)(h[6] & 0x03) + 1) * 1024;
        pos += len;
    }
    free(buf);
    if (idx->count == 0 || idx->coreRate == 0) {
        free(idx->marks);
        memset(idx, 0, sizeof(*idx));
        return MA_FALSE;
    }
    return MA_TRUE;
}

/* ---- The decoder ------------------------------------------------------------------ */

typedef struct
{
    ma_format format;
    ma_uint32 channels;
    ma_uint32 sampleRate;
    ma_uint32 bytesPerFrame;
} ld_mf_fmt;

typedef struct
{
    ma_data_source_base ds;     /* MUST be first: miniaudio treats ld_mf* as ma_data_source*. */
    ld_mf_src* src;
    ld_mf_kind kind;
    const wchar_t* mime;        /* the content type MF accepted at open */
    IMFSourceReader* reader;
    /* What miniaudio is told, fixed once the first chunk has decoded, versus
       what the current reader is producing right now. */
    ld_mf_fmt fmt;
    ld_mf_fmt readerFmt;
    ma_bool32 fmtLocked;
    ma_uint64 length;           /* Total frames; 0 = unknown (sources: end of ld_mf_open). */
    ma_bool32 stopAtLength;     /* `length` is exact: never hand out frames past it. */
    ma_uint64 cursor;           /* Frame index of the next frame read() returns. */
    ma_int64 origin;            /* Timestamp, in frames, that counts as frame 0. */
    ma_int64 firstStart;        /* Where the file's first decoded chunk starts, relative to origin. */
    ma_bool32 eof;
    ld_adts_index adts;         /* ADTS only */
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

/* Reads the reader's current output format into m->readerFmt. */
static ma_result ld_mf_query_format(ld_mf* m)
{
    IMFMediaType* got = NULL;
    UINT32 channels = 0, rate = 0, bits = 0, align = 0;
    GUID subtype;
    ld_mf_fmt f;

    if (FAILED(m->reader->lpVtbl->GetCurrentMediaType(m->reader, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, &got))) return MA_INVALID_FILE;
    got->lpVtbl->GetGUID(got, &MF_MT_SUBTYPE, &subtype);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_NUM_CHANNELS, &channels);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_SAMPLES_PER_SECOND, &rate);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_BITS_PER_SAMPLE, &bits);
    got->lpVtbl->GetUINT32(got, &MF_MT_AUDIO_BLOCK_ALIGNMENT, &align);
    got->lpVtbl->Release(got);

    if (IsEqualGUID(&subtype, &MFAudioFormat_Float) && bits == 32) f.format = ma_format_f32;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 16) f.format = ma_format_s16;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 24) f.format = ma_format_s24;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 32) f.format = ma_format_s32;
    else if (IsEqualGUID(&subtype, &MFAudioFormat_PCM) && bits == 8)  f.format = ma_format_u8;
    else return MA_INVALID_FILE;

    if (channels == 0 || channels > MA_MAX_CHANNELS || rate == 0) return MA_INVALID_FILE;
    f.channels = channels;
    f.sampleRate = rate;
    f.bytesPerFrame = channels * (bits / 8);
    if (align != 0 && align != f.bytesPerFrame) return MA_INVALID_FILE;
    m->readerFmt = f;
    return MA_SUCCESS;
}

/* Selects the audio stream and asks for float output, falling back to integer
   PCM if a decoder only offers that (miniaudio converts either to the float
   the app uses). Only the sample type is requested, so rate and channel count
   stay the file's own. */
static ma_result ld_mf_choose_format(ld_mf* m)
{
    IMFSourceReader* rd = m->reader;
    IMFMediaType* want = NULL;
    HRESULT hr;

    rd->lpVtbl->SetStreamSelection(rd, (DWORD)MF_SOURCE_READER_ALL_STREAMS, FALSE);
    hr = rd->lpVtbl->SetStreamSelection(rd, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, TRUE);
    if (FAILED(hr)) return MA_INVALID_FILE;   /* no audio stream (e.g. a video-only MP4) */

    if (FAILED(MFCreateMediaType(&want))) return MA_OUT_OF_MEMORY;
    want->lpVtbl->SetGUID(want, &MF_MT_MAJOR_TYPE, &MFMediaType_Audio);
    want->lpVtbl->SetGUID(want, &MF_MT_SUBTYPE, &MFAudioFormat_Float);
    hr = rd->lpVtbl->SetCurrentMediaType(rd, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, NULL, want);
    if (FAILED(hr)) {
        want->lpVtbl->SetGUID(want, &MF_MT_SUBTYPE, &MFAudioFormat_PCM);
        hr = rd->lpVtbl->SetCurrentMediaType(rd, (DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, NULL, want);
    }
    want->lpVtbl->Release(want);
    if (FAILED(hr)) return MA_INVALID_FILE;   /* no installed decoder for this codec */
    return ld_mf_query_format(m);
}

/* (Re)creates the source reader on the file from byte `base` onward (0 = the
   whole file; ADTS seeks start later, at a frame boundary). MF picks the
   container parser from the byte stream's MIME type, there being no file name
   to go on; some Windows builds register a parser under only one of the common
   names, so at open each is tried in turn, and the one that worked is reused. */
static ma_result ld_mf_open_reader(ld_mf* m, QWORD base)
{
    static const wchar_t* mp4[]  = { L"audio/mp4", L"video/mp4", NULL };
    static const wchar_t* adts[] = { L"audio/aac", L"audio/vnd.dlna.adts", L"audio/aacp", NULL };
    static const wchar_t* asf[]  = { L"audio/x-ms-wma", L"video/x-ms-asf", NULL };
    const wchar_t* only[] = { m->mime, NULL };
    const wchar_t** mimes = (m->mime != NULL) ? only : (m->kind == LD_MF_MP4) ? mp4 : (m->kind == LD_MF_ADTS) ? adts : asf;
    IMFByteStream* bs = NULL;
    IMFAttributes* attrs = NULL;

    if (m->reader != NULL) {
        m->reader->lpVtbl->Release(m->reader);
        m->reader = NULL;
    }
    m->eof = MA_FALSE;
    m->pendFrames = m->pendPos = 0;

    ld_mf_view* v = ld_view_create(m->src, base);
    if (v == NULL) return MA_INVALID_FILE;
    HRESULT hr = MFCreateMFByteStreamOnStream(&v->iface, &bs);
    v->iface.lpVtbl->Release(&v->iface);    /* the byte stream keeps its own reference */
    if (FAILED(hr)) return MA_ERROR;

    bs->lpVtbl->QueryInterface(bs, &IID_IMFAttributes, (void**)&attrs);
    for (int i = 0; mimes[i] != NULL && m->reader == NULL; i++) {
        if (attrs != NULL) attrs->lpVtbl->SetString(attrs, &MF_BYTESTREAM_CONTENT_TYPE, mimes[i]);
        bs->lpVtbl->SetCurrentPosition(bs, 0);
        if (FAILED(MFCreateSourceReaderFromByteStream(bs, NULL, &m->reader))) {
            m->reader = NULL;
        } else {
            m->mime = mimes[i];
        }
    }
    if (attrs != NULL) attrs->lpVtbl->Release(attrs);
    bs->lpVtbl->Release(bs);    /* the reader keeps its own reference */
    if (m->reader == NULL) return MA_INVALID_FILE;
    return ld_mf_choose_format(m);
}

static ma_bool32 ld_mf_fmt_equal(const ld_mf_fmt* a, const ld_mf_fmt* b)
{
    return a->format == b->format && a->channels == b->channels && a->sampleRate == b->sampleRate;
}

/* Decodes the next chunk into `pend` (replacing what was there) and reports
   where it starts, in frames relative to `origin`, from its timestamp.
   MA_AT_END once the stream is exhausted. */
static ma_result ld_mf_fetch(ld_mf* m, ma_int64* pStart)
{
    if (m->eof) return MA_AT_END;
    if (m->reader == NULL) return MA_ERROR;     /* an ADTS reopen failed */
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
        /* The decoder may change its output format, usually on the first
           chunk: an HE-AAC stream only reveals its doubled rate (and, for
           HE-AACv2, its second channel) once decoding starts. Until the
           format is locked (end of open) the new one is adopted; afterwards
           only a change back to the locked format is acceptable, since
           miniaudio has already set up its conversion. */
        if (flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED) {
            ma_result r = ld_mf_query_format(m);
            if (r != MA_SUCCESS) {
                if (sample != NULL) sample->lpVtbl->Release(sample);
                return r;
            }
        }
        if (flags & MF_SOURCE_READERF_ENDOFSTREAM) {
            /* Reads stop after this; a chunk delivered with the flag still counts. */
            m->eof = MA_TRUE;
            m->pendFrames = m->pendPos = 0;
            if (sample == NULL) return MA_AT_END;
        }
        if (sample == NULL) continue;   /* a gap marker with no audio */

        if (!m->fmtLocked) {
            m->fmt = m->readerFmt;
        } else if (!ld_mf_fmt_equal(&m->fmt, &m->readerFmt)) {
            sample->lpVtbl->Release(sample);
            return MA_ERROR;
        }

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
        ma_uint64 frames = bytes / m->fmt.bytesPerFrame;
        size_t need = (size_t)(frames * m->fmt.bytesPerFrame);
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
        if (frames == 0) {
            if (m->eof) return MA_AT_END;
            continue;
        }
        *pStart = ld_mf_ts_to_frames(ts, m->fmt.sampleRate) - m->origin;
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
    pv.hVal.QuadPart = ld_mf_frames_to_ts(frame + m->origin, m->fmt.sampleRate);
    HRESULT hr = m->reader->lpVtbl->SetCurrentPosition(m->reader, &GUID_NULL, &pv);
    PropVariantClear(&pv);
    if (FAILED(hr)) return MA_ERROR;
    m->eof = MA_FALSE;
    m->pendFrames = m->pendPos = 0;
    return ld_mf_fetch(m, pStart);
}

/* ADTS mark -> frame position at the output rate. */
static ma_int64 ld_mf_mark_frames(ld_mf* m, size_t i)
{
    return (ma_int64)llround((double)m->adts.marks[i].pos * (double)m->fmt.sampleRate / (double)m->adts.coreRate);
}

/* ADTS: starts a fresh reader at mark `i` and decodes its first chunk. A new
   reader's timestamps restart near 0, offset the same way the whole file's
   were at open, so the chunk sits at the mark's position plus that offset. */
static ma_result ld_mf_adts_jump(ld_mf* m, size_t i, ma_int64* pStart)
{
    ma_int64 s = 0;
    ma_result r = ld_mf_open_reader(m, m->adts.marks[i].off);
    if (r == MA_SUCCESS) r = ld_mf_fetch(m, &s);
    if (r == MA_SUCCESS) *pStart = ld_mf_mark_frames(m, i) + (s - m->firstStart);
    return r;
}

/* Back to the very start of the file (timestamp 0, before any priming that
   gets dropped), so the decoder rebuilds exactly the state it had at open. The
   first chunk is the same one decoded at open, so it gets the same position. */
static ma_result ld_mf_restart(ld_mf* m, ma_int64* pStart)
{
    if (m->adts.count > 0) return ld_mf_adts_jump(m, 0, pStart);
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
                memcpy((ma_uint8*)pFramesOut + done * m->fmt.bytesPerFrame,
                       m->pend + m->pendPos * m->fmt.bytesPerFrame, (size_t)(n * m->fmt.bytesPerFrame));
            }
            m->pendPos += n;
            done += n;
            continue;
        }
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

/* Sample-accurate seek in bounded time.
 *
 * The resource manager runs every stream's decoding on one job thread, so a
 * slow seek in one file starves all the others of fresh audio. Each path below
 * therefore decodes at most a few seconds, however long the file:
 *
 *   - Short forward seeks (up to LD_MF_DECODE_FORWARD_SECS) decode forward
 *     from the current position and discard.
 *   - ADTS: open a fresh reader at the last frame mark at least
 *     LD_MF_SEEK_PREROLL_SECS before the target, then decode forward to it.
 *     Exact (see "ADTS: exact length and a seek index").
 *   - MP4 and ASF: ask MF to jump LD_MF_SEEK_PREROLL_SECS before the target
 *     (lossy decoders need the packet before the target to rebuild their
 *     state), read the timestamp of the first decoded chunk to learn where it
 *     landed, then decode forward to the target. If MF lands past the target,
 *     retry from 1 s and then 4 s back (and, should even that fail, from the
 *     start, the one unbounded case; not seen in practice). MP4 timestamps count samples, so this is exact;
 *     ASF stores timestamps in whole milliseconds, so WMA seeks can land up to
 *     about a millisecond (measured: under 20 frames) off.
 */
#define LD_MF_SEEK_PREROLL_SECS 0.25
#define LD_MF_DECODE_FORWARD_SECS 2

static ma_result ld_mf_seek_inner(ld_mf* m, ma_uint64 frameIndex)
{
    ma_int64 start = 0;
    ma_int64 preroll = (ma_int64)(m->fmt.sampleRate * LD_MF_SEEK_PREROLL_SECS);
    ma_uint64 forwardLimit = (ma_uint64)m->fmt.sampleRate * LD_MF_DECODE_FORWARD_SECS;
    ma_result r;

    if (frameIndex >= m->cursor && frameIndex - m->cursor <= forwardLimit) {
        /* `pend` resumes exactly at the cursor. */
        return ld_mf_skip_to(m, (ma_int64)m->cursor, frameIndex);
    }

    if (m->adts.count > 0) {
        /* The last mark at or before target - preroll (binary search). */
        ma_int64 want = (ma_int64)frameIndex - preroll;
        size_t lo = 0, hi = m->adts.count;
        while (hi - lo > 1) {
            size_t mid = (lo + hi) / 2;
            if (ld_mf_mark_frames(m, mid) <= want) lo = mid; else hi = mid;
        }
        r = ld_mf_adts_jump(m, lo, &start);
    } else {
        /* Pre-roll of 0.25 s, then 1 s, then 4 s if MF keeps landing late. */
        r = MA_ERROR;
        for (int attempt = 0; attempt < 3; attempt++, preroll *= 4) {
            ma_int64 jump = (ma_int64)frameIndex - preroll;
            r = (jump <= 0) ? ld_mf_restart(m, &start) : ld_mf_reposition(m, jump, &start);
            if (r != MA_SUCCESS || start <= (ma_int64)frameIndex || jump <= 0) break;
        }
        if (r == MA_SUCCESS && start > (ma_int64)frameIndex) r = ld_mf_restart(m, &start);
    }

    if (r == MA_AT_END) { m->cursor = frameIndex; return MA_SUCCESS; }
    if (r != MA_SUCCESS) return r;
    return ld_mf_skip_to(m, start, frameIndex);
}

static ma_result ld_mf_seek(ma_data_source* pDataSource, ma_uint64 frameIndex)
{
    ld_mf* m = (ld_mf*)pDataSource;
    int com = ld_mf_com_enter();
    ma_result r = ld_mf_seek_inner(m, frameIndex);
    if (r != MA_SUCCESS) {
        /* The reader may have moved partway. Put it back at frame 0 so the
           cursor matches what read() returns next; if even that fails, end
           the stream rather than hand out audio from an unknown position. */
        ma_int64 start = 0;
        if (ld_mf_restart(m, &start) != MA_SUCCESS || ld_mf_skip_to(m, start, 0) != MA_SUCCESS) {
            m->eof = MA_TRUE;
            m->pendFrames = m->pendPos = 0;
            m->cursor = 0;
        }
    }
    ld_mf_com_leave(com);
    return r;
}

static ma_result ld_mf_get_data_format(ma_data_source* pDataSource, ma_format* pFormat, ma_uint32* pChannels, ma_uint32* pSampleRate, ma_channel* pChannelMap, size_t channelMapCap)
{
    ld_mf* m = (ld_mf*)pDataSource;
    if (pFormat != NULL) *pFormat = m->fmt.format;
    if (pChannels != NULL) *pChannels = m->fmt.channels;
    if (pSampleRate != NULL) *pSampleRate = m->fmt.sampleRate;
    /* MF delivers channels in the standard WAV order. */
    if (pChannelMap != NULL) ma_channel_map_init_standard(ma_standard_channel_map_microsoft, pChannelMap, channelMapCap, m->fmt.channels);
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

/* Releases everything. The source is cut off before our reference goes, in
   case MF still holds a view of it. */
static void ld_mf_free(ld_mf* m, const ma_allocation_callbacks* pAlloc)
{
    if (m->reader != NULL) m->reader->lpVtbl->Release(m->reader);
    if (m->src != NULL) {
        ld_src_detach(m->src);
        ld_src_release(m->src);
    }
    free(m->adts.marks);
    free(m->pend);
    ma_free(m, pAlloc);
}

/* Longest plausible length from MF's declared duration: anything above this
   is a placeholder ("unknown") rather than a real duration. */
#define LD_MF_MAX_SECS (60.0 * 60.0 * 24.0 * 7.0)

/* Builds the decoder on `src`, whose content sniffed as `kind`. Takes over the
   caller's reference to `src`, success or not. */
static ma_result ld_mf_open(ld_mf_src* src, ld_mf_kind kind, QWORD audioStart, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    ma_result r = MA_INVALID_FILE;
    ma_int64 start = 0;
    ld_mp4_trim_t trim = { 0.0, 0.0 };
    ma_bool32 haveTrim = MA_FALSE;

    ld_mf* m = (ld_mf*)ma_malloc(sizeof(ld_mf), pAlloc);
    if (m == NULL) {
        ld_src_detach(src);
        ld_src_release(src);
        return MA_OUT_OF_MEMORY;
    }
    memset(m, 0, sizeof(*m));
    m->src = src;
    m->kind = kind;

    /* Container facts MF doesn't give us, read straight from the file. */
    if (kind == LD_MF_MP4) haveTrim = ld_mp4_trim(src, &trim);
    if (kind == LD_MF_ADTS) ld_adts_scan(src, audioStart, &m->adts);

    r = ld_mf_open_reader(m, 0);
    if (r != MA_SUCCESS) goto fail;

    /* Decode the first chunk now: proves a decoder is installed and working
       (Windows "N" editions lack some), settles the output format (see
       ld_mf_fetch), and fixes the timestamp origin, i.e. which timestamp
       counts as frame 0:
         - an MP4 trim: the priming frames after the first chunk's start are
           cut, so frame 0 is that many frames in (skip_to drops them);
         - MF already gave the priming negative timestamps: keep origin 0 and
           let skip_to drop everything before it, with no trim on top;
         - otherwise the first chunk's start, so a file whose audio starts
           after timestamp 0 does not gain leading silence. */
    r = ld_mf_fetch(m, &start);
    if (r != MA_SUCCESS) { if (r == MA_AT_END) r = MA_INVALID_FILE; goto fail; }
    m->fmtLocked = MA_TRUE;
    if (start >= 0) {
        ma_int64 skip = haveTrim ? (ma_int64)llround(trim.skip * m->fmt.sampleRate) : 0;
        m->origin = start + skip;
        start = -skip;
    }
    m->firstStart = start;
    r = ld_mf_skip_to(m, start, 0);
    if (r != MA_SUCCESS) goto fail;

    if (haveTrim && trim.len > 0.0) {
        /* Exact, and the end trim is enforced in read. */
        m->length = (ma_uint64)llround(trim.len * m->fmt.sampleRate);
        m->stopAtLength = MA_TRUE;
    } else if (m->adts.count > 0) {
        m->length = (ma_uint64)llround((double)m->adts.total * m->fmt.sampleRate / m->adts.coreRate);
    } else {
        /* The container's declared duration (100 ns units). For ASF this is
           stored to the millisecond and may come out a few ms short of what
           decodes; reads still run to the real end. For a fragmented MP4 with
           no stated duration it is absent or a placeholder, and the length
           stays 0 ("unknown"): reads run to the end, and the app's duration
           probe falls back to counting decoded frames. */
        PROPVARIANT pv;
        PropVariantInit(&pv);
        if (SUCCEEDED(m->reader->lpVtbl->GetPresentationAttribute(m->reader, (DWORD)MF_SOURCE_READER_MEDIASOURCE,
                                                                  &MF_PD_DURATION, &pv)) && pv.vt == VT_UI8 &&
            (double)pv.uhVal.QuadPart / 10000000.0 < LD_MF_MAX_SECS) {
            ma_int64 frames = ld_mf_ts_to_frames((LONGLONG)pv.uhVal.QuadPart, m->fmt.sampleRate) - m->origin;
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
    *ppBackend = (ma_data_source*)m;
    return MA_SUCCESS;

fail:
    ld_mf_free(m, pAlloc);
    return (r == MA_SUCCESS) ? MA_INVALID_FILE : r;
}

/* Shared by both entry points: sniff, then (only for content this backend
   claims) start MF and build the decoder. Takes over `src`. */
static ma_result ld_mf_open_src(ld_mf_src* src, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    QWORD audioStart = 0;
    ld_mf_kind kind = ld_mf_sniff(src, &audioStart);
    if (kind == LD_MF_NONE || !ld_mf_ready()) {
        ld_src_detach(src);
        ld_src_release(src);
        return (kind == LD_MF_NONE) ? MA_INVALID_FILE : MA_NOT_IMPLEMENTED;
    }
    return ld_mf_open(src, kind, audioStart, pAlloc, ppBackend);
}

static ma_result ld_mf_backend_init(void* pUserData, ma_read_proc onRead, ma_seek_proc onSeek, ma_tell_proc onTell, void* pReadSeekTellUserData, const ma_decoding_backend_config* pConfig, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    (void)pUserData; (void)pConfig;
    if (onRead == NULL || onSeek == NULL || onTell == NULL) return MA_INVALID_ARGS;
    ld_mf_src* src = ld_src_from_callbacks(onRead, onSeek, onTell, pReadSeekTellUserData);
    if (src == NULL) return MA_INVALID_FILE;
    int com = ld_mf_com_enter();
    ma_result r = ld_mf_open_src(src, pAlloc, ppBackend);
    ld_mf_com_leave(com);
    return r;
}

static ma_result ld_mf_backend_init_file_w(void* pUserData, const wchar_t* pFilePath, const ma_decoding_backend_config* pConfig, const ma_allocation_callbacks* pAlloc, ma_data_source** ppBackend)
{
    (void)pUserData; (void)pConfig;
    ld_mf_src* src = ld_src_from_file(pFilePath);
    if (src == NULL) return MA_DOES_NOT_EXIST;
    int com = ld_mf_com_enter();
    ma_result r = ld_mf_open_src(src, pAlloc, ppBackend);
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
