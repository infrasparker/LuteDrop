# Audio Engine Refactor

## Problem

The audio engine is a single 978-line file (`audio/engine.rs`) containing playback, crossfade timing, gap scheduling, volume math, sound loading, duration probing, and library preview. The library preview (`play_preview`) and soundstage playback (`start_clip_inner`) are two separate code paths with different behavior — trim enforcement, crossfade support, volume chain, and fade handling diverge between them. This caused bugs (the "All's Well" crossfade timing issue) and means users hear different audio in the library vs the soundstage.

## Solution

Split the engine into focused modules with clear boundaries and one-directional dependencies. Unify library and soundstage playback through a shared `SoundPlayer`. Replace the bolted-on `__preview` track with a dedicated library sound handle.

## Module Structure

```
audio/
├── mod.rs              — AudioEngine: owns kira AudioManager, public API, thin coordinator
├── sound.rs            — SoundPlayer: load file, apply trim/loop/volume/fade, play/stop
├── mixer.rs            — volume chain computation, mute state management
├── transitions.rs      — TransitionManager: crossfade/gap/gapless lifecycle state + decisions
├── orchestrator.rs     — TrackOrchestrator: clip sequencing, mode handling, advancement
└── util.rs             — probe_duration, amplitude_to_decibels
```

### Dependency Flow (one-directional)

```
mod.rs (AudioEngine)
  ├── owns AudioManager (kira)
  ├── owns HashMap<String, ManagedTrack>
  ├── owns MuteState
  ├── owns library_sound: Option<SoundHandle>
  ├── delegates to orchestrator for track operations
  └── delegates to sound player for library playback

orchestrator.rs
  ├── receives &mut AudioManager, &mut HashMap, &MuteState
  ├── uses sound.rs to start/stop sounds
  ├── uses transitions.rs per-track for crossfade/gap decisions
  └── uses mixer.rs for volume computation

transitions.rs
  ├── per-instance state: outgoing fade, pending gap, crossfade_started flag
  └── returns TransitionAction enum to orchestrator

sound.rs
  ├── receives &mut AudioManager, &TrackHandle
  ├── loads files, applies trim/loop/volume/fade
  └── returns SoundHandle

mixer.rs
  ├── compute_volume(master, track, clip, library, mute_state) → Decibels
  └── MuteState struct with per-master, per-track, per-clip mute

util.rs
  └── probe_duration, shared helpers
```

Nothing depends upward. SoundPlayer doesn't know about tracks. TransitionManager doesn't know about kira. Orchestrator combines them.

## sound.rs — SoundPlayer

Lowest layer. One job: play a sound file with parameters.

### Types

```rust
pub struct SoundHandle {
    pub handle: StreamingSoundHandle<FromFileError>,
    pub fade_out: Option<f64>,  // default fade-out duration stored with the handle
}

pub struct PlayParams {
    pub file_path: String,
    pub start_position: f64,
    pub volume: Decibels,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub loop_region: Option<(f64, f64)>,  // (start, end)
    pub fade_in: Option<f64>,             // seconds
    pub fade_out: Option<f64>,            // default fade-out seconds (stored on handle)
}
```

### Functions

- `play(manager: &mut AudioManager, track_handle: &TrackHandle, params: PlayParams) → Result<SoundHandle, String>` — loads file, applies all parameters, plays on the given mixer sub-track
- `stop(handle: &mut SoundHandle, fade_override: Option<Duration>)` — stops with handle's stored fade_out or the override
- `pause(handle: &mut SoundHandle)` / `resume(handle: &mut SoundHandle)`
- `position(handle: &SoundHandle) → f64` — current playback position
- `state(handle: &SoundHandle) → PlaybackState` — playing/stopped/paused
- `set_volume(handle: &mut SoundHandle, volume: Decibels)`

No module-level state. All functions take what they need. Preview and soundstage both call `play()` with different `PlayParams`. Same code path.

## mixer.rs — Volume and Mute

### Types

```rust
pub struct MuteState {
    pub master: bool,
    pub tracks: HashMap<String, bool>,
    pub clips: HashMap<String, bool>,
}
```

### Functions

```rust
pub fn compute_volume(
    master: f64,
    track: f64,
    clip: f64,
    library: f64,
    mute_state: &MuteState,
    track_id: &str,
    clip_id: &str,
) -> Decibels
```

Returns silence if any level in the chain is muted (master, track, or clip). Otherwise returns `amplitude_to_decibels(master * track * clip * library)`.

`MuteState` methods: `is_track_muted()`, `is_clip_muted()`, `toggle_track_mute()`, `toggle_clip_mute()`, `toggle_master_mute()`.

## transitions.rs — TransitionManager

Stateful per-track. Manages the lifecycle of transitions between clips.

### Types

```rust
pub enum TransitionAction {
    None,
    StartCrossfade { next_clip_index: usize, duration: f64 },
    AdvanceClip { next_clip_index: usize },
    ScheduleGap { next_clip_index: usize, delay_seconds: f64 },
    StartPendingClip { clip_index: usize },
    CleanupOutgoing,
}

pub struct TransitionManager {
    outgoing: Option<OutgoingFade>,
    crossfade_started: bool,
    pending_start: Option<PendingStart>,
}

struct OutgoingFade {
    handle: SoundHandle,
    started: Instant,
    duration: f64,
}

struct PendingStart {
    clip_index: usize,
    start_at: Instant,
}
```

### Methods

