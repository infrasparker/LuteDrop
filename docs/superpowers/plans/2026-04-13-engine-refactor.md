# Audio Engine Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the monolithic 978-line audio engine into focused modules (sound, mixer, transitions, orchestrator) with clear boundaries, unifying library and soundstage playback through a shared SoundPlayer.

**Architecture:** The current `engine.rs` is decomposed into 6 files. `SoundPlayer` (sound.rs) handles raw audio playback. `TransitionManager` (transitions.rs) manages crossfade/gap lifecycle per track. `TrackOrchestrator` (orchestrator.rs) sequences clips using the other modules. `AudioEngine` (mod.rs) is a thin public API. All library and soundstage playback flows through `SoundPlayer::play()`.

**Tech Stack:** Rust, kira 0.12, symphonia 0.5

**Spec:** `docs/superpowers/specs/2026-04-13-engine-refactor-design.md`

---

## File Structure

### New files

| File | Responsibility |
|---|---|
| `src-tauri/src/audio/util.rs` | `probe_duration`, `amplitude_to_decibels` |
| `src-tauri/src/audio/sound.rs` | `SoundPlayer`: load file, apply params, play/stop/pause/resume |
| `src-tauri/src/audio/mixer.rs` | `MuteState`, `compute_volume` |
| `src-tauri/src/audio/transitions.rs` | `TransitionManager`, `TransitionAction` |
| `src-tauri/src/audio/orchestrator.rs` | Track lifecycle, clip sequencing, polling |

### Modified files

| File | What changes |
|---|---|
| `src-tauri/src/audio/mod.rs` | Becomes the AudioEngine public API (replaces engine.rs re-export) |
| `src-tauri/src/audio/engine.rs` | Deleted (content distributed across new files) |
| `src-tauri/src/actions.rs` | Update engine method calls, rename preview to library_sound, standardize naming |
| `src-tauri/src/commands.rs` | Rename preview commands, add mute commands |
| `src-tauri/src/lib.rs` | Register new commands, fold persistence paths inline |
| `src-tauri/src/persistence.rs` | Deleted (paths folded into lib.rs) |
| `src-tauri/src/models.rs` | No changes |
| `src-tauri/src/server.rs` | Update preview → library_sound in remote command handler |
| `src/composables/useAudio.ts` | Rename playPreview/stopPreview to playLibrarySound/stopLibrarySound |
| `src/stores/appStore.ts` | Rename previewEntryId to libraryPlayingEntryId |
| `src/components/WaveformEditor.vue` | Update __preview to __library in playback state |
| `src/components/WaveformEditorDialog.vue` | Update preview references |
| `src/components/LibraryEntryRow.vue` | Update preview references |
| `src/App.vue` | Update preview references |
| `src-tauri/src/database/library.rs` | Remove dead `update_library_entry_trim` |
| `src-tauri/src/database/hierarchy.rs` | Remove dead `reorder_group`, `reorder_comp`, `reorder_track` |

### Deleted files

| File | Why |
|---|---|
| `src-tauri/src/audio/engine.rs` | Content distributed across new modules |
| `src-tauri/src/persistence.rs` | Two path functions folded into lib.rs |

---

## Task 1: Create util.rs — extract pure helpers

**Files:**
- Create: `src-tauri/src/audio/util.rs`
- Modify: `src-tauri/src/audio/mod.rs`

Extract `probe_duration` and `amplitude_to_decibels` from engine.rs into util.rs. These are pure functions with no engine dependencies.

- [ ] **Step 1: Create util.rs with the two functions**

Create `src-tauri/src/audio/util.rs` containing:
- `pub fn probe_duration(path: &str) -> Option<f64>` — copy from engine.rs lines 14-51, unchanged
- `pub fn amplitude_to_decibels(amp: f64) -> Decibels` — copy from engine.rs lines 59-65, make pub
- `pub const CROSSFADE_OVERLAP: f64 = 4.0;` — move constant here too

- [ ] **Step 2: Update mod.rs to declare the new module**

Add `pub mod util;` to `src-tauri/src/audio/mod.rs`.

- [ ] **Step 3: Update engine.rs to use util**

Replace the local `probe_duration`, `amplitude_to_decibels`, and `CROSSFADE_OVERLAP` with imports from `super::util`. Remove the original definitions. Add:
```rust
use super::util::{probe_duration, amplitude_to_decibels, CROSSFADE_OVERLAP};
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cd src-tauri && cargo check && cargo test`

- [ ] **Step 5: Commit**

```
refactor: extract probe_duration and amplitude_to_decibels to audio/util.rs
```

---

## Task 2: Create sound.rs — SoundPlayer

**Files:**
- Create: `src-tauri/src/audio/sound.rs`
- Modify: `src-tauri/src/audio/mod.rs`

The SoundPlayer is the core of the unification. Both library playback and soundstage playback will call these functions.

- [ ] **Step 1: Create sound.rs with types and play function**

Create `src-tauri/src/audio/sound.rs`:

