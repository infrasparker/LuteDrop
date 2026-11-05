# Orientation: a guided tour of LuteDrop

Orientation guide for this codebase: application structure, two end-to-end flows,
and a Rust primer with TypeScript comparisons drawn from this repository. Assumes
TypeScript familiarity; assumes no Rust.

For the precise reference (every function, the invariants, the volume model), see
[`docs/engine-architecture.md`](./engine-architecture.md). Read this first.

---

## The 10,000-ft map

LuteDrop is a **Tauri** app. Think of Tauri as "Electron, but the backend is Rust
instead of Node." Three things are running at once:

1. **The Vue webview** (`src/`) — the desktop UI you click. Ordinary Vue 3 +
   TypeScript + Vuetify, running in a webview window. It cannot make sound or
   touch files directly; it asks the backend to.
2. **The Rust backend** (`src-tauri/`) — owns the database, the audio engine, and
   the filesystem. The UI reaches it by `invoke("some_command", args)`, which
   calls a `#[tauri::command]` function. This is the **IPC** boundary (inter-process
   communication: the message-passing hop between the webview and the native
   backend).
3. **The phone-remote server** (`src-tauri/src/server.rs`) — a small `axum` web
   server the backend starts on its own thread (port 3333). It serves a *second*
   Vue app (`src/remote/`, built to `dist-remote/`) to your phone's browser and
   talks to it over a WebSocket. So a phone on the same Wi-Fi can drive playback
   without the desktop UI.

Top-level directories:

| Path | What it is |
|---|---|
| `src/` | the main desktop Vue app (components, views, stores, composables) |
| `src/remote/` | the separate, smaller Vue app served to phones |
| `src-tauri/src/` | the Rust backend |
| `src-tauri/src/audio/` | the audio engine (the hard part — see engine-architecture.md) |
| `src-tauri/vendor/` | vendored C: miniaudio + the opus backend + our `shim.c` |
| `src-tauri/src/autotag/` | the auto-tagging worker (see below) |
| `tagger/` | the separate analyzer service used by auto-tagging, with its own README |
| `generated/` | TypeScript types auto-generated from Rust structs by `ts-rs` |
| `docs/` | you are here |

Auto-tagging adds a fourth, optional piece. A background worker thread in
`src-tauri/src/autotag/` wakes on requests (new imports, "Analyze library", or
a tag change that calls for re-scoring), sends audio paths to the analyzer
service over HTTP, and sends the current tagged examples along with each
scoring request. The service keeps no tag state, only an embedding cache. The
worker writes the resulting scores and auto tags to the database, emits
`autotag-progress` while it works and `autotag-complete` when it finishes, and
the Vue side reloads library state on that event.

`src-tauri/src/autotag/vibe.rs` holds the vibe commands (`similar_entries`,
`search_vibe`, `vibe_map`). Each one maps music entries to their content
hashes, asks the analyzer's `/similar`, `/search` or `/map` endpoint, and maps
the returned hashes back to entry ids; entries that share a hash all appear in
the result. Scry (`src/views/VibeView.vue`) combines the text search
and seeds with `VibeResults.vue` (ranked list with Role and Pace filters),
`VibeMap.vue` (canvas scatter with lasso selection) and
`CreatePlaylistDialog.vue` (selection to a new shuffle track or an existing
track). `RoleControl.vue` is the Combat / Non-combat control used by the entry
editor; the bulk bar has its own menu.

The split to keep in your head: **Vue decides *what* the user wants; Rust decides
*how* to make it happen and actually does it.**

---

## Follow one click: pressing play on a track

You click the play button on a `TrackControl`. Here is the whole path:

1. **Vue** — `src/components/TrackControl.vue`: the button's `@click` calls
   `playTrack()` (line ~72), which calls `audio.playTrack(props.track.id)` from
   the `useAudio()` composable.
