# Engine Architecture

> For a plain-language tour of the codebase with a Rust-for-TypeScript primer,
> see [`docs/orientation.md`](./orientation.md). This document is the precise
> reference; that one is the on-ramp.

## Summary

LuteDrop's real-time audio is owned by **miniaudio**, a single-header C library
vendored under `src-tauri/vendor/` (`miniaudio.h` + the `miniaudio_libopus`
backend for `.opus`). miniaudio runs its own high-priority audio thread and does
all the sample-by-sample work: decoding, mixing, fading, and pushing PCM
(raw audio samples) to the sound card. The Rust side never touches a sample.

Instead, Rust is a **20 Hz control plane**: a polling loop wakes every 50 ms
(so ~20 times a second), looks at where each sound is, and tells miniaudio what
to do next — start this clip, fade that one out, schedule a loop restart. It
talks to miniaudio through a tiny C shim (`vendor/shim.c` / `shim.h`) exposing a
flat `ld_*` function API, wrapped in safe Rust in `audio/ffi.rs`.

**None of this is [Kira](https://github.com/tesselode/kira) anymore.** The old
engine doc described a Kira-based design (`compute_volume`, `Decibels`,
`StreamingSoundHandle`, a `mixer` that computed dB) — all deleted. If you see
those names anywhere, they are stale. The volume model is now linear and
multiplied natively by miniaudio's node graph; there is no dB math and no
per-tick volume recompute.

---

## The layers

Each layer only knows about the one below it. Top to bottom:

```
Vue UI  ──invoke──▶  Tauri command (commands.rs)
                          │
                          ▼
                     actions.rs          ← locks state, talks to DB + engine
                          │
                          ▼
                  AudioEngine (audio/mod.rs)   ← the public engine API
                    │                    │
                    ▼                    ▼
          orchestrator.rs         (library preview)
          + transitions.rs         uses a Voice directly
                    │
                    ▼
                 voice.rs          ← plays ONE sound at a time, click-free
                    │
                    ▼
                 sound.rs          ← one clip = one ma_sound (PlayParams)
                    │
                    ▼
                  ffi.rs           ← safe Rust wrappers (Engine/Group/Sound)
                    │
                    ▼
             shim.c / shim.h       ← flat C API over miniaudio
                    │
                    ▼
              miniaudio (C)        ← the actual audio thread
```

One sentence per layer:

- **`shim.c` / `shim.h`** — a flat C API (`ld_engine_*`, `ld_group_*`,
  `ld_sound_*`, `ld_decoder_*`) that owns the miniaudio objects and hands Rust
  back opaque pointers; also opens files via the wide (UTF-16) `_w` APIs so
  non-ASCII paths work on Windows.
- **`ffi.rs`** — safe Rust wrappers (`Engine`, `Group`, `Sound`,
  `StreamingDecoder`) around those pointers; the only file with `unsafe` blocks,
  and the place the drop-order contract is written down.
- **`sound.rs`** — turns a `PlayParams` into one running `ma_sound`
  (`SoundHandle`); thin verbs like `play`, `stop`, `pause`, `position`, `state`.
- **`voice.rs`** — the `Voice`: plays exactly ONE sound at a time inside its own
  sub-mix group, with click-free stops (the fade-out reaper) and seamless-loop
  crossfading. Both the library preview and every track embed a `Voice`.
- **`orchestrator.rs`** (+ **`transitions.rs`**) — multi-clip sequencing on top
  of a `Voice`: which clip plays next, when to crossfade, gap timing, shuffle.
  `ManagedTrack` is one track's state; `TransitionManager` is its state machine.
- **`AudioEngine` (`mod.rs`)** — the public surface: owns all tracks, the
  library `Voice`, master volume, and mute state; every method the rest of the
  app calls lives here.
- **`actions.rs`** — the glue: locks the engine mutex, reads/writes the DB,
  converts command arguments into engine calls.
- **Tauri commands (`commands.rs`)** — the `#[tauri::command]` functions the Vue
  frontend can `invoke`, plus the remote's WebSocket dispatch in `server.rs`.

`levels.rs` and `util.rs` sit to the side: `levels.rs` computes the linear volume
for each node and tracks mute intent; `util.rs` holds constants
(`CROSSFADE_OVERLAP`, `AUDIO_EXTENSIONS`) and offline helpers (`probe_duration`,
`is_audio_path`).

---

## The volume model

miniaudio arranges sounds in a **node graph** — a tree where each node has its
own volume and a node's output is multiplied by its parent's on the way to the
speakers. LuteDrop uses exactly three levels, and lets miniaudio do the
multiplication natively:

| Level  | miniaudio node        | Set via                                    | Stored on            |
|--------|-----------------------|--------------------------------------------|----------------------|
| master | engine node           | `Engine::set_master_volume`                | `AudioEngine.master_volume` |
| track  | sub-mix group node    | `Group::set_volume` (via `Voice::set_group_volume`) | `Track.volume`       |
| clip   | individual sound node | `Sound::set_volume` (via `Voice::set_volume`) | `Clip.volume` × `Clip.library_volume` |

Final loudness = `master × track × clip` — but **no Rust code multiplies those
together**. Each level is set on its own node and miniaudio combines them on the
audio thread. This is why the old `compute_volume` / dB conversion is gone: there
is nothing to recompute per tick.

`levels.rs` provides the two small linear helpers that feed the group and sound
nodes:

- `levels::track_group_volume(track_vol, muted)` → group node (`0.0` if muted).
- `levels::clip_sound_volume(clip_vol, library_vol, muted)` → sound node
  (`clip × library`, clamped to `0..=1`, or `0.0` if muted).

**Mute = zero the relevant node.** There is no separate "silence" state:

- master mute → `engine.set_master_volume(0.0)` (`AudioEngine::toggle_master_mute`)
- track mute → `refresh_track_group_volume` sets the group node to 0
- clip mute → `set_clip_volume` re-applies the sound node at 0

`levels::MuteState` (`master: bool`, `tracks`/`clips: HashMap<String, bool>`) only
records **intent** — which things the user muted — so it can restore the
intended level on unmute and report a snapshot to remotes. It never stores actual
volumes.

---

## How a sound plays

Everything that makes noise starts as a `PlayParams` (`sound.rs`) and ends as a
running `ma_sound`:

```rust
pub struct PlayParams {
    pub file_path: String,
    pub start_position: f64,
    pub volume: f32,            // linear 0..=1 (the clip/sound node)
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub loop_region: Option<(f64, f64)>,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
}
```

`sound::play(engine, group, params)`:

1. `engine.create_sound(Some(group), path, stream=true)` → a **streaming**
   `ma_sound` (decoded on the fly as it plays, so a long ambience track doesn't
   sit in RAM). Every format, including `.opus`, streams natively now — there is
   no decode-to-WAV cache and no `decode_file`.
2. `sound.set_volume(params.volume)` sets the sound (clip) node.
3. `sound.play(start_position, fade_in)` seeks and starts.
4. Wraps it in a `SoundHandle` caching `fade_out`, effective `duration`
   (explicit `trim_end`, else the file length from `length_secs`),
   `start_position`, and `loop_region` for later transition math.

Note step 1's `loop_region` is **stored on the handle, not applied to the
`ma_sound`**. Looping is done by LuteDrop's own crossfade in the poll loop (see
Seamless loops), not by miniaudio's built-in loop flag — a built-in loop would
butt-join the end to the start and click.