```rust
use std::time::Duration;
use kira::sound::streaming::{StreamingSoundData, StreamingSoundHandle};
use kira::sound::{EndPosition, FromFileError, PlaybackPosition, PlaybackState, Region};
use kira::track::TrackHandle;
use kira::{AudioManager, Decibels, Easing, Tween};
use super::util::probe_duration;

pub struct SoundHandle {
    pub handle: StreamingSoundHandle<FromFileError>,
    pub fade_out: Option<f64>,
    pub duration: Option<f64>,
    pub start_position: Option<f64>,
}

pub struct PlayParams {
    pub file_path: String,
    pub start_position: f64,
    pub volume: Decibels,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub loop_region: Option<(f64, f64)>,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
}

pub fn play(
    manager: &mut AudioManager,
    track_handle: &TrackHandle,
    params: PlayParams,
) -> Result<SoundHandle, String> {
    let mut sound_data = StreamingSoundData::from_file(&params.file_path)
        .map_err(|e| format!("Failed to load audio file '{}': {}", params.file_path, e))?;

    // Apply start position
    if params.start_position > 0.0 {
        sound_data = sound_data.start_position(params.start_position);
    }

    // Apply loop region
    if let Some((loop_start, loop_end)) = params.loop_region {
        if loop_end > loop_start {
            sound_data = sound_data.loop_region(Region {
                start: PlaybackPosition::Seconds(loop_start),
                end: EndPosition::Custom(PlaybackPosition::Seconds(loop_end)),
            });
        }
    }

    // Apply fade in
    if let Some(fade_secs) = params.fade_in {
        if fade_secs > 0.0 {
            sound_data = sound_data.fade_in_tween(Tween {
                duration: Duration::from_secs_f64(fade_secs),
                easing: Easing::InPowi(2),
                ..Default::default()
            });
        }
    }

    // Apply volume
    sound_data = sound_data.volume(params.volume);

    // Play on the given mixer sub-track
    let handle = track_handle
        .play(sound_data)
        .map_err(|e| format!("Failed to play sound: {}", e))?;

    // Compute effective duration
    let raw_duration = probe_duration(&params.file_path);
    let duration = match (params.trim_end, raw_duration) {
        (Some(te), _) => Some(te),
        (None, Some(d)) => Some(d),
        _ => None,
    };

    Ok(SoundHandle {
        handle,
        fade_out: params.fade_out,
        duration,
        start_position: if params.start_position > 0.0 {
            Some(params.start_position)
        } else {
            params.trim_start
        },
    })
}

pub fn stop(sound: &mut SoundHandle, fade_override: Option<Duration>) {
    let fade = fade_override.unwrap_or_else(|| {
        sound.fade_out
            .map(|f| Duration::from_secs_f64(f))
            .unwrap_or(Duration::from_millis(10))
    });
    let _ = sound.handle.stop(Tween {
        duration: fade,
        ..Default::default()
    });
}

pub fn stop_quick(sound: &mut SoundHandle) {
    let _ = sound.handle.stop(Tween {
        duration: Duration::from_millis(10),
        ..Default::default()
    });
}

pub fn pause(sound: &mut SoundHandle) {
    sound.handle.pause(Tween {
        duration: Duration::from_millis(50),
        ..Default::default()
    });
}

pub fn resume(sound: &mut SoundHandle) {
    sound.handle.resume(Tween {
        duration: Duration::from_millis(50),
        ..Default::default()
    });
}

pub fn position(sound: &SoundHandle) -> f64 {
    sound.handle.position()
}

pub fn state(sound: &SoundHandle) -> PlaybackState {
    sound.handle.state()
}

pub fn set_volume(sound: &mut SoundHandle, volume: Decibels) {
    sound.handle.set_volume(volume, Tween {
        duration: Duration::from_millis(50),
        ..Default::default()
    });
}
```

- [ ] **Step 2: Register in mod.rs**

Add `pub mod sound;` to mod.rs.

- [ ] **Step 3: Verify compilation**

Run: `cd src-tauri && cargo check`

- [ ] **Step 4: Commit**

```
refactor: create audio/sound.rs with SoundPlayer (PlayParams, SoundHandle)
```

---

## Task 3: Create mixer.rs — volume computation and mute state

**Files:**
- Create: `src-tauri/src/audio/mixer.rs`

- [ ] **Step 1: Create mixer.rs**

```rust
use std::collections::HashMap;
use kira::Decibels;
use super::util::amplitude_to_decibels;

pub struct MuteState {
    pub master: bool,
    pub tracks: HashMap<String, bool>,
    pub clips: HashMap<String, bool>,
}

impl MuteState {
    pub fn new() -> Self {
        Self {
            master: false,
            tracks: HashMap::new(),
            clips: HashMap::new(),
        }
    }

    pub fn is_muted(&self, track_id: &str, clip_id: &str) -> bool {
        self.master
            || self.tracks.get(track_id).copied().unwrap_or(false)
            || self.clips.get(clip_id).copied().unwrap_or(false)
    }

    pub fn toggle_master(&mut self) -> bool {
        self.master = !self.master;
        self.master
    }

    pub fn set_track_muted(&mut self, track_id: &str, muted: bool) {
        self.tracks.insert(track_id.to_string(), muted);
    }

    pub fn set_clip_muted(&mut self, clip_id: &str, muted: bool) {
        self.clips.insert(clip_id.to_string(), muted);
    }
}

pub fn compute_volume(
    master: f64,
    track: f64,
    clip: f64,
    library: f64,
    mute_state: &MuteState,
    track_id: &str,
    clip_id: &str,
) -> Decibels {
    if mute_state.is_muted(track_id, clip_id) {
        return Decibels::SILENCE;
    }
    amplitude_to_decibels(master * track * clip * library)
}
```

- [ ] **Step 2: Register in mod.rs**

Add `pub mod mixer;` to mod.rs.

- [ ] **Step 3: Verify and commit**

Run: `cd src-tauri && cargo check`

```
refactor: create audio/mixer.rs with MuteState and compute_volume
```

---

## Task 4: Create transitions.rs — TransitionManager

**Files:**
- Create: `src-tauri/src/audio/transitions.rs`