2. **Composable** — `src/composables/useAudio.ts`: `playTrack` is just
   `await invoke('play_track', { trackId })` wrapped in a try/catch that shows a
   toast on error. This is the IPC hop — the string `'play_track'` names a Rust
   command, and `{ trackId }` is serialized to JSON and sent across.
   *(Data-mutating commands instead go through `appStore.command(...)` in
   `src/stores/appStore.ts`, which invokes and then re-hydrates the whole app
   state from the returned `AppState`. Playback commands don't return state, so
   they skip that.)*
3. **Command** — `src-tauri/src/commands.rs`:
   `#[tauri::command] pub fn play_track(state, track_id)` → `actions::play_track_audio`.
   The `state: State<Arc<AppStateContainer>>` argument is Tauri handing you the
   shared app state automatically.
4. **Action** — `src-tauri/src/actions.rs` `play_track_audio`: loads the track's
   config from the DB, checks the clip files exist, then
   `let mut audio = container.lock_audio();` (takes the engine mutex) and, if this
   track isn't set up yet, `audio.setup_track(&config)` before `audio.play_track(track_id)`.
5. **Engine** — `src-tauri/src/audio/mod.rs` `AudioEngine::play_track` →
   `orchestrator::play_track` → `start_clip`: builds a `PlayParams` for the first
   clip and calls the track's `Voice`.
6. **Voice → sound → shim** — `voice.rs` `Voice::play_with_trim_stop` →
   `sound::play` (`sound.rs`) → `engine.create_sound(...)` + `sound.play(...)`
   (`ffi.rs`) → `ld_sound_create` / `ld_sound_play` in `vendor/shim.c` → miniaudio
   starts streaming the file on its audio thread. Sound comes out.

The phone remote takes the same path from step 3 on: `useRemote.ts` sends
`{ command: 'play_track', trackId }` over the WebSocket, and `server.rs`
`handle_remote_command` calls the very same `actions::play_track_audio`.

---

## Follow one tick: the 50 ms poll loop

miniaudio plays sounds on its own thread, but it won't decide "this clip ended,
start the next one" or "we're near the loop point, crossfade now" — that's
LuteDrop's job. So a background thread (`playback_polling_loop` in `lib.rs`) wakes
every 50 ms and nudges the engine. Without it, a track would play its first clip
and then just stop; loops would never restart; the UI's playhead would freeze.

Each tick (see `AudioEngine::poll`):

1. **Reap** finished fade-outs (drop sounds that have gone silent — else they leak).
2. **Loop check**: if a sound is near its loop point, do the seamless crossfade.
3. **Transitions**: ask each track's `TransitionManager` "anything to do?" and act
   on the answer (advance clip, start a gap, clean up a crossfade…).
4. **Report**: build a `TrackPlaybackState` per track (is it playing? where's the
   playhead?) and emit it to the UI (`playback-state-update`) and, if any phones
   are connected, broadcast it over the WebSocket.

So the loop is both the **clock** that drives sequencing and the **heartbeat** that
keeps the UI's playhead and the phone remotes in sync. 20 times a second is fast
enough to feel instant and slow enough to be cheap.

---

## Rust for TypeScript readers (keyed to this repo)

Covers the constructs used in these files, each next to its TS equivalent, rather
than a general Rust tutorial.

### `Result<T, E>` and `?` — errors as values, not exceptions

Rust has no `throw`/`try`/`catch`. A function that can fail returns
`Result<T, E>` — either `Ok(value)` or `Err(error)`. The `?` operator is the
punchline: "if this is `Err`, return it from the whole function right now;
otherwise unwrap the `Ok`."

```rust
// actions.rs
pub fn play_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;  // ? = bail out on Err
    ...
    audio.play_track(track_id)   // returns Result — becomes this fn's result
}
```

`Result<(), String>` means "on success I return nothing (`()`, the unit type, like
`void`); on failure a `String` message." The TS you'd write is a function that
either returns or `throw`s a string; `?` is early-return-on-throw made explicit.

### `Option<T>` — no `null`, no `undefined`

