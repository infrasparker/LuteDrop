#ifndef LD_SHIM_H
#define LD_SHIM_H
#include <stdint.h>

// ---- Offline decode (Task 1) ------------------------------------------------
// Decode a whole audio file (any miniaudio format incl .opus) to interleaved f32.
// Returns 0 on success. Caller frees with ld_free_decoded.
int ld_decode_file(const char* path, float** out_interleaved,
                   uint64_t* out_frames, uint32_t* out_channels, uint32_t* out_sample_rate);
void ld_free_decoded(float* p);

// Cheap duration probe: inits a decoder (libopus backend registered, same as
// ld_decode_file), reads the PCM frame count via ma_decoder_get_length_in_pcm_frames
// and divides by the output sample rate -- NO full decode. Returns the duration in
// seconds, or a negative value if the file can't be opened or its length is unknown
// (e.g. a streamed source with no reported length).
double ld_file_duration_secs(const char* path);

// ---- Playback layer (Task 2) ------------------------------------------------
// Opaque handles. The shim owns the underlying miniaudio structs.
typedef struct ld_engine ld_engine;
typedef struct ld_group  ld_group;
typedef struct ld_sound  ld_sound;

// Engine wraps ma_engine + a ma_resource_manager that registers the libopus
// backend (so streamed .opus works). Returns NULL on failure.
ld_engine* ld_engine_create(void);
void       ld_engine_destroy(ld_engine*);
void       ld_engine_set_volume(ld_engine*, float linear);   // master volume
uint64_t   ld_engine_time_frames(ld_engine*);                // global engine clock
uint32_t   ld_engine_sample_rate(ld_engine*);

// Group wraps ma_sound_group (a per-track / library sub-mix). Returns NULL on failure.
ld_group*  ld_group_create(ld_engine*);
void       ld_group_set_volume(ld_group*, float linear);
void       ld_group_destroy(ld_group*);

// Sound wraps ma_sound. stream != 0 => streaming (use for playback);
// group may be NULL (routes to the engine endpoint). Returns NULL on failure.
ld_sound*  ld_sound_create(ld_engine*, ld_group*, const char* path, int stream);
int        ld_sound_play(ld_sound*, double start_secs, double fade_in_secs);
void       ld_sound_stop(ld_sound*, double fade_out_secs);
void       ld_sound_pause(ld_sound*);    // = stop (keeps cursor)
void       ld_sound_resume(ld_sound*);   // = start (cursor preserved)
void       ld_sound_set_volume(ld_sound*, float linear);
double     ld_sound_position_secs(ld_sound*);
double     ld_sound_length_secs(ld_sound*);   // total length in seconds (0 if unknown)
int        ld_sound_is_playing(ld_sound*);
int        ld_sound_at_end(ld_sound*);
void       ld_sound_schedule_start_frame(ld_sound*, uint64_t engine_frame);
void       ld_sound_destroy(ld_sound*);

#endif