- [ ] **Step 1: Create transitions.rs**

Extract crossfade trigger logic, gap scheduling, and outgoing fade tracking from engine.rs into a stateful TransitionManager.

```rust
use std::time::{Duration, Instant};
use kira::sound::PlaybackState;
use crate::models::{TrackMode, TrackTransition};
use super::sound::SoundHandle;
use super::util::CROSSFADE_OVERLAP;

pub enum TransitionAction {
    None,
    StartCrossfade { next_clip_index: usize, duration: f64 },
    AdvanceClip { next_clip_index: usize },
    ScheduleGap { next_clip_index: usize, delay_seconds: f64 },
    StartPendingClip { clip_index: usize },
    CleanupOutgoing,
}

pub struct OutgoingFade {
    pub handle: SoundHandle,
    pub started: Instant,
    pub duration: f64,
}

struct PendingStart {
    clip_index: usize,
    start_at: Instant,
}

pub struct TransitionManager {
    outgoing: Option<OutgoingFade>,
    crossfade_started: bool,
    pending_start: Option<PendingStart>,
}

impl TransitionManager {
    pub fn new() -> Self {
        Self {
            outgoing: None,
            crossfade_started: false,
            pending_start: None,
        }
    }

    /// Main polling tick. Called every 50ms.
    /// Returns the action the orchestrator should take.
    pub fn update(
        &self,
        position: f64,
        clip_duration: Option<f64>,
        clip_start: Option<f64>,
        mode: &TrackMode,
        current_clip_index: Option<usize>,
        clip_count: usize,
        is_paused: bool,
        sound_playing: bool,
        sound_stopped: bool,
    ) -> TransitionAction {
        if is_paused {
            return TransitionAction::None;
        }

        // Check for pending gap start
        if let Some(ref pending) = self.pending_start {
            if Instant::now() >= pending.start_at {
                return TransitionAction::StartPendingClip { clip_index: pending.clip_index };
            }
            return TransitionAction::None;
        }

        // Check for outgoing fade cleanup
        if let Some(ref outgoing) = self.outgoing {
            let should_cleanup = outgoing.started.elapsed() >= Duration::from_secs_f64(outgoing.duration + 0.5);
            if should_cleanup {
                return TransitionAction::CleanupOutgoing;
            }
        }

        // Check for sound finished naturally
        if sound_stopped && !is_paused {
            let next = pick_next_clip_index(mode, current_clip_index, clip_count);
            return match next {
                Some(idx) => {
                    // Determine transition type
                    let transition = match mode {
                        TrackMode::Sequence { transition, .. } => transition.clone(),
                        TrackMode::Shuffle { transition, .. } => transition.clone(),
                        TrackMode::Seamless { .. } => TrackTransition::Gapless,
                    };
                    match transition {
                        TrackTransition::Gapless => TransitionAction::AdvanceClip { next_clip_index: idx },
                        TrackTransition::Gap { min, max } => {
                            let mut rng = rand::thread_rng();
                            use rand::Rng;
                            let delay = rng.gen_range(min..=max);
                            TransitionAction::ScheduleGap { next_clip_index: idx, delay_seconds: delay }
                        }
                        TrackTransition::Crossfade => TransitionAction::AdvanceClip { next_clip_index: idx },
                    }
                }
                None => TransitionAction::None, // orchestrator will stop the track
            };
        }

        // Check for crossfade trigger
        if !self.crossfade_started && sound_playing {
            let crossfade_duration = match mode {
                TrackMode::Sequence { transition: TrackTransition::Crossfade, .. } => CROSSFADE_OVERLAP,
                TrackMode::Shuffle { transition: TrackTransition::Crossfade, .. } => CROSSFADE_OVERLAP,
                _ => return TransitionAction::None,
            };

            if let Some(clip_dur) = clip_duration {
                let start = clip_start.unwrap_or(0.0);
                let playable = clip_dur - start;
                let effective_xfade = if playable > crossfade_duration {
                    crossfade_duration
                } else {
                    (playable * 0.5).max(0.1)
                };

                if position >= clip_dur - effective_xfade {
                    let next = pick_next_clip_index(mode, current_clip_index, clip_count);
                    if let Some(idx) = next {
                        return TransitionAction::StartCrossfade {
                            next_clip_index: idx,
                            duration: effective_xfade,
                        };
                    }
                }
            }
        }

        TransitionAction::None
    }

    pub fn begin_crossfade(&mut self, outgoing_handle: SoundHandle, duration: f64) {
        self.outgoing = Some(OutgoingFade {
            handle: outgoing_handle,
            started: Instant::now(),
            duration,
        });
        self.crossfade_started = true;
    }

    pub fn schedule_gap(&mut self, clip_index: usize, delay: f64) {
        self.pending_start = Some(PendingStart {
            clip_index,
            start_at: Instant::now() + Duration::from_secs_f64(delay),
        });
    }

    pub fn clip_started(&mut self) {
        self.crossfade_started = false;
        self.pending_start = None;
    }

    pub fn cleanup_outgoing(&mut self) {
        if let Some(ref mut outgoing) = self.outgoing {
            super::sound::stop_quick(&mut outgoing.handle);
        }
        self.outgoing = None;
    }

    pub fn reset(&mut self) {
        self.cleanup_outgoing();
        self.crossfade_started = false;
        self.pending_start = None;
    }

    pub fn has_pending(&self) -> bool {
        self.pending_start.is_some()
    }
}

/// Determine the next clip index. Moved from engine.rs.
fn pick_next_clip_index(mode: &TrackMode, current_index: Option<usize>, clip_count: usize) -> Option<usize> {
    if clip_count == 0 {
        return None;
    }
    match mode {
        TrackMode::Seamless { .. } => None,
        TrackMode::Sequence { repeat, .. } => {
            let current = current_index.unwrap_or(0);
            let next = current + 1;
            if next < clip_count {
                Some(next)
            } else if *repeat {
                Some(0)
            } else {
                None
            }
        }
        TrackMode::Shuffle { repeat, .. } => {
            if *repeat || clip_count > 1 {
                use rand::Rng;
                let mut rng = rand::thread_rng();
                if clip_count > 1 {
                    loop {
                        let idx = rng.gen_range(0..clip_count);
                        if Some(idx) != current_index {
                            return Some(idx);
                        }
                    }
                } else {
                    Some(0)
                }
            } else {
                None
            }
        }
    }
}
```