Rust has no `null`. A value that might be absent is `Option<T>`: either
`Some(value)` or `None`. You can't accidentally use a `None` as if it were
present — the compiler makes you handle both.

```rust
// orchestrator.rs
pub current_clip_index: Option<usize>,   // maybe an index, maybe nothing playing
```

`.map(...)`, `.unwrap_or(default)`, and `if let Some(x) = opt { ... }` are how you
work with it — the same instincts as `x?.foo` and `x ?? default` in TS.

### Ownership & `Drop` — and why field order matters here

Every value has exactly one owner. When the owner goes out of scope, the value is
**dropped** (freed) automatically — Rust's version of a destructor, but
deterministic (no GC). You can hook it with the `Drop` trait; `ffi.rs` does, to
tell miniaudio to free each object:

```rust
// ffi.rs
impl Drop for Sound {
    fn drop(&mut self) {
        unsafe { ld_sound_destroy(self.ptr) }
    }
}
```

The load-bearing detail for *this* codebase: **struct fields are dropped in
declaration order.** miniaudio requires sounds be freed before their group before
the engine, so the field order in `AudioEngine`, `Voice`, and `ManagedTrack` *is*
the teardown order. Reordering them is not a style choice — it's undefined
behavior at shutdown. (This is the single most surprising thing in the audio
module; it's called out as Invariant #1 in engine-architecture.md.)

### `&` and `&mut` — borrows

In JS everything is a shared reference and you never think about it. Rust makes you
say whether you're borrowing a value to *read* (`&T`, any number at once) or to
*mutate* (`&mut T`, exclusive — only one at a time). This is checked at compile
time and is why you see `&self` vs `&mut self` on methods:

```rust
// mod.rs
pub fn get_master_volume(&self) -> f64 { self.master_volume }        // read-only borrow
pub fn set_master_volume(&mut self, volume: f64) { ... }             // exclusive mutable borrow
```

If it reads, it takes `&`; if it changes something, it takes `&mut`. The
orchestrator functions take `&mut HashMap<String, ManagedTrack>` because they
mutate track state.

### `Arc<Mutex<T>>` — sharing across threads

Single-threaded JS never needs this. LuteDrop has several threads (the UI's
command handlers, the poll loop, the web server), and they all share one
`AudioEngine`. Two wrappers make that safe:

- `Arc<T>` — an atomically reference-counted shared pointer (many owners; freed
  when the last drops). Like a shared value you can hand to multiple threads.
- `Mutex<T>` — a lock: only one thread holds the inside at a time.

```rust
// state.rs
pub struct AppStateContainer {
    pub audio: Mutex<AudioEngine>,   // the engine, behind a lock
    ...
}
```

Everyone calls `container.lock_audio()` to get exclusive access. That serialization
is also what makes the raw C pointers safe to send between threads (see the
`unsafe impl Send` note below).

### `match` and `enum` — discriminated unions

Rust `enum`s are tagged unions that can carry data per variant — exactly TS
discriminated unions. `match` is `switch`, but the compiler forces you to cover
every case.

```rust
// models.rs — and ts-rs generates the TypeScript twin of this into generated/
pub enum TrackMode {
    Seamless { loop_start: f64, loop_end: f64 },
    Sequence { repeat: bool, transition: TrackTransition },
    Shuffle  { repeat: bool, transition: TrackTransition },
}
```

```rust
// transitions.rs — matching, with data destructured out
match transition {
    TrackTransition::Gapless => TransitionAction::AdvanceClip { next_clip_index: idx },
    TrackTransition::Gap { min, max } => TransitionAction::ScheduleGap { ... },
    TrackTransition::Crossfade => TransitionAction::AdvanceClip { next_clip_index: idx },
}
```

Because `models.rs` derives `TS` (`#[ts(export, ...)]`), the build step writes a
matching TypeScript type into `generated/`, so the frontend's `TrackMode` and the
Rust `TrackMode` can't drift apart.

### Closures and iterators — analogous to `map`/`filter`

Rust iterators work like array methods, but lazily (nothing runs until you
`.collect()` or loop). Closures use `|args|` instead of `(args) =>`:

```rust
// mod.rs
let track_ids: Vec<String> = self.tracks.keys()
    .filter(|id| id.as_str() != LIBRARY_TRACK_ID)
    .cloned()
    .collect();
```

Read that exactly like `Object.keys(...).filter(id => id !== '__library')`.

### `unsafe` and FFI — what the `unsafe` blocks actually promise

`unsafe` does **not** mean "buggy" — it means "the compiler can't verify this;
*I* am promising it's correct." It appears in one place: `ffi.rs`, wrapping calls
into the C shim. The compiler can't know a raw C pointer is valid, so you assert
it:

```rust
// ffi.rs
pub fn set_master_volume(&self, linear: f32) {
    unsafe { ld_engine_set_volume(self.0, linear) }
}
```

The promises being made are: (a) these pointers came from the shim and are still
valid, and (b) they're never used from two threads at once — guaranteed because
every `Engine`/`Group`/`Sound` lives behind the `AudioEngine` mutex. That second
promise is exactly what `unsafe impl Send for Engine {}` (etc.) asserts. The rest
of the codebase is safe Rust; `unsafe` is quarantined to this boundary file.

---

## Where things get hard, and why

The densest code in the repository, and the technical reason each area is dense:

- **`voice.rs` timing** — click-free audio is all about *when* a sound is stopped
  vs. *when* it's freed. A sound dropped mid-fade hard-cuts and pops, so stopped
  sounds are parked and reaped a tick later. Following it means holding "audio
  thread time," "poll thread time," and, for seamless loops, "engine clock time"
  (the ping-pong swap is scheduled on miniaudio's own clock, not the poll tick)
  in your head at once.
- **The `ffi.rs` / `shim.c` boundary** — two languages, manual memory, and the
  drop-order contract. A mistake here doesn't fail to compile; it corrupts memory
  or crashes at shutdown.
- **`transitions.rs` state** — a small state machine (crossfade started? gap
  pending? outgoing fading?) driven by a polling loop that can tick late. The
  edge cases (short clips, a delayed tick landing past the loop point on the
  slow-drive fallback path) are where the bugs live, and they're timing-dependent
  so they're hard to reproduce.

Everything else in the app — the DB layer, the Vue components, the command
plumbing — is ordinary code. The audio module is where the intrinsic difficulty
is concentrated.

---

## Glossary

- **Crossfade** — overlapping two sounds so one fades down as the other fades up,
  with no silence or click between them. Used between clips (4 s,
  `CROSSFADE_OVERLAP`) and at loop points (0.12 s, `LOOP_CROSSFADE_SECS`).
- **De-click fade** — a very short fade (a few ms) applied when *stopping* a
  sound. Cutting audio at a non-zero sample is a step discontinuity you hear as a
  click; the tiny fade ramps to zero so the stop is silent.
- **Sub-mix group** — a miniaudio node that several sounds route through, so you
  can set one volume for the whole group. Each track and the library preview owns
  one (a `Voice`'s `group`); it's the "track" level of the volume model.
- **Sample-accurate scheduling** — pinning a sound's start or stop to an exact
  PCM frame on miniaudio's own clock, rather than "whenever the poll thread
  next runs": starts via `schedule_start_frame`, stops via
  `schedule_stop_secs`. Used so a loop restart lines up precisely.
- **WAL** — Write-Ahead Logging, a SQLite mode (`PRAGMA journal_mode=WAL` in
  `database/mod.rs`) that lets a reader (e.g. the backup thread) run while the app
  writes, instead of blocking each other.
- **N+1** — the query anti-pattern of doing one query, then one more per row it
  returned. `database/mod.rs` deliberately loads the hierarchy with four flat
  queries (one per level) instead of an N+1 cascade.
- **IPC** — inter-process communication: the boundary between the Vue webview and
  the Rust backend. Crossed by `invoke("command", args)` from the frontend and
  Tauri `emit(...)` events back.
