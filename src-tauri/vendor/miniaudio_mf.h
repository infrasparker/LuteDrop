/*
Decoding backend for AAC (.m4a/.mp4 and raw ADTS .aac), ALAC (.m4a) and WMA,
using Windows Media Foundation, the system's built-in media framework. Windows
ships decoders for all of these, so nothing extra is bundled.

Media Foundation (MF) pieces used:
  - IMFByteStream: MF's view of a file. Made with MFCreateMFByteStreamOnStream
    from a minimal COM IStream of our own, which reads either a Win32 file
    handle (path open) or miniaudio's read/seek callbacks (see onInit below).
  - IMFSourceReader: parses the container, runs the decoder, and hands back
    decoded audio in chunks ("samples", each with a start timestamp in 100 ns
    units). Asked to output float32 PCM at the file's own rate and channels.

Which files it claims: only content that sniffs as an MP4 container, an ADTS AAC
stream or an ASF (WMA) container, judged from the first bytes, not the file
extension. MF can also decode MP3, WAV and FLAC, but miniaudio's own decoders
handle those (and libopus/stb_vorbis handle Ogg), and miniaudio offers every file
to custom backends like this one before its built-in decoders, so claiming by
content keeps those files on their existing decoders.

Both miniaudio entry points are implemented:
  - onInitFileW (wide path): ma_decoder_init_file_w, i.e. StreamingDecoder and
    the duration probe.
  - onInit (read/seek/tell callbacks): the resource manager, i.e. playback,
    opens every file through ma_decoder_init_vfs_w, which only ever calls
    onInit, with only those callbacks to read the file through.
*/
#ifndef miniaudio_mf_h
#define miniaudio_mf_h

#include "miniaudio.h"

/* Already a pointer: put it in a backends array directly, without &. */
extern ma_decoding_backend_vtable* ld_decoding_backend_mf;

#endif /* miniaudio_mf_h */