- [ ] **Step 2: Register in mod.rs**

Add `pub mod transitions;` to mod.rs.

- [ ] **Step 3: Verify and commit**

Run: `cd src-tauri && cargo check`

```
refactor: create audio/transitions.rs with TransitionManager
```

---

## Task 5: Create orchestrator.rs — track lifecycle

**Files:**
- Create: `src-tauri/src/audio/orchestrator.rs`

- [ ] **Step 1: Create orchestrator.rs**

Extract track management logic: setup_track, play_track, stop_track, pause/resume, polling, clip selection. All functions receive the tracks HashMap and other dependencies as parameters.

```rust
use std::collections::HashMap;
use kira::sound::PlaybackState;
use kira::track::TrackBuilder;
use kira::AudioManager;
use rand::Rng;
use crate::models::{Track, TrackMode, TrackPlaybackState};
use super::sound::{self, SoundHandle, PlayParams};
use super::mixer::{self, MuteState};
use super::transitions::{TransitionManager, TransitionAction};
use super::util::probe_duration;

pub struct ManagedTrack {
    pub track_handle: kira::track::TrackHandle,
    pub config: Track,
    pub current_sound: Option<SoundHandle>,
    pub current_clip_index: Option<usize>,
    pub is_paused: bool,
    pub transitions: TransitionManager,
}

pub fn setup_track(
    manager: &mut AudioManager,
    tracks: &mut HashMap<String, ManagedTrack>,
    config: &Track,
) -> Result<(), String> {
    if tracks.contains_key(&config.id) {
        update_track_config(tracks, config);
        return Ok(());
    }

    let track_handle = manager
        .add_sub_track(TrackBuilder::new())
        .map_err(|e| format!("Failed to create sub-track: {}", e))?;

    tracks.insert(config.id.clone(), ManagedTrack {
        track_handle,
        config: config.clone(),
        current_sound: None,
        current_clip_index: None,
        is_paused: false,
        transitions: TransitionManager::new(),
    });

    Ok(())
}

pub fn update_track_config(tracks: &mut HashMap<String, ManagedTrack>, config: &Track) {
    if let Some(track) = tracks.get_mut(&config.id) {
        track.config = config.clone();
    }
}

pub fn has_track(tracks: &HashMap<String, ManagedTrack>, track_id: &str) -> bool {
    tracks.contains_key(track_id)
}

pub fn play_track(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let is_paused = tracks.get(track_id).map(|t| t.is_paused).unwrap_or(false);
    if is_paused {
        return resume_track(tracks, track_id);
    }

    let clip_index = pick_first_clip(tracks, track_id)?;
    match clip_index {
        Some(idx) => start_clip(tracks, manager, track_id, idx, None, true, master_volume, mute_state),
        None => Err(format!("Track {} has no clips", track_id)),
    }
}

pub fn play_track_clip(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    clip_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let clip_index = {
        let track = tracks.get(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
        track.config.clips.iter().position(|c| c.id == clip_id)
            .ok_or_else(|| format!("Clip not found: {}", clip_id))?
    };
    start_clip(tracks, manager, track_id, clip_index, None, true, master_volume, mute_state)
}

pub fn play_track_from(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    position: f64,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let clip_index = pick_first_clip(tracks, track_id)?;
    match clip_index {
        Some(idx) => start_clip(tracks, manager, track_id, idx, Some(position), true, master_volume, mute_state),
        None => Err(format!("Track {} has no clips", track_id)),
    }
}

pub fn pause_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    track.is_paused = true;
    track.transitions.reset();
    if let Some(ref mut snd) = track.current_sound {
        sound::pause(snd);
    }
    Ok(())
}

pub fn resume_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    track.is_paused = false;
    if let Some(ref mut snd) = track.current_sound {
        sound::resume(snd);
    }
    Ok(())
}

pub fn stop_track(tracks: &mut HashMap<String, ManagedTrack>, track_id: &str) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    track.is_paused = false;
    track.current_clip_index = None;
    track.transitions.reset();
    if let Some(ref mut snd) = track.current_sound {
        let fade = if track.config.fade_out > 0.0 {
            Some(std::time::Duration::from_secs_f64(track.config.fade_out))
        } else {
            None
        };
        sound::stop(snd, fade);
    }
    track.current_sound = None;
    Ok(())
}

pub fn stop_all(tracks: &mut HashMap<String, ManagedTrack>) {
    let ids: Vec<String> = tracks.keys().cloned().collect();
    for id in ids {
        let _ = stop_track(tracks, &id);
    }
}

pub fn pause_all(tracks: &mut HashMap<String, ManagedTrack>) {
    let ids: Vec<String> = tracks.keys().cloned().collect();
    for id in ids {
        let is_playing = tracks.get(&id).map(|t| {
            if t.is_paused { return false; }
            match &t.current_sound {
                Some(snd) => matches!(sound::state(snd), PlaybackState::Playing | PlaybackState::Resuming),
                None => t.transitions.has_pending(),
            }
        }).unwrap_or(false);
        if is_playing {
            let _ = pause_track(tracks, &id);
        }
    }
}

pub fn resume_all(tracks: &mut HashMap<String, ManagedTrack>) {
    let ids: Vec<String> = tracks.keys().cloned().collect();
    for id in ids {
        let is_paused = tracks.get(&id).map(|t| t.is_paused).unwrap_or(false);
        if is_paused {
            let _ = resume_track(tracks, &id);
        }
    }
}

pub fn set_track_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    track_id: &str,
    volume: f64,
    master_volume: f64,
    mute_state: &MuteState,
) {
    let found = if let Some(track) = tracks.get_mut(track_id) {
        track.config.volume = volume.clamp(0.0, 1.0);
        true
    } else {
        false
    };
    if found {
        update_sound_volume(tracks, track_id, master_volume, mute_state);
    }
}

pub fn set_clip_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    clip_id: &str,
    volume: f64,
    master_volume: f64,
    mute_state: &MuteState,
) {
    let volume = volume.clamp(0.0, 1.0);
    let mut found_track_id: Option<String> = None;
    for (track_id, track) in tracks.iter_mut() {
        for clip in &mut track.config.clips {
            if clip.id == clip_id {
                clip.volume = volume;
                found_track_id = Some(track_id.clone());
                break;
            }
        }
        if found_track_id.is_some() { break; }
    }
    if let Some(track_id) = found_track_id {
        update_sound_volume(tracks, &track_id, master_volume, mute_state);
    }
}

/// Poll a single track for transition actions and execute them.
pub fn poll_track(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
) {
    // Gather state for transition check
    let (position, clip_duration, clip_start, mode, current_clip_index, clip_count, is_paused, sound_playing, sound_stopped) = {
        let track = match tracks.get(track_id) {
            Some(t) => t,
            None => return,
        };
        let (pos, playing, stopped) = match &track.current_sound {
            Some(snd) => {
                let st = sound::state(snd);
                (sound::position(snd),
                 matches!(st, PlaybackState::Playing | PlaybackState::Resuming),
                 matches!(st, PlaybackState::Stopped))
            }
            None => (0.0, false, false),
        };
        let dur = track.current_sound.as_ref().and_then(|s| s.duration);
        let start = track.current_sound.as_ref().and_then(|s| s.start_position);
        (pos, dur, start, track.config.mode.clone(), track.current_clip_index, track.config.clips.len(), track.is_paused, playing, stopped)
    };

    let action = {
        let track = match tracks.get(track_id) {
            Some(t) => t,
            None => return,
        };
        track.transitions.update(
            position, clip_duration, clip_start, &mode,
            current_clip_index, clip_count, is_paused, sound_playing, sound_stopped,
        )
    };

    match action {
        TransitionAction::StartCrossfade { next_clip_index, duration } => {
            // Take current sound as outgoing
            if let Some(track) = tracks.get_mut(track_id) {
                let old_sound = track.current_sound.take();
                if let Some(old) = old_sound {
                    track.transitions.begin_crossfade(old, duration);
                }
            }
            // Start new clip (no track fade)
            let _ = start_clip(tracks, manager, track_id, next_clip_index, None, false, master_volume, mute_state);
        }
        TransitionAction::AdvanceClip { next_clip_index } => {
            let _ = start_clip(tracks, manager, track_id, next_clip_index, None, false, master_volume, mute_state);
        }
        TransitionAction::ScheduleGap { next_clip_index, delay_seconds } => {
            if let Some(track) = tracks.get_mut(track_id) {
                track.current_sound = None;
                track.transitions.schedule_gap(next_clip_index, delay_seconds);
            }
        }
        TransitionAction::StartPendingClip { clip_index } => {
            let _ = start_clip(tracks, manager, track_id, clip_index, None, false, master_volume, mute_state);
        }
        TransitionAction::CleanupOutgoing => {
            if let Some(track) = tracks.get_mut(track_id) {
                track.transitions.cleanup_outgoing();
            }
        }
        TransitionAction::None => {
            // Check if sound stopped naturally (no transition action means end of track)
            if sound_stopped && !is_paused {
                let _ = stop_track(tracks, track_id);
            }
        }
    }
}

pub fn get_playback_state(track: &ManagedTrack) -> TrackPlaybackState {
    let (is_playing, is_paused_state, current_time) = match &track.current_sound {
        Some(snd) => {
            let st = sound::state(snd);
            let playing = matches!(st, PlaybackState::Playing | PlaybackState::Resuming);
            let paused = matches!(st, PlaybackState::Paused | PlaybackState::Pausing | PlaybackState::WaitingToResume);
            (playing, paused, sound::position(snd))
        }
        None => (track.transitions.has_pending(), track.is_paused, 0.0),
    };

    let current_clip_id = track.current_clip_index
        .and_then(|idx| track.config.clips.get(idx))
        .map(|clip| clip.id.clone());

    TrackPlaybackState {
        track_id: track.config.id.clone(),
        is_playing,
        is_paused: is_paused_state,
        current_clip_id,
        current_time,
    }
}

// --- Internal helpers ---

fn start_clip(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    clip_index: usize,
    start_position: Option<f64>,
    apply_track_fade: bool,
    master_volume: f64,
    mute_state: &MuteState,
) -> Result<(), String> {
    let track = tracks.get_mut(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    let clip = track.config.clips.get(clip_index)
        .ok_or_else(|| format!("Clip index {} out of range", clip_index))?.clone();

    // Stop current sound
    if let Some(ref mut snd) = track.current_sound {
        sound::stop_quick(snd);
    }

    // Compute volume
    let volume = mixer::compute_volume(
        master_volume, track.config.volume, clip.volume, clip.library_volume,
        mute_state, &track.config.id, &clip.id,
    );

    // Build play params
    let effective_start = start_position.or(clip.trim_start).unwrap_or(0.0);

    // Compute loop region for seamless mode
    let loop_region = match &track.config.mode {
        TrackMode::Seamless { loop_start, loop_end } => {
            let ls = clip.trim_start.map(|ts| (*loop_start).max(ts)).unwrap_or(*loop_start);
            let raw_le = if *loop_end > 0.0 { *loop_end } else { clip.trim_end.unwrap_or(f64::MAX) };
            let le = clip.trim_end.map(|te| raw_le.min(te)).unwrap_or(raw_le);
            if le < f64::MAX && le > ls {
                Some((ls, le))
            } else {
                None
            }
        }
        _ => None,
    };

    let fade_in = if apply_track_fade && track.config.fade_in > 0.0 {
        Some(track.config.fade_in)
    } else {
        None
    };

    let params = PlayParams {
        file_path: clip.file_path.clone(),
        start_position: effective_start,
        volume,
        trim_start: clip.trim_start,
        trim_end: clip.trim_end,
        loop_region,
        fade_in,
        fade_out: if track.config.fade_out > 0.0 { Some(track.config.fade_out) } else { None },
    };

    let snd = sound::play(manager, &track.track_handle, params)?;
    track.current_sound = Some(snd);
    track.current_clip_index = Some(clip_index);
    track.transitions.clip_started();
    track.is_paused = false;

    Ok(())
}

fn pick_first_clip(tracks: &HashMap<String, ManagedTrack>, track_id: &str) -> Result<Option<usize>, String> {
    let track = tracks.get(track_id).ok_or_else(|| format!("Track not found: {}", track_id))?;
    if track.config.clips.is_empty() {
        return Ok(None);
    }
    match &track.config.mode {
        TrackMode::Shuffle { .. } => {
            let mut rng = rand::thread_rng();
            Ok(Some(rng.gen_range(0..track.config.clips.len())))
        }
        _ => Ok(Some(0)),
    }
}

fn update_sound_volume(
    tracks: &mut HashMap<String, ManagedTrack>,
    track_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
) {
    if let Some(track) = tracks.get_mut(track_id) {
        let (clip_vol, lib_vol, clip_id) = track.current_clip_index
            .and_then(|idx| track.config.clips.get(idx))
            .map(|c| (c.volume, c.library_volume, c.id.clone()))
            .unwrap_or((1.0, 1.0, String::new()));

        let vol = mixer::compute_volume(
            master_volume, track.config.volume, clip_vol, lib_vol,
            mute_state, &track.config.id, &clip_id,
        );

        if let Some(ref mut snd) = track.current_sound {
            sound::set_volume(snd, vol);
        }
    }
}
```