### The library preview shares `Voice`, not the orchestrator

The library preview (auditioning one entry in the waveform editor) and a track
both play through a **`Voice`**, so they share all the single-sound machinery:
click-free stop, seamless-loop crossfade, the reaper. What they do *not* share is
the orchestrator — the preview is one sound with no clip sequencing, so
`AudioEngine.library` is a bare `Voice` that `play_library_sound` /
`stop_library_sound` drive directly. It also gets a **scheduled trim-end stop**
(`Voice::play_with_trim_stop`, see below), and its loop restart is *not*
sample-accurately anchored (`Voice::new`, `anchor = LoopAnchor::Immediate`),
matching the pre-refactor preview behavior exactly.

---

## How tracks sequence

A `ManagedTrack` (`orchestrator.rs`) holds one track's live state:

```rust
pub struct ManagedTrack {
    pub transitions: TransitionManager,   // declared BEFORE voice — see Invariants
    pub voice: Voice,
    pub config: Track,
    pub current_clip_index: Option<usize>,
    pub clips_played: usize,
    pub is_paused: bool,
}
```

`orchestrator::start_clip` builds the `PlayParams` for a clip (resolving trim,
volume, loop region, and fades) and plays it on the track's `Voice`. Whether it
uses `voice.play` or `voice.play_with_trim_stop` depends on the transition
(crossfade clips must NOT get a scheduled trim-end stop — see the comment block
in `start_clip`).

