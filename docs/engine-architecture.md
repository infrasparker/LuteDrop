# Engine Architecture

LuteDrop's audio engine lives in `src-tauri/src/audio/`. It is a multi-module Rust library that wraps [Kira](https://github.com/tesselode/kira) for real-time audio playback.

---

## Overview

The engine manages two independent playback contexts:

1. **Track playback** — one or more clips assigned to a track, played through an orchestrator that handles sequencing, looping, shuffling, and transitions between clips.
2. **Library playback** — a single sound played directly from a library entry for preview or auditioning purposes, with no orchestrator involvement.

The public surface is `AudioEngine` (`audio/mod.rs`). All Tauri commands go through `actions.rs`, which calls into `AudioEngine` methods. A 50 ms polling loop in `lib.rs` drives transitions and emits playback state events to the frontend.

---

## Module Structure

```
audio/
  mod.rs          — AudioEngine struct; public API; owns all state
  sound.rs        — SoundPlayer: wraps Kira streaming handles (PlayParams, SoundHandle)
  orchestrator.rs — Track lifecycle: setup, play, pause, stop, poll, volume
  transitions.rs  — TransitionManager: crossfade, gap, gapless logic
  mixer.rs        — Volume computation and mute state
  util.rs         — probe_duration, amplitude_to_decibels, CROSSFADE_OVERLAP
```

Dependency direction (no cycles):

```
mod.rs
  └── orchestrator.rs
        ├── sound.rs
        ├── mixer.rs
        └── transitions.rs
              └── sound.rs (stop_quick)
  └── sound.rs  (library path)
  └── mixer.rs  (library volume)
  └── util.rs
```

---

## How a Sound Gets Played

Entry point: `AudioEngine::play_library_sound` (`audio/mod.rs`) or `orchestrator::start_clip` (`audio/orchestrator.rs`).

1. Caller constructs a `PlayParams` (`audio/sound.rs:PlayParams`):
   - `file_path`, `start_position`, `volume` (as `Decibels`)
   - Optional `trim_start`, `trim_end`, `loop_region`, `fade_in`, `fade_out`

2. `sound::play` (`audio/sound.rs:play`) opens a `StreamingSoundData` from disk, applies start position, loop region, fade-in tween, and volume, then calls `track_handle.play(sound_data)`.

3. Kira returns a `StreamingSoundHandle`, which is wrapped in `SoundHandle` along with cached `fade_out`, `duration`, and `start_position` for later transition calculations.

4. The caller stores the `SoundHandle` (either in `ManagedTrack.current_sound` for track playback or `AudioEngine.library_sound` for library playback).

---

## How Tracks Work

### Setup (`audio/orchestrator.rs:setup_track`)

`AudioEngine::setup_track` is called from `actions::ensure_track_setup` before any playback command. It creates a Kira sub-track (`TrackHandle`) for the track and inserts a `ManagedTrack` into `AudioEngine.tracks`. If the track already exists, config is updated in-place.

### Play (`audio/orchestrator.rs:play_track`, `start_clip`)

`play_track` picks the first clip (or a random one in shuffle mode) and calls `start_clip`. `start_clip`:
1. Stops any currently playing sound on the track (`sound::stop_quick`).
2. Computes final volume via `mixer::compute_volume`.
3. Builds `PlayParams` (including seamless loop region if mode is `Seamless`).
4. Calls `sound::play` and stores the handle.
5. Calls `track.transitions.clip_started()` to reset transition state.

### Poll Loop (`lib.rs:playback_polling_loop`, `audio/orchestrator.rs:poll_track`)

`lib.rs` spawns a thread that calls `audio.poll()` every 50 ms. `AudioEngine::poll` iterates all tracks and calls `orchestrator::poll_track` for each.

`poll_track` does the following each tick:
1. Reads the current sound's position and playback state.
2. Calls `TransitionManager::update` to decide if any transition action is needed.
3. Executes the returned `TransitionAction`.