- [ ] **Step 2: Register in mod.rs**

Add `pub mod orchestrator;` to mod.rs.

- [ ] **Step 3: Verify compilation**

Run: `cd src-tauri && cargo check`

- [ ] **Step 4: Commit**

```
refactor: create audio/orchestrator.rs with ManagedTrack and track lifecycle
```

---

## Task 6: Rewrite mod.rs as AudioEngine public API, delete engine.rs

**Files:**
- Rewrite: `src-tauri/src/audio/mod.rs`
- Delete: `src-tauri/src/audio/engine.rs`

- [ ] **Step 1: Rewrite mod.rs**

Replace mod.rs entirely. The AudioEngine is a thin coordinator that owns the AudioManager, tracks HashMap, MuteState, and library sound handle. Each method delegates to the appropriate module.

```rust
pub mod mixer;
pub mod orchestrator;
pub mod sound;
pub mod transitions;
pub mod util;

use std::collections::HashMap;
use kira::track::TrackBuilder;
use kira::{AudioManager, AudioManagerSettings};

use crate::models::{Track, TrackPlaybackState};
use mixer::MuteState;
use orchestrator::ManagedTrack;
use sound::{PlayParams, SoundHandle};
use util::amplitude_to_decibels;

const LIBRARY_TRACK_ID: &str = "__library";

pub struct AudioEngine {
    manager: AudioManager,
    tracks: HashMap<String, ManagedTrack>,
    master_volume: f64,
    mute_state: MuteState,
    library_sound: Option<SoundHandle>,
    library_track_handle: kira::track::TrackHandle,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let mut manager = AudioManager::new(AudioManagerSettings::default())
            .map_err(|e| format!("Failed to create audio manager: {}", e))?;
        let library_track_handle = manager.add_sub_track(TrackBuilder::default())
            .map_err(|e| format!("Failed to create library track: {}", e))?;
        Ok(Self {
            manager,
            tracks: HashMap::new(),
            master_volume: 1.0,
            mute_state: MuteState::new(),
            library_sound: None,
            library_track_handle,
        })
    }

    // --- Track operations ---

    pub fn setup_track(&mut self, config: &Track) -> Result<(), String> {
        orchestrator::setup_track(&mut self.manager, &mut self.tracks, config)
    }

    pub fn update_track_config(&mut self, config: &Track) {
        orchestrator::update_track_config(&mut self.tracks, config);
    }

    pub fn has_track(&self, track_id: &str) -> bool {
        orchestrator::has_track(&self.tracks, track_id)
    }

    pub fn play_track(&mut self, track_id: &str) -> Result<(), String> {
        self.stop_library_sound();
        orchestrator::play_track(&mut self.tracks, &mut self.manager, track_id, self.master_volume, &self.mute_state)
    }

    pub fn play_track_clip(&mut self, track_id: &str, clip_id: &str) -> Result<(), String> {
        self.stop_library_sound();
        orchestrator::play_track_clip(&mut self.tracks, &mut self.manager, track_id, clip_id, self.master_volume, &self.mute_state)
    }

    pub fn play_track_from(&mut self, track_id: &str, position: f64) -> Result<(), String> {
        orchestrator::play_track_from(&mut self.tracks, &mut self.manager, track_id, position, self.master_volume, &self.mute_state)
    }

    pub fn pause_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::pause_track(&mut self.tracks, track_id)
    }

    pub fn resume_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::resume_track(&mut self.tracks, track_id)
    }

    pub fn stop_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::stop_track(&mut self.tracks, track_id)
    }

    pub fn stop_all(&mut self) {
        orchestrator::stop_all(&mut self.tracks);
        self.stop_library_sound();
    }

    pub fn pause_all(&mut self) {
        orchestrator::pause_all(&mut self.tracks);
    }

    pub fn resume_all(&mut self) {
        orchestrator::resume_all(&mut self.tracks);
    }

    // --- Volume ---

    pub fn get_master_volume(&self) -> f64 {
        self.master_volume
    }

    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        let ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in ids {
            orchestrator::set_track_volume(&mut self.tracks, &id, 
                self.tracks.get(&id).map(|t| t.config.volume).unwrap_or(1.0),
                self.master_volume, &self.mute_state);
        }
        // Update library sound volume too
        if let Some(ref mut snd) = self.library_sound {
            sound::set_volume(snd, amplitude_to_decibels(self.master_volume));
        }
    }

    pub fn set_track_volume(&mut self, track_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_track_volume(&mut self.tracks, track_id, volume, self.master_volume, &self.mute_state);
        Ok(())
    }

    pub fn set_clip_volume(&mut self, clip_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_clip_volume(&mut self.tracks, clip_id, volume, self.master_volume, &self.mute_state);
        Ok(())
    }

    // --- Mute ---

    pub fn mute_master(&mut self) { self.mute_state.master = true; self.refresh_all_volumes(); }
    pub fn unmute_master(&mut self) { self.mute_state.master = false; self.refresh_all_volumes(); }
    pub fn mute_track(&mut self, track_id: &str) { self.mute_state.set_track_muted(track_id, true); self.refresh_track_volume(track_id); }
    pub fn unmute_track(&mut self, track_id: &str) { self.mute_state.set_track_muted(track_id, false); self.refresh_track_volume(track_id); }
    pub fn mute_clip(&mut self, clip_id: &str) { self.mute_state.set_clip_muted(clip_id, true); /* refresh would need track lookup */ }
    pub fn unmute_clip(&mut self, clip_id: &str) { self.mute_state.set_clip_muted(clip_id, false); }

    // --- Library playback ---

    pub fn play_library_sound(&mut self, params: PlayParams) -> Result<(), String> {
        self.stop_library_sound();
        let snd = sound::play(&mut self.manager, &self.library_track_handle, params)?;
        self.library_sound = Some(snd);
        Ok(())
    }

    pub fn stop_library_sound(&mut self) {
        if let Some(ref mut snd) = self.library_sound {
            sound::stop(snd, None);
        }
        self.library_sound = None;
    }

    // --- Polling ---

    pub fn poll(&mut self) -> Vec<TrackPlaybackState> {
        let track_ids: Vec<String> = self.tracks.keys()
            .filter(|id| id.as_str() != LIBRARY_TRACK_ID)
            .cloned()
            .collect();

        for id in &track_ids {
            orchestrator::poll_track(&mut self.tracks, &mut self.manager, id, self.master_volume, &self.mute_state);
        }

        let mut states: Vec<TrackPlaybackState> = self.tracks.values()
            .filter(|t| t.config.id != LIBRARY_TRACK_ID)
            .map(orchestrator::get_playback_state)
            .collect();

        // Add library sound state
        if let Some(ref snd) = self.library_sound {
            let st = sound::state(snd);
            states.push(TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: matches!(st, kira::sound::PlaybackState::Playing | kira::sound::PlaybackState::Resuming),
                is_paused: matches!(st, kira::sound::PlaybackState::Paused | kira::sound::PlaybackState::Pausing),
                current_clip_id: None,
                current_time: sound::position(snd),
            });
        }

        states
    }

    // --- Internal ---

    fn refresh_all_volumes(&mut self) {
        let ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in ids {
            self.refresh_track_volume(&id);
        }
    }

    fn refresh_track_volume(&mut self, track_id: &str) {
        if let Some(track) = self.tracks.get(track_id) {
            let vol = track.config.volume;
            orchestrator::set_track_volume(&mut self.tracks, track_id, vol, self.master_volume, &self.mute_state);
        }
    }
}
```