Sequencing decisions live in `TransitionManager::update`, which is called once
per tick — taking a `TickInputs` struct bundling the poll-gathered state
(position, clip duration/start, mode, pause/playing/stopped flags, and the
pre-picked next clip index) — and returns a `TransitionAction` telling
`poll_track` what to do:

| `TransitionAction` variant | When it fires | Effect in `poll_track` |
|---|---|---|
| `AdvanceClip { next_clip_index }` | current sound stopped, gapless / (crossfade fallback) | start next clip immediately |
| `StartCrossfade { next_clip_index, duration }` | position reached `clip_dur − effective_xfade` (crossfade mode) | `take_current` the outgoing sound, hand it to `begin_crossfade`, start next clip overlapping |
| `ScheduleGap { next_clip_index, delay_seconds }` | current sound stopped, gap mode | `clear_current`, record a wall-clock start time |
| `StartPendingClip { clip_index }` | a scheduled gap's start time arrived | start the queued clip |
| `CleanupOutgoing` | `outgoing.duration + 0.5 s` elapsed | `stop_quick` + drop the outgoing crossfade handle |
| `None` | nothing to do | if the sound stopped and there's no next clip, `stop_track` |

**Terminating and choosing the next clip** — `pick_next_clip_index`
(`transitions.rs`):

- `Sequence { repeat, .. }`: advance to `index+1`; wrap to 0 if `repeat`, else
  `None` (track ends).
- `Shuffle { repeat, .. }`: a random index that avoids an immediate repeat.
  Without `repeat`, stops once `clips_played >= clip_count` (the orchestrator
  counts plays in `ManagedTrack.clips_played`; clips may repeat within that
  budget — exact-once shuffle is a future feature).
- `Seamless { .. }`: always `None` — the single clip loops itself via the Voice,
  so there is never a "next clip".

**Gap / gapless / crossfade** are the three `TrackTransition` values:

- **Gapless** — start the next clip the instant the current one stops (no
  silence). If it broke you'd hear a gap.