- `new() → TransitionManager`
- `update(position: f64, clip_duration: f64, clip_start: f64, mode: &TrackMode, current_clip_index: Option<usize>, clip_count: usize) → TransitionAction` — main tick called every 50ms. Handles:
  - Pending gap whose time has arrived → `StartPendingClip`
  - Outgoing crossfade that's done → `CleanupOutgoing`
  - Clip approaching end → `StartCrossfade`, `AdvanceClip`, or `ScheduleGap` based on transition type
  - Short clips (playable duration < crossfade duration) → scaled-down crossfade
- `begin_crossfade(outgoing_handle: SoundHandle, duration: f64)` — store outgoing state after orchestrator executes crossfade
- `schedule_gap(clip_index: usize, delay: f64)` — store pending start
- `reset()` — clear all state (called when track stops)
- `take_outgoing(&mut self) → Option<SoundHandle>` — for cleanup

## orchestrator.rs — TrackOrchestrator

Manages track lifecycle. Stateless module — receives everything it needs.

### Types

```rust
pub struct ManagedTrack {
    pub track_handle: TrackHandle,
    pub config: Track,
    pub current_sound: Option<SoundHandle>,
    pub current_clip_index: Option<usize>,
    pub current_clip_start: Option<f64>,
    pub current_clip_duration: Option<f64>,
    pub is_paused: bool,
    pub transitions: TransitionManager,
}
```

### Functions

All receive the tracks HashMap, AudioManager, and other dependencies as parameters.

- `setup_track(manager, tracks, config)` — create kira sub-track, insert ManagedTrack
- `play_track(tracks, manager, track_id, mute_state)` — resume if paused, else pick first clip and play via SoundPlayer
- `play_track_clip(tracks, manager, track_id, clip_id, mute_state)` — play specific clip
- `play_track_from(tracks, manager, track_id, position, mute_state)` — play from position
- `pause_track(tracks, track_id)` / `resume_track(tracks, track_id)`
- `stop_track(tracks, track_id)` — stop with fade, reset TransitionManager
- `advance_track(tracks, manager, track_id, next_clip_index, mute_state)` — start next clip
- `poll_track(tracks, manager, track_id, mute_state)` — called every 50ms, delegates to TransitionManager, executes returned actions
- `pick_next_clip(config, current_index) → Option<usize>` — mode-aware clip selection

## mod.rs — AudioEngine

Public API. Thin coordinator. Owns system resources.

### Struct

```rust
pub struct AudioEngine {
    manager: AudioManager,
    tracks: HashMap<String, ManagedTrack>,
    master_volume: f64,
    mute_state: MuteState,
    library_sound: Option<SoundHandle>,
    library_track_handle: TrackHandle,  // permanent kira mixer bus for library playback, created in new()
}
```

### Public Methods

Track operations (delegate to orchestrator):
- `setup_track`, `play_track`, `play_track_clip`, `play_track_from`
- `pause_track`, `resume_track`, `stop_track`
- `stop_all`, `pause_all`, `resume_all`
- `has_track`, `update_track_config`

Comp operations:
- `activate_comp`, `stop_comp`, `pause_comp`, `resume_comp` — iterate comp's tracks and delegate

Volume:
- `set_master_volume`, `set_track_volume`, `set_clip_volume`

Mute:
- `mute_master` / `unmute_master`
- `mute_track` / `unmute_track`
- `mute_clip` / `unmute_clip`

Library playback (direct SoundPlayer):
- `play_library_sound(params: PlayParams) → Result<(), String>` — stop current, play new on library mixer bus
- `stop_library_sound()`

Polling:
- `poll() → Vec<TrackPlaybackState>` — iterate tracks, call orchestrator::poll_track, collect states. Also check library sound state.

Each method is 5-15 lines. The engine file stays under 200 lines.

## Library Playback (Unified Path)

Library "preview" is eliminated as a concept. Both library and soundstage use `SoundPlayer::play()`. The difference:

| Concern | Soundstage | Library |
|---|---|---|
| Who calls SoundPlayer | Orchestrator | AudioEngine directly |
| Kira sub-track | Per-track mixer bus | Permanent library mixer bus |
| Volume | mixer::compute_volume (full chain) | master × library_entry.volume |
| Trim/loop | From Clip struct (resolved from library entry) | From library entry directly |
| Transitions | TransitionManager handles crossfade/gap | None — single sound, stops at end |
| Clip sequencing | Orchestrator picks next clip | N/A — one sound at a time |

Same sound loading, same trim/loop application, same fade handling. No divergent code paths.

## Cleanup Included

### Remove dead code
- `update_library_entry_trim()` in database/library.rs (superseded by `update_library_entry_markers`)
- `reorder_group()`, `reorder_comp()`, `reorder_track()` in database/hierarchy.rs (not wired to commands)

### Fold persistence.rs into lib.rs
Two 7-line path functions don't need their own module.

### Standardize action naming
Drop `_audio` / `_action` suffixes from actions.rs functions. Commands.rs has the clean names; actions should match.

### Frontend renames
- `playPreview` / `stopPreview` → `playLibrarySound` / `stopLibrarySound` in useAudio.ts
- `previewEntryId` → `libraryPlayingEntryId` in appStore.ts
- `previewState` → `libraryPlaybackState` in components
- `__preview` playback state key → `__library` (or whatever the engine uses for the library sound's track ID in playback state reporting)

### Living documentation
- `docs/engine-architecture.md` — written after refactor completes. Explains module structure, data flow, how to add a new transition type, library vs track playback.