- [ ] **Step 2: Delete engine.rs**

Delete `src-tauri/src/audio/engine.rs`.

- [ ] **Step 3: Update imports across the codebase**

In `src-tauri/src/actions.rs`, update any imports. The public API of `AudioEngine` hasn't changed method names (play_track, stop_track, etc.) except:
- `play_preview` → `play_library_sound`
- `stop_preview` → `stop_library_sound`
- Preview takes `PlayParams` instead of individual args

Update `play_preview_audio` and `stop_preview_audio` in actions.rs:

```rust
pub fn play_library_sound(
    container: &AppStateContainer,
    file_path: &str,
    position: f64,
    volume: f64,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    let vol = crate::audio::util::amplitude_to_decibels(audio.get_master_volume() * volume);
    let loop_region = match (loop_start, loop_end) {
        (Some(ls), Some(le)) if le > ls => Some((ls, le)),
        _ => None,
    };
    audio.play_library_sound(crate::audio::sound::PlayParams {
        file_path: file_path.to_string(),
        start_position: position,
        volume: vol,
        trim_start,
        trim_end,
        loop_region,
        fade_in: None,
        fade_out: None,
    })
}

pub fn stop_library_sound(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_library_sound();
    Ok(())
}
```

Also update `check_crossfade_triggers` and `check_finished_sounds` calls in lib.rs — these are now handled inside `audio.poll()`. Replace the polling loop in lib.rs:

```rust
fn playback_polling_loop(app_handle: tauri::AppHandle, container: Arc<AppStateContainer>) {
    loop {
        std::thread::sleep(Duration::from_millis(50));

        let (states, master_vol) = {
            let mut audio = match container.audio.lock() {
                Ok(a) => a,
                Err(_) => continue,
            };
            let states = audio.poll();
            let vol = audio.get_master_volume();
            (states, vol)
        };

        for state in &states {
            let _ = app_handle.emit("playback-state-update", state);
        }
        let _ = app_handle.emit("master-volume-update", master_vol);

        server::broadcast_state(&container);
    }
}
```

- [ ] **Step 4: Update commands.rs**

Rename `play_preview` → `play_library_sound`, `stop_preview` → `stop_library_sound` in commands.rs. Update the function names in the invoke_handler in lib.rs.

- [ ] **Step 5: Verify compilation and all tests pass**

Run: `cd src-tauri && cargo check && cargo test`

Fix any compilation errors — the most common will be import paths changing.

- [ ] **Step 6: Commit**

```
refactor: rewrite AudioEngine as thin coordinator, delete engine.rs
```

---

## Task 7: Frontend renames and dead code cleanup

**Files:**
- Modify: `src/composables/useAudio.ts`
- Modify: `src/stores/appStore.ts`
- Modify: `src/components/WaveformEditor.vue`
- Modify: `src/components/WaveformEditorDialog.vue`
- Modify: `src/components/LibraryEntryRow.vue`
- Modify: `src/App.vue`
- Modify: `src-tauri/src/server.rs`
- Modify: `src-tauri/src/database/library.rs`
- Modify: `src-tauri/src/database/hierarchy.rs`
- Delete: `src-tauri/src/persistence.rs`