- **Gap { min, max }** — insert a random silence of `min..=max` seconds between
  clips (`gap_delay`, which tolerates inverted/non-finite config so a bad value
  can't panic the poll thread and poison the mutex).
- **Crossfade** — overlap the outgoing and incoming clips by `CROSSFADE_OVERLAP`
  (4.0 s) so one fades out as the next fades in. The outgoing sound is handed to
  `TransitionManager` (via `OutgoingFade`) so it keeps mixing through its fade;
  short clips shrink the window to half their playable length (min 0.1 s) to
  avoid triggering a crossfade at clip start.

---

## Seamless loops

A seamless-mode track (or a looping library preview) plays one clip forever with
no audible seam at the loop point. A naive loop — jump the cursor from `loop_end`
back to `loop_start` — produces a **step discontinuity** (the waveform teleports)
that you hear as a click or pop. LuteDrop avoids that with a **crossfade window**:
a short overlap (the last `LOOP_CROSSFADE_SECS` before `loop_end`) where the
outgoing instance fades out while a fresh instance, started at `loop_start`, fades
in — so the two sum smoothly across the loop point.

The mechanism (engine-clocked ping-pong):

1. When a looping sound starts, the Voice requests its next instance be
   preloaded off-lock (`loop_wants_preload` → the loader thread opens + seeks it
   to `loop_start` without the mutex).
2. When the preload completes (an earlier poll tick, well before the loop
   point), the tick attaches it to the group and — for tracks
   (`LoopAnchor::EngineClock`) — schedules BOTH sides on the engine clock: the
   incoming to start at the `loop_end − LOOP_CROSSFADE_SECS` frame with a
   fade-in, and the outgoing to stop, faded, ending at `loop_end`. The audio
   clock then executes the crossfade regardless of poll timing, so a poll stall
   can no longer cause a seam.
3. A later tick detects the swap completed, parks the (already-faded) outgoing
   in the reaper, promotes the incoming to `current`, and requests the next
   preload.

The library preview (`LoopAnchor::Immediate`) uses the same preloaded instance
but starts it immediately when the window is detected rather than frame-anchored
(preserving its historical behavior with strictly fewer file opens). If the
preload is not ready by the window on either path, the Voice falls back to the
synchronous `do_loop_crossfade` (open + play at the window) and warns once
per path.

**Why the swap check sits above the Playing gate.** `Voice::poll_loop` detects
the staged-instance swap (`self.next_loop.is_some()`) *before* its own
`!matches!(st, Playing) || !should_run` early return, not after — this ordering
is load-bearing, not incidental. The EngineClock swap trigger fires on
`!current.is_playing() || current.at_end() || pos >= loop_end`, which is the
exact logical complement of the `Playing` gate: put the check below the gate
and it becomes provably unreachable. The reason is timing, not logic — the
outgoing's *scheduled* stop (step 2 above) makes it report non-`Playing` on
precisely the tick the swap must run (the "stopped-outgoing tick"). Gate the
check behind `Playing` and that tick instead falls through to
`transitions.update`, which reads the stopped sound as a natural end and stops
the track — an EngineClock seamless track would die one tick after its first
`loop_end` whenever the preload arrived in time (a fast drive breaks; a slow
drive is masked by the synchronous fallback). Pause safety is unaffected: `pause()`
discards any staged instance first, so `next_loop` is `None` whenever a Voice is
paused, and the swap check is skipped.

```rust
pub const LOOP_CROSSFADE_SECS: f64 = 0.12;   // voice.rs
```

**Why 0.12 s?** The check runs on the 50 ms poll tick, so a single delayed tick
can land up to ~50 ms late. The window is sized to at least **two poll periods**
(2.4 × 50 ms) so even a late tick still catches the window before `loop_end` is
overshot and the loop audibly jumps.

**Engine-clock anchoring (tracks only).** For tracks, `anchor = LoopAnchor::EngineClock`
(`Voice::with_loop_anchor`), so the incoming instance is scheduled to start
at an absolute engine frame — `engine.time_frames() + lead` — via
`Sound::schedule_start_frame`. "Sample-accurate" means the start is pinned to an
exact PCM frame on miniaudio's own clock rather than to whenever the poll thread
happened to run, so the overlap lines up precisely. The library preview passes
`LoopAnchor::Immediate` and does not anchor (matching its historical behavior).

**The `LoopRestart` capture.** Because the restart is built from a snapshot taken
at play time (`file_path`, `volume`, `trim_start`/`trim_end`, `loop_region`,
`loop_start_position`), every loop iteration is byte-for-byte the same source and
trim. `Voice::set_volume` also updates the stored `LoopRestart.volume` so a
volume change mid-loop carries into the next iteration — and, if a next-loop
instance is already staged (`next_loop`), propagates to that instance's sound
node and its `loop_incoming_restart.volume` too, so a volume change doesn't
arrive one iteration late for whichever instance is currently pre-picked.

---

## The poll loop

`playback_polling_loop` (`lib.rs`) runs on its own thread:

```rust
loop {
    sleep(50 ms);
    let (states, missing, found) = audio.poll();   // under lock_audio()
    // emit "playback-state-update" per state; emit "master-volume-update"
    // mark newly missing/found library entries
    server::broadcast_tick(&container);
}
```

`AudioEngine::poll` (`mod.rs`), each tick:

0. `drain_completions()` — first thing each tick: pull finished off-lock
   preloads off the loader's completion channel (`try_recv` loop). Each is
   routed by sequence token (`loader::route_completion`); stale ones (target
   gone or superseded) are dropped. Accepted loop instances are handed to their
   Voice (`accept_loop_incoming`); accepted next-clip instances wait in
   `pending` until the transition consumes them.
1. For every real track, `orchestrator::poll_track`, which:
   - `voice.drop_finished()` — **the reaper**: drop any parked fade-out sounds
     that have gone silent (run first, so it happens every tick regardless of
     which branch returns early below).
   - `voice.poll_loop(...)` — now **bookkeeping** for seamless loops: when a
     preloaded next instance has been scheduled (engine-clocked) it detects the
     swap, parks the outgoing, and clears the request flag. Only if the preload
     is not ready by the crossfade window (slow drive) does it fall back to the
     old synchronous restart (with a one-per-path `log::warn!`). The
     swap-detection check runs *before* `poll_loop`'s own
     `!matches!(st, Playing) || !should_run` early return, not after it — see
     "Why the swap check sits above the Playing gate" below.
   - `transitions.update(..., next_clip_index)` — the next index is now
     **pre-picked at clip-start time** and passed in, not computed here.
2. Collect a `TrackPlaybackState` per track.
3. `library.drop_finished()` then `library.poll_loop(engine, true, StopAtEnd::Yes)`
   — the preview *does* stop a non-looping sound at its `trim_end` here.
4. `request_loop_preloads()` and `request_next_clip_preloads()` queue the next
   off-lock opens — for every looping Voice (library + tracks) that wants its
   next loop instance, and for every playing track's pre-picked next clip.
5. Always push a `__library` state (a stopped one if nothing plays) so the
   frontend can tell when the preview ends.

**What it emits.** Two channels:

- **Tauri events** to the desktop webview: `playback-state-update` (one per
  track, incl. `__library`) and `master-volume-update`.
- **`broadcast_tick`** (`server.rs`) to phone remotes over WebSocket. This is
  split in two: a cheap `"playback"` message (positions + master volume) every
  tick, and a full `"state"` structural snapshot **only when the dirty flag is
  set** — `AppStateContainer.take_state_dirty()`. Mutations call
  `mark_state_dirty()`; the tick consumes it. With zero connected remotes,
  `broadcast_tick` returns immediately (no locks, no serialization).

---

## Invariants (read this before touching the audio module)

These are the rules that keep the C object graph and the shared engine state
sound. Breaking one is usually silent — no compile error, a crash or click at
runtime.

**1. Drop order: Sounds → Groups → Engine.** miniaudio sounds and groups
reference the engine; destroying the engine (or a group) while its children are
alive is undefined behavior. Rust drops struct fields **in declaration order**,
so field order *is* teardown order:

- `AudioEngine` (`mod.rs`) declares `tracks` and `library` **before** `engine`.
- `Voice` (`voice.rs`) declares `current` / `fading_out` **before** `group`.
- `ManagedTrack` (`orchestrator.rs`) declares `transitions` **before** `voice` —
  because `transitions` holds the outgoing crossfade sound (`OutgoingFade.handle`),
  which lives in `voice`'s group and must be uninitialized before that group.
  Reordering these fields reintroduces mid-crossfade UB. `ffi.rs` documents the
  contract on the `Engine` type.

**2. Everything runs under the audio mutex, with poison recovery.** The engine
lives in `AppStateContainer.audio: Mutex<AudioEngine>`. All access goes through
`container.lock_audio()`, which recovers from a poisoned mutex (a panic while the
lock was held) instead of propagating the panic — engine state stays structurally
valid across a panic (single-field mutations only), and permanently killing audio
because one poll tick panicked would be worse. The poison is logged. This is also
why the raw pointers can be `unsafe impl Send` (see `ffi.rs`): access is
serialized to one thread at a time by this mutex.

**Exception: the loader thread.** `audio/loader.rs` runs one
background thread that creates and pre-seeks streaming sounds *without* the
audio mutex, using `ffi::EngineRef` (a `Send + Sync` engine-pointer copy limited
to sound creation). This is sound because miniaudio synchronizes node
init/attach/uninit internally with spinlocks (miniaudio.h §7.2) and the engine
outlives the loader thread (`AudioEngine.loader` is joined on drop, before the
`engine` field). Preloaded sounds are created group-less (engine endpoint) and
only become audible after the poll tick re-parents them to a live group and
starts them — under the mutex (`Voice::start_preloaded` / `accept_loop_incoming`).
A group-less, never-started preloaded `Sound` may be dropped on any thread.

**3. DB lock before audio lock.** Actions that touch both take the DB lock first,
then the audio lock (e.g. `set_track_volume` persists to the DB, then
updates the engine). Keep this order everywhere to avoid a lock-ordering deadlock.

**4. Loop points are snapshotted at track-load time.** The `loop_region` a
seamless track uses is computed in `start_clip` from the clip's config when it
starts, and captured into `LoopRestart` at play time. **Editing an entry's
loop/trim markers changes playback only after the track is reloaded** (next
`setup_track` / `update_track_config` + replay), not live for an already-playing
loop.

**5. The reaper contract: call `drop_finished` every tick.** Parked fade-out
sounds are only freed by `Voice::drop_finished`. `poll_track` and
`AudioEngine::poll` both call it first thing every tick. Skip it and stopped
sounds leak (they stay alive in miniaudio's mixer forever). It reaps on the
sound's own engine-clock state (silent → drop now); the wall-clock
`REAP_BACKSTOP` (2 s) grace is only a leak backstop for a stalled device.

---

## How to add X

### A new transition type

1. Add a variant to `TrackTransition` in `src-tauri/src/models.rs` (it derives
   `TS`, so `ts-rs` regenerates the TypeScript twin under `generated/` on build).
2. If it needs a new orchestrator action, add a variant to `TransitionAction`
   (`transitions.rs`).
3. Add a branch in `TransitionManager::update` (`transitions.rs`) that returns
   the action when its trigger condition is met.
4. Handle that action in `orchestrator::poll_track` (`orchestrator.rs`).
5. If it changes how the outgoing sound is retired, mirror the existing
   `begin_crossfade` / `cleanup_outgoing` handoff so the sound isn't dropped
   mid-fade.
6. Expose the option in the track settings UI (`src/components/TrackSettings.vue`).

### A new engine-level operation (e.g. solo)

1. Add the method to `AudioEngine` (`audio/mod.rs`), delegating to
   `orchestrator`/`levels` helpers.
2. Add a `pub fn` in `actions.rs` that locks the engine and calls it.
3. Add a `#[tauri::command]` in `commands.rs` and register it in the
   `generate_handler![...]` list in `lib.rs` (passed to `.invoke_handler(...)`).
4. Add an `invoke(...)` wrapper in `src/composables/useAudio.ts`.
5. If remotes should trigger it, add a case in `handle_remote_command`
   (`server.rs`) and a sender in `src/remote/composables/useRemote.ts`.

### A new preview / playback parameter (e.g. pitch)

1. Add the field to `PlayParams` (`sound.rs`) and apply it in `sound::play`
   before `sound.play(...)` (you may also need a new `ld_sound_*` shim function
   in `vendor/shim.c` + `shim.h` and a `Sound` wrapper method in `ffi.rs`).
2. Thread it through `Voice::play` / `play_inner` if the Voice must remember it
   (e.g. for the `LoopRestart` snapshot).
3. Pass it from `AudioEngine::play_library_sound` and/or
   `orchestrator::start_clip`.
4. Extend the `play_library_sound` command (`commands.rs` / `actions.rs`) and the
   `playLibrarySound` call in `src/composables/useAudio.ts`.

---

## Manual verification: audio engine (live device)

Run the desktop app with a real audio device. `cargo test` covers the pure
logic (seq-routing, backoff decision, `update` index threading, `pick_next_clip_index`);
these cover the device-only behaviors.

1. **Seamless loop over a slow drive.** Put a looping seamless-mode clip on a
   network/removable drive; play it. Expected: no seam or pop at the loop point
   across many iterations, including while other audio commands run (the open is
   off-lock now). Temporarily throttle/disconnect the drive mid-loop: expect a
   single `log::warn!` about a synchronous restart (fallback), not silence or a
   command freeze. Also confirm the pre-seek actually removes the stall rather
   than deferring it: with an opus/mp3 source and a non-zero `loop_start`, the
   first audio of each incoming instance must arrive on time with no dropout at
   the swap — a dropout there would mean the decode-skip I/O was deferred to
   the audio thread's first read instead of happening on the loader thread.
   Then pause the track while a staged next instance exists (pause shortly
   after a loop point, when the preload has landed): expect full silence during
   the pause (no ghost instance starting underneath) and a clean, seamless
   resume.
2. **Transition under load.** A Sequence/Shuffle track with crossfade or gapless
   transitions, clips on a slow drive. Expected: transitions stay on time (the
   next clip was preloaded); the crossfade seam is inaudible. Confirm shuffle
   still avoids immediate repeats and the distribution looks unchanged (the
   choice now commits one clip early — inaudible).
3. **Teardown mid-preload.** Start playback that triggers a preload, then quit
   the app while a preload is in flight (easiest on a slow drive). Expected:
   clean exit, no crash — quit may pause up to the OS file-open timeout if a
   load is blocked on a dead drive (acceptable; the join is required for
   soundness — the loader must not outlive the engine, so do NOT detach the
   thread). Any group-less pending sound drops before the engine.
4. **Failed-start backoff.** Point a clip at a missing/disconnected file and play
   the track. Expected: one failed start, then no retry for ~2 s (no 20 Hz
   open storm); the file's missing state is still reported to the UI.
