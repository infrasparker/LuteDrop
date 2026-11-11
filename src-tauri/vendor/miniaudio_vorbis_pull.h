/*
Ogg Vorbis decoding backend built on stb_vorbis's "pull" API.

miniaudio already ships an stb_vorbis backend, but on every path this app uses
it runs stb_vorbis in "push" mode, because the built-in backend has no
wide-path (onInitFileW) entry and so all Unicode-path opens arrive through
miniaudio's read/seek callbacks. Push mode cannot report a length (miniaudio
returns 0, so duration probes fail) and seeks by decoding forward from the
current position, or from the very start when seeking backwards. Pull mode,
which needs either a FILE* or the whole file in memory, reports the exact length
and seeks sample-accurately using the Ogg page index. This backend gives
stb_vorbis exactly that:

  - onInitFileW: opens the file with _wfopen (Unicode-safe) and hands stb_vorbis
    the FILE*. Used by ma_decoder_init_file_w (waveforms, duration, analysis).
  - onInit: reads the whole compressed file into memory through miniaudio's
    read/seek/tell callbacks and decodes from that buffer. This is the path the
    resource manager (playback) takes. The cost is the compressed file size in
    RAM per open stream (a few MB per song).

Requires the stb_vorbis declarations (stb_vorbis.c included with
STB_VORBIS_HEADER_ONLY) before this file.
*/
#ifndef miniaudio_vorbis_pull_h
#define miniaudio_vorbis_pull_h

#include "miniaudio.h"

/* Already a pointer: put it in a backends array directly, without &. */
extern ma_decoding_backend_vtable* ld_decoding_backend_vorbis;

#endif /* miniaudio_vorbis_pull_h */