- [ ] **Step 1: Rename preview to library_sound in useAudio.ts**

- `playPreview(...)` → `playLibrarySound(...)`
- `stopPreview()` → `stopLibrarySound()`
- Invoke names: `'play_preview'` → `'play_library_sound'`, `'stop_preview'` → `'stop_library_sound'`

- [ ] **Step 2: Rename in appStore.ts**

- `previewEntryId` → `libraryPlayingEntryId`

- [ ] **Step 3: Update all component references**

In `WaveformEditor.vue`: `__preview` → `__library` in playback state lookup.

In `WaveformEditorDialog.vue`: `previewEntryId` → `libraryPlayingEntryId`, `playPreview` → `playLibrarySound`, `stopPreview` → `stopLibrarySound`.

In `LibraryEntryRow.vue`: same renames.

In `App.vue`: `previewEntryId` → `libraryPlayingEntryId`, `stopPreview` → `stopLibrarySound`.

- [ ] **Step 4: Update server.rs remote command handler**

Update `"play_preview"` → `"play_library_sound"`, `"stop_preview"` → `"stop_library_sound"` in the match arms. Also add `"pause_comp"` and `"resume_comp"` if not already present.

- [ ] **Step 5: Remove dead database code**

In `src-tauri/src/database/library.rs`: remove `update_library_entry_trim` (the `#[allow(dead_code)]` function).

In `src-tauri/src/database/hierarchy.rs`: remove `reorder_group`, `reorder_comp`, `reorder_track` (all `#[allow(dead_code)]`).

- [ ] **Step 6: Fold persistence.rs into lib.rs**

Move `json_data_file_path` and `db_file_path` from `persistence.rs` directly into `lib.rs` as private functions. Delete `persistence.rs`. Remove `mod persistence;` from lib.rs and update the calls.

- [ ] **Step 7: Verify everything compiles and works**

Run: `cd src-tauri && cargo check && cargo test`
Run: `npx vue-tsc --noEmit`

- [ ] **Step 8: Commit**

```
refactor: rename preview to library_sound, remove dead code, fold persistence
```

---

## Task 8: Write engine architecture documentation

**Files:**
- Create: `docs/engine-architecture.md`

- [ ] **Step 1: Write the living documentation**

Create `docs/engine-architecture.md` explaining:
- Module structure and responsibilities
- Dependency flow diagram
- How a sound gets played (SoundPlayer → PlayParams → SoundHandle)
- How tracks work (orchestrator → transitions → sound player)
- How library playback works (engine → sound player directly)
- Volume chain (mixer.rs)
- Mute system
- Transition lifecycle (TransitionManager states)
- How the polling loop works
- How to add a new transition type
- How to add a new playback feature

- [ ] **Step 2: Commit**

```
docs: engine architecture living documentation
```