### Transitions (`audio/orchestrator.rs:poll_track`, `audio/transitions.rs`)

`TransitionManager::update` (`transitions.rs:update`) inspects the current playback position against the clip duration and the configured transition type, returning a `TransitionAction` variant:

| Variant | Trigger | Effect |
|---|---|---|
| `AdvanceClip` | Sound stopped naturally (gapless) | Immediately start next clip |
| `StartCrossfade` | Position reaches `clip_dur - CROSSFADE_OVERLAP` | Take outgoing sound, start next clip overlapping |
| `ScheduleGap` | Sound stopped (gap transition) | Record delayed start time |
| `StartPendingClip` | Delayed start time reached | Start the queued clip |
| `CleanupOutgoing` | Crossfade duration + 0.5 s elapsed | Stop and drop the outgoing handle |
| `None` | Nothing to do | If sound stopped and no next clip, track stops |

Clip advancement wraps or stops based on the mode's `repeat` flag. Shuffle picks a random index that avoids repeating the previous clip (`transitions.rs:pick_next_clip_index`).

### Advance

When the last clip in a non-repeating sequence ends and no next clip is available, `poll_track` calls `stop_track`, which resets `current_clip_index`, clears the transition state, and drops the sound handle.

---

## How Library Playback Works

Library playback bypasses the orchestrator entirely.

- `AudioEngine::play_library_sound` (`mod.rs`) stops any existing `library_sound`, builds `PlayParams` from the command arguments, and calls `sound::play` targeting `AudioEngine.library_track_handle` (a dedicated Kira sub-track).
- `AudioEngine::stop_library_sound` calls `sound::stop` with the stored handle's fade-out duration.
- The 50 ms poll loop **always** emits a `TrackPlaybackState` for `track_id = "__library"` — either the current sound's state or a stopped state (`is_playing: false, is_paused: false`). This ensures the frontend properly detects when library playback ends.
- The poll loop also checks if the library sound's position has passed its `trim_end` (stored in `SoundHandle.duration`). If so, it stops the sound. This enforces trim_end for non-looping library playback, since Kira has no native "stop at position X" feature.
- When the user navigates away from the Library view, `App.vue` calls `stopLibrarySound`.
- `useAudio.playLibrarySound` takes `entryId` as its first parameter and centrally sets `store.libraryPlayingEntryId` on success. `stopLibrarySound` centrally clears it and removes the `__library` playback state entry. Callers never manage this state manually.

### Waveform click-to-play behavior

When the user clicks the waveform in the library editor, `WaveformEditor.vue` calls `playLibrarySound` with boundary-aware parameters:
- `trim_end` is only passed if the click position is **before** it. Clicking past trim_end plays to the end of the file.
- `loop_region` is only passed if the click position is **before** loop_end. Clicking past the loop plays without looping.
- This allows users to audition any part of the file freely while still enforcing boundaries they'll "hit" during playback.

### Playhead rendering

`WaveformEditor.vue` uses a plain function `getPlaybackState()` (not a Vue computed) to check playback state on every RAF frame. This avoids Vue's computed caching, which can serve stale values during component mount lifecycle. The function checks `store.libraryPlayingEntryId === props.entryId` to ensure the playhead only renders on the entry that is actually playing.

---

## Volume Chain

All volume computation goes through `mixer::compute_volume` (`audio/mixer.rs:compute_volume`):

```
final_dB = amplitude_to_decibels(master × track × clip × library)
```

- **master**: global slider, stored in `AudioEngine.master_volume` (0.0–1.0).
- **track**: per-track slider, stored in `Track.volume`.
- **clip**: per-clip slider, stored in `Clip.volume`.
- **library**: the `LibraryEntry.volume` gain baked into the clip at assignment time (`Clip.library_volume`).

If `MuteState::is_muted` returns `true` for the `(track_id, clip_id)` pair, the result is `Decibels::SILENCE` regardless of volume levels.

