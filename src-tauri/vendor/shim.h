#ifndef LD_SHIM_H
#define LD_SHIM_H
#include <stdint.h>
// Decode a whole audio file (any miniaudio format incl .opus) to interleaved f32.
// Returns 0 on success. Caller frees with ld_free_decoded.
int ld_decode_file(const char* path, float** out_interleaved,
                   uint64_t* out_frames, uint32_t* out_channels, uint32_t* out_sample_rate);
void ld_free_decoded(float* p);
#endif