`amplitude_to_decibels` (`util.rs`) uses `20 * log10(amp)` and clamps values below 0.0001 to `Decibels::SILENCE`.

---

## Transition Lifecycle

`TransitionManager` (`transitions.rs`) is owned by each `ManagedTrack`. Key state fields:

- `outgoing: Option<OutgoingFade>` — sound handle fading out during a crossfade, with its start time and duration.
- `crossfade_started: bool` — prevents re-triggering once a crossfade is in progress.
- `pending_start: Option<PendingStart>` — clip index and wall-clock start time for gap transitions.

Lifecycle for a crossfade:
1. `update` detects `position >= clip_dur - effective_xfade` → returns `StartCrossfade`.
2. `poll_track` takes the current sound out of `ManagedTrack.current_sound`, calls `transitions.begin_crossfade(old_sound, duration)`, then starts the next clip.
3. On subsequent ticks, `update` returns `CleanupOutgoing` once the fade duration has elapsed.
4. `poll_track` calls `transitions.cleanup_outgoing()`, which calls `sound::stop_quick` on the outgoing handle and drops it.

Lifecycle for a gap transition:
1. Sound stops → `update` returns `ScheduleGap { next_clip_index, delay_seconds }`.
2. `poll_track` sets `current_sound = None` and calls `transitions.schedule_gap(index, delay)`.
3. On subsequent ticks, `update` checks `Instant::now() >= pending_start.start_at` and returns `StartPendingClip`.

---

## How the Polling Loop Works

`lib.rs:playback_polling_loop` runs on a dedicated thread:

```
loop {
    sleep(50ms)
    audio.poll()       → drives transitions, collects TrackPlaybackState vec
    emit playback-state-update for each state
    emit master-volume-update
    server::broadcast_state  → push snapshot to WebSocket remotes
}
```

`AudioEngine::poll` (`mod.rs:poll`):
1. Collects all track IDs (excluding `__library`).
2. Calls `orchestrator::poll_track` for each.
3. Collects `TrackPlaybackState` from all tracks via `orchestrator::get_playback_state`.
4. Appends library sound state if `library_sound` is `Some`.
5. Returns the full state vec.

The frontend listens to `playback-state-update` events and stores them in `appStore.playbackStates` (a `Map<string, TrackPlaybackState>`).

---

## How to Add a New Transition Type

1. Add a variant to `TrackTransition` in `src-tauri/src/models.rs` and update the TypeScript bindings.
2. Add a new `TransitionAction` variant in `transitions.rs` if the new transition requires a new kind of orchestrator action.
3. Extend `TransitionManager::update` (`transitions.rs:update`) with a new branch that returns the new action when its trigger condition is met.
4. Handle the new `TransitionAction` variant in `orchestrator::poll_track` (`orchestrator.rs:poll_track`).
5. Update the UI track settings panel to expose the new option.

---

## How to Add a New Playback Feature

**New per-track behaviour** (e.g., a new mode type):
1. Add the mode variant to `TrackMode` in `models.rs`.
2. Handle it in `orchestrator::start_clip` for clip setup (e.g., loop region computation).
3. Handle it in `transitions.rs:pick_next_clip_index` for clip advancement logic.
4. Update the track settings UI.

**New engine-level operation** (e.g., solo):
1. Add the method to `AudioEngine` in `mod.rs`, delegating to orchestrator or mixer helpers.
2. Add a Tauri command in `commands.rs` and an action function in `actions.rs`.
3. Register the command in the `invoke_handler!` list in `lib.rs`.
4. Add an `invoke` call in `src/composables/useAudio.ts` and expose it from the composable.

**New library/preview feature** (e.g., pitch shifting):
1. Extend `PlayParams` in `sound.rs` with the new parameter.
2. Apply it inside `sound::play` before calling `track_handle.play`.
3. Pass the parameter through from `AudioEngine::play_library_sound` (and `start_clip` for track playback if applicable).
4. Extend the Tauri command `play_library_sound` and the frontend `playLibrarySound` call.
