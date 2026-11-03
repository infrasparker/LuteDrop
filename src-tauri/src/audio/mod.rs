pub mod ffi;
pub mod levels;
pub mod loader;
pub mod orchestrator;
pub mod sound;
pub mod transitions;
pub mod util;
pub mod voice;

use std::collections::HashMap;
use std::time::Duration;

use crate::models::{Track, TrackPlaybackState};
use levels::MuteState;
use loader::{LoadRequest, Loader, PreloadKind, PreloadTarget};
use orchestrator::ManagedTrack;
use sound::{PlayParams, PlaybackState};
use transitions::retry_suppressed;
use voice::{StopAtEnd, Voice};

const LIBRARY_TRACK_ID: &str = "__library";

/// Interval between audio-engine poll ticks: drives the transition state
/// machine (`transitions.rs` assumes this cadence when sizing its cleanup
/// grace) and the seamless-loop crossfade window (`voice.rs`'s
/// `LOOP_CROSSFADE_SECS` is sized against this). Also the sleep in `lib.rs`'s
/// `playback_polling_loop`.
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub struct AudioEngine {
    // Fields are declared so playing sounds / groups drop before the engine.
    // Rust drops struct fields in declaration order; miniaudio requires the
    // object graph torn down innermost-first (Sounds → Groups → Engine).
    // Threads that hold the engine pointer (`loader`, `broadcaster`'s encoder
    // thread) are likewise declared before `engine` so they are joined first.
    tracks: HashMap<String, ManagedTrack>,
    /// Single-sound playback for the library preview: owns its sub-mix group, the
    /// current sound, the de-click fade-out reaper, and the seamless-loop
    /// crossfade. Declared before `engine` so its group/sounds drop first.
    library: Voice,
    /// In-flight and ready preloads, one per target (D5). Keyed by target
    /// String (track id / "__library") — a deliberate deviation from D5's
    /// `HashMap<u64, _>` shorthand, since track ids are String UUIDs; the seq
    /// token lives in the value. Holds group-less preloaded sounds; declared
    /// before `engine` so they uninit before it.
    pending: HashMap<String, PreloadTarget>,
    /// Monotonic sequence source; never resets, so a stale completion's seq can
    /// never collide with a live entry's.
    next_seq: u64,
    /// The loader thread; its Drop joins it before `engine` is dropped.
    loader: Loader,
    master_volume: f64,
    mute_state: MuteState,
    /// The live broadcast, when it is on. Declared before `engine`: its
    /// encoder thread reads the engine's tap through a raw pointer, and
    /// dropping the `Broadcaster` joins that thread, so it must finish
    /// before the engine is destroyed.
    broadcaster: Option<crate::broadcast::Broadcaster>,
    /// Why the broadcast could not be turned back on at launch, kept so the
    /// Broadcast dialog can show it later (the launch itself has no dialog
    /// open to show it in). Set by [`AudioEngine::restore_broadcast`] and
    /// cleared whenever a broadcast starts successfully.
    last_broadcast_error: Option<String>,
    /// Owns the whole audio graph; declared LAST so it outlives all groups/
    /// sounds AND the loader thread (see ffi::EngineRef safety (b)).
    engine: ffi::Engine,
}

impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let engine = ffi::Engine::new()?;
        // Library preview does NOT sample-accurately anchor its loop restart
        // (matches the pre-refactor library behavior).
        let library = Voice::new(&engine)?;
        let loader = Loader::spawn(engine.engine_ref());
        Ok(Self {
            tracks: HashMap::new(),
            library,
            pending: HashMap::new(),
            next_seq: 0,
            loader,
            master_volume: 1.0,
            mute_state: MuteState::new(),
            broadcaster: None,
            last_broadcast_error: None,
            engine,
        })
    }

    // --- Broadcast ---

    /// Start the live broadcast on `port`, reachable at `/listen/{key}`.
    /// Fails if a broadcast is already running, or if the encoder or the
    /// listener server cannot start (e.g. the port is busy); after a failure
    /// nothing is left running.
    pub fn start_broadcast(&mut self, port: u16, key: String) -> Result<(), String> {
        if self.broadcaster.is_some() {
            return Err("Broadcast is already running".into());
        }
        let encoder = self.engine.create_encoder(128_000)?;
        self.broadcaster = Some(crate::broadcast::Broadcaster::start(encoder, port, key)?);
        self.last_broadcast_error = None;
        Ok(())
    }

    /// Like [`start_broadcast`](Self::start_broadcast), for turning the
    /// stream back on at launch: if it fails, the reason is also kept (see
    /// [`last_broadcast_error`](Self::last_broadcast_error)) so the dialog can
    /// show it whenever it is opened.
    pub fn restore_broadcast(&mut self, port: u16, key: String) -> Result<(), String> {
        let result = self.start_broadcast(port, key);
        if let Err(e) = &result {
            self.last_broadcast_error = Some(e.clone());
        }
        result
    }

    /// Why the launch-time restore failed, if it did and no broadcast has
    /// started successfully since.
    pub fn last_broadcast_error(&self) -> Option<String> {
        self.last_broadcast_error.clone()
    }

    /// Stop the live broadcast, if it is running. This joins the encoder and
    /// listener-server threads while the caller holds the audio lock. That is
    /// safe: neither thread ever takes the audio lock, so the join cannot
    /// wait on us.
    pub fn stop_broadcast(&mut self) {
        if let Some(b) = self.broadcaster.take() {
            b.stop();
        }
    }

    /// How many listeners are connected, or `None` when the broadcast is off.
    pub fn broadcast_listeners(&self) -> Option<usize> {
        self.broadcaster.as_ref().map(|b| b.listener_count())
    }

    /// Tests only: the raw engine, so a test can switch the broadcast tap on
    /// and read it without starting the whole broadcast.
    #[cfg(test)]
    pub(crate) fn engine_for_test(&self) -> &ffi::Engine {
        &self.engine
    }

    // --- Track operations ---

    pub fn setup_track(&mut self, config: &Track) -> Result<(), String> {
        orchestrator::setup_track(&self.engine, &mut self.tracks, &self.mute_state, config)
    }

    pub fn update_track_config(&mut self, config: &Track) {
        self.invalidate_preload(&config.id);
        orchestrator::update_track_config(&mut self.tracks, config);
    }

    pub fn has_track(&self, track_id: &str) -> bool {
        orchestrator::has_track(&self.tracks, track_id)
    }

    pub fn play_track(&mut self, track_id: &str) -> Result<(), String> {
        self.invalidate_preload(track_id);
        self.stop_library_sound();
        orchestrator::play_track(&mut self.tracks, &self.engine, track_id, &self.mute_state)
    }

    pub fn play_track_clip(&mut self, track_id: &str, clip_id: &str) -> Result<(), String> {
        self.invalidate_preload(track_id);
        self.stop_library_sound();
        orchestrator::play_track_clip(&mut self.tracks, &self.engine, track_id, clip_id, &self.mute_state)
    }

    pub fn play_track_from(&mut self, track_id: &str, position: f64) -> Result<(), String> {
        self.invalidate_preload(track_id);
        orchestrator::play_track_from(&mut self.tracks, &self.engine, track_id, position, &self.mute_state)
    }

    pub fn pause_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::pause_track(&mut self.tracks, track_id)
    }

    pub fn resume_track(&mut self, track_id: &str) -> Result<(), String> {
        orchestrator::resume_track(&mut self.tracks, track_id)
    }

    pub fn stop_track(&mut self, track_id: &str) -> Result<(), String> {
        self.invalidate_preload(track_id);
        orchestrator::stop_track(&mut self.tracks, track_id)
    }

    pub fn stop_all(&mut self) {
        let ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in ids {
            self.invalidate_preload(&id);
        }
        orchestrator::stop_all(&mut self.tracks);
        self.stop_library_sound();
    }

    pub fn pause_all(&mut self) {
        orchestrator::pause_all(&mut self.tracks);
        self.library.pause();
    }

    pub fn resume_all(&mut self) {
        orchestrator::resume_all(&mut self.tracks);
        self.library.resume();
    }

    // --- Volume ---
    //
    // Native miniaudio hierarchy: master = engine node, track = group node,
    // clip = sound node. miniaudio multiplies them automatically, so there is
    // no master×track×clip recompute / per-sound re-apply loop anymore.

    pub fn get_master_volume(&self) -> f64 {
        self.master_volume
    }

    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        if !self.mute_state.master {
            self.engine.set_master_volume(self.master_volume as f32);
        }
    }

    pub fn set_track_volume(&mut self, track_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_track_volume(&mut self.tracks, track_id, volume, &self.mute_state);
        Ok(())
    }

    pub fn set_clip_volume(&mut self, clip_id: &str, volume: f64) -> Result<(), String> {
        orchestrator::set_clip_volume(&mut self.tracks, clip_id, volume, &self.mute_state);
        Ok(())
    }

    pub fn set_library_volume(&mut self, volume: f64) {
        self.library.set_volume(volume.clamp(0.0, 1.0) as f32);
    }

    // --- Mute ---
    //
    // Mute sets the affected node's volume to 0; unmute restores the stored
    // (intended) level. MuteState tracks intent for bookkeeping / snapshots.

    pub fn toggle_master_mute(&mut self) -> bool {
        self.mute_state.master = !self.mute_state.master;
        let vol = if self.mute_state.master { 0.0 } else { self.master_volume as f32 };
        self.engine.set_master_volume(vol);
        self.mute_state.master
    }

    pub fn toggle_track_mute(&mut self, track_id: &str) -> bool {
        let new_state = !self.mute_state.tracks.get(track_id).copied().unwrap_or(false);
        self.mute_state.set_track_muted(track_id, new_state);
        orchestrator::refresh_track_group_volume(&mut self.tracks, track_id, &self.mute_state);
        new_state
    }

    pub fn toggle_clip_mute(&mut self, clip_id: &str) -> bool {
        let new_state = !self.mute_state.clips.get(clip_id).copied().unwrap_or(false);
        self.mute_state.set_clip_muted(clip_id, new_state);
        // Re-apply the owning track's currently-playing sound volume.
        let track_id = self.tracks.iter()
            .find(|(_, t)| t.config.clips.iter().any(|c| c.id == clip_id))
            .map(|(id, _)| id.clone());
        if let Some(tid) = track_id {
            // set_clip_volume with the existing clip volume re-applies the sound node.
            let cur = self.tracks.get(&tid)
                .and_then(|t| t.config.clips.iter().find(|c| c.id == clip_id))
                .map(|c| c.volume)
                .unwrap_or(1.0);
            orchestrator::set_clip_volume(&mut self.tracks, clip_id, cur, &self.mute_state);
        }
        new_state
    }

    pub fn toggle_comp_mute(&mut self, track_ids: &[String]) -> bool {
        let all_muted = track_ids.iter().all(|id| {
            self.mute_state.tracks.get(id).copied().unwrap_or(false)
        });
        let new_state = !all_muted;
        for id in track_ids {
            self.mute_state.set_track_muted(id, new_state);
            orchestrator::refresh_track_group_volume(&mut self.tracks, id, &self.mute_state);
        }
        new_state
    }

    pub fn get_mute_snapshot(&self) -> (bool, std::collections::HashMap<String, bool>, std::collections::HashMap<String, bool>) {
        (self.mute_state.master, self.mute_state.tracks.clone(), self.mute_state.clips.clone())
    }

    // --- Library playback ---

    pub fn play_library_sound(&mut self, params: PlayParams) -> Result<(), String> {
        // A SEEK is `stop_library_sound` + `play_library_sound`; the de-click stop
        // (below) keeps the outgoing sound mixing through its fade so the
        // replacement doesn't click.
        self.stop_library_sound();
        // Scheduled de-clicked stop at trim_end — replaces the old late
        // poll-check hard cut. The library preview never crossfades and
        // `poll_loop(stop_at_end=StopAtEnd::Yes)` still detects the resulting
        // Stopped state to finish. For a looping preview `play_with_trim_stop`
        // no-ops the schedule (loop_region is Some).
        self.library.play_with_trim_stop(&self.engine, params)
    }

    pub fn stop_library_sound(&mut self) {
        self.invalidate_preload(LIBRARY_TRACK_ID);
        // Use the sound's own fade_out if it has one, else a small de-click
        // default. The Voice parks the outgoing sound until the fade finishes.
        self.library.stop(None);
    }

    // --- State queries ---

    /// Returns current playback state of all tracks without driving transitions.
    /// Used by server.rs for snapshot building.
    pub fn get_all_playback_states(&self) -> Vec<TrackPlaybackState> {
        let mut states: Vec<TrackPlaybackState> = self.tracks.values()
            .filter(|t| t.config.id != LIBRARY_TRACK_ID)
            .map(orchestrator::get_playback_state)
            .collect();

        if self.library.has_current() {
            let st = self.library.state();
            states.push(TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: matches!(st, PlaybackState::Playing),
                is_paused: matches!(st, PlaybackState::Paused),
                current_clip_id: None,
                current_time: self.library.position(),
            });
        }

        states
    }

    // --- Polling ---

    pub fn poll(&mut self) -> (Vec<TrackPlaybackState>, Vec<String>, Vec<String>) {
        // Off-lock preloads that finished since the last tick land here first,
        // so this tick's loop/transition handoffs can use them.
        self.drain_completions();

        let mut missing_entries: Vec<String> = Vec::new();
        let mut found_entries: Vec<String> = Vec::new();
        let track_ids: Vec<String> = self.tracks.keys()
            .filter(|id| id.as_str() != LIBRARY_TRACK_ID)
            .cloned()
            .collect();

        for id in &track_ids {
            orchestrator::poll_track(
                &mut self.tracks,
                &self.engine,
                id,
                &self.mute_state,
                &mut self.pending,
                &mut missing_entries,
                &mut found_entries,
            );
        }

        let mut states: Vec<TrackPlaybackState> = self.tracks.values()
            .filter(|t| t.config.id != LIBRARY_TRACK_ID)
            .map(orchestrator::get_playback_state)
            .collect();

        // Drop finished held sounds, then drive the library's seamless-loop
        // crossfade / stop-at-trim-end. The Voice holds the outgoing loop
        // instance so it keeps mixing through its fade (no mid-fade drop / pop).
        self.library.drop_finished();
        self.library.poll_loop(&self.engine, true, StopAtEnd::Yes);

        // Ask each looping Voice for its next loop instance (D6); the loader
        // opens it off-lock and a later tick's drain hands it back.
        self.request_loop_preloads();

        // Queue an off-lock preload of each track's pre-picked next clip (D7).
        self.request_next_clip_preloads();

        // Always emit library sound state so frontend can detect when it stops.
        let lib_state = if self.library.has_current() {
            let st = self.library.state();
            TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: matches!(st, PlaybackState::Playing),
                is_paused: matches!(st, PlaybackState::Paused),
                current_clip_id: None,
                current_time: self.library.position(),
            }
        } else {
            TrackPlaybackState {
                track_id: LIBRARY_TRACK_ID.to_string(),
                is_playing: false,
                is_paused: false,
                current_clip_id: None,
                current_time: 0.0,
            }
        };
        states.push(lib_state);

        (states, missing_entries, found_entries)
    }

    // --- Preload plumbing ---

    /// Queue a preload for `target`, recording it in `pending` with a fresh seq.
    /// Overwrites any existing entry for the target (superseding it — the old
    /// entry's ready handle, if any, drops here, safe per the EngineRef contract).
    fn request_preload(
        &mut self,
        target: String,
        kind: PreloadKind,
        clip_index: Option<usize>,
        params: PlayParams,
    ) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.pending.insert(
            target.clone(),
            PreloadTarget { seq, kind, clip_index, params: params.clone(), ready: None },
        );
        self.loader.request(LoadRequest { target, seq, params });
    }

    /// Ask each looping Voice (library + tracks) whether it wants its next loop
    /// instance preloaded, and queue the requests (D6).
    fn request_loop_preloads(&mut self) {
        if !self.pending.contains_key(LIBRARY_TRACK_ID) {
            if let Some(params) = self.library.loop_wants_preload() {
                self.request_preload(LIBRARY_TRACK_ID.to_string(), PreloadKind::LoopInstance, None, params);
            }
        }
        let ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in ids {
            if self.pending.contains_key(&id) {
                continue;
            }
            if let Some(params) = self.tracks.get_mut(&id).and_then(|t| t.voice.loop_wants_preload()) {
                self.request_preload(id, PreloadKind::LoopInstance, None, params);
            }
        }
    }

    /// Queue an off-lock preload of each track's pre-picked next clip (D7), if
    /// not already pending/ready for that exact index.
    fn request_next_clip_preloads(&mut self) {
        let ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in ids {
            let want = {
                let track = match self.tracks.get(&id) { Some(t) => t, None => continue };
                // Only preload for a track that is actually playing something:
                // a stopped/idle track (no current sound) must not keep opening
                // files. (A gap-transition wait has no current sound either —
                // its pending clip was already picked and, if preloaded before
                // the gap began, sits ready; do not re-request during the gap.)
                if !track.voice.has_current() {
                    continue;
                }
                // I-1: a recent preload failure for this track suppresses
                // re-requesting for START_RETRY_BACKOFF, mirroring the D8 gate
                // on transition-driven clip starts. Without this, `drain_completions`'
                // Err arm removes the pending entry every failed attempt, and this
                // function re-requests it again next tick (~20 Hz retry storm for
                // the whole current clip).
                if retry_suppressed(track.last_preload_failure, std::time::Instant::now()) {
                    continue;
                }
                let already = self
                    .pending
                    .get(&id)
                    .map(|p| p.clip_index == track.next_clip_index)
                    .unwrap_or(false);
                match track.next_clip_index {
                    Some(idx) if !already => orchestrator::preload_params_for(track, idx, &self.mute_state).map(|p| (idx, p)),
                    _ => None,
                }
            };
            if let Some((idx, params)) = want {
                self.request_preload(id, PreloadKind::NextClip, Some(idx), params);
            }
        }
    }

    /// Discard any pending/ready preload for `target` (D7 invalidation).
    /// Dropping the entry forgets its seq; the next request gets a fresh
    /// monotonic seq, so an in-flight completion for the old seq routes as
    /// stale. The ready handle (if any) drops here.
    fn invalidate_preload(&mut self, target: &str) {
        self.pending.remove(target);
    }

    /// Drain the loader completion channel (called first thing in `poll`, under
    /// the mutex). Accepted LoopInstance handles are handed to their Voice
    /// immediately; accepted NextClip handles wait in `pending.ready` until the
    /// transition consumes them.
    fn drain_completions(&mut self) {
        while let Ok(done) = self.loader.rx.try_recv() {
            let pending_seq = self.pending.get(&done.target).map(|p| p.seq);
            if loader::route_completion(pending_seq, done.seq) == loader::Routing::DropStale {
                continue; // handle in `done.result` falls out of scope here
            }
            let handle = match done.result {
                Ok(h) => h,
                Err(e) => {
                    // Open failed off-lock; forget it. The loop / transition
                    // fallback will retry synchronously (and log) when it fires.
                    log::warn!("Preload failed for '{}': {}", done.target, e);
                    // I-1: stamp the owning track so `request_next_clip_preloads`
                    // backs off instead of re-requesting every poll tick (~20 Hz)
                    // for the rest of the current clip. Only NextClip targets are
                    // tracks (LoopInstance failures are handled by the loop's own
                    // synchronous fallback in `poll_loop`, not this gate).
                    if matches!(self.pending.get(&done.target).map(|p| p.kind), Some(PreloadKind::NextClip)) {
                        if let Some(t) = self.tracks.get_mut(&done.target) {
                            t.last_preload_failure = Some(std::time::Instant::now());
                        }
                    }
                    self.pending.remove(&done.target);
                    continue;
                }
            };
            match self.pending.get(&done.target).map(|p| p.kind) {
                Some(PreloadKind::LoopInstance) => {
                    // Never hand a loop instance to a PAUSED voice: an
                    // EngineClock-scheduled incoming is live the moment it is
                    // accepted (the engine clock keeps advancing while paused),
                    // so it would fire audibly under the pause. Skip and drop
                    // the entry; the voice re-requests on resume
                    // (`loop_wants_preload` gates on Playing). The handle here
                    // is still group-less and never-started, so a bare drop is
                    // safe (EngineRef contract).
                    let paused = if done.target == LIBRARY_TRACK_ID {
                        matches!(self.library.state(), PlaybackState::Paused)
                    } else {
                        self.tracks.get(&done.target).map(|t| t.is_paused).unwrap_or(true)
                    };
                    if paused {
                        self.pending.remove(&done.target);
                        continue;
                    }
                    let params = self.pending.get(&done.target).unwrap().params.clone();
                    let vol = params.volume;
                    if done.target == LIBRARY_TRACK_ID {
                        self.library.accept_loop_incoming(&self.engine, handle, &params, vol);
                    } else if let Some(t) = self.tracks.get_mut(&done.target) {
                        t.voice.accept_loop_incoming(&self.engine, handle, &params, vol);
                    }
                    // Handed off; the post-swap re-request creates a fresh entry.
                    self.pending.remove(&done.target);
                }
                Some(PreloadKind::NextClip) => {
                    // Succeeded; clear any earlier failure stamp so the gate in
                    // `request_next_clip_preloads` doesn't outlive its cause (I-1).
                    if let Some(t) = self.tracks.get_mut(&done.target) {
                        t.last_preload_failure = None;
                    }
                    if let Some(p) = self.pending.get_mut(&done.target) {
                        p.ready = Some(handle);
                    }
                }
                None => { /* invalidated between route check and here; drop */ }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AudioEngine;
    use crate::models::{Clip, Track, TrackMode, TrackTransition};
    use std::path::Path;
    use std::time::Duration;

    fn fixture() -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sample.opus")
            .to_str()
            .unwrap()
            .to_string()
    }

    /// Pull everything currently queued in the broadcast tap.
    fn drain_tap(audio: &AudioEngine) -> Vec<f32> {
        let eng = audio.engine_for_test();
        let ch = eng.channels() as usize;
        let mut scratch = vec![0f32; 4800 * ch];
        let mut all = Vec::new();
        loop {
            let frames = eng.tap_read(&mut scratch);
            if frames == 0 {
                break;
            }
            all.extend_from_slice(&scratch[..frames * ch]);
        }
        all
    }

    /// Largest absolute sample value (0.0 for silence or an empty slice).
    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// The library preview is for the operator's ears only: it must never
    /// reach the broadcast, even while the tap is on.
    #[test]
    fn library_preview_never_reaches_the_tap() {
        let mut audio = AudioEngine::new().expect("engine");
        audio.engine_for_test().tap_set_enabled(true);
        drain_tap(&audio);
        audio
            .play_library_sound(super::sound::PlayParams {
                file_path: fixture(),
                start_position: 10.0,
                volume: 1.0,
                trim_start: None,
                trim_end: None,
                loop_region: None,
                fade_in: None,
                fade_out: None,
            })
            .expect("play preview");
        std::thread::sleep(Duration::from_millis(300));
        let captured = drain_tap(&audio);
        assert!(!captured.is_empty(), "tap produced no frames while enabled");
        assert!(peak(&captured) < 1e-6, "library preview leaked into the tap");
        audio.stop_library_sound();
        audio.engine_for_test().tap_set_enabled(false);
    }

    /// Master volume is the operator's local speaker level. The tap sits
    /// before it, so turning master all the way down must not silence the
    /// broadcast.
    #[test]
    fn master_volume_zero_does_not_silence_the_tap() {
        let mut audio = AudioEngine::new().expect("engine");
        let track = Track {
            id: "t1".into(),
            name: "Track".into(),
            clips: vec![Clip {
                id: "c1".into(),
                name: "Clip".into(),
                file_path: fixture(),
                volume: 1.0,
                library_entry_id: "e1".into(),
                library_volume: 1.0,
                // Start 10 s in, where the fixture is clearly audible.
                trim_start: Some(10.0),
                trim_end: None,
            }],
            volume: 1.0,
            mode: TrackMode::Sequence { repeat: false, transition: TrackTransition::Gapless },
            fade_in: 0.0,
            fade_out: 0.0,
        };
        audio.setup_track(&track).expect("setup track");
        audio.set_master_volume(0.0);
        audio.engine_for_test().tap_set_enabled(true);
        drain_tap(&audio);
        audio.play_track("t1").expect("play track");
        std::thread::sleep(Duration::from_millis(300));
        assert!(peak(&drain_tap(&audio)) > 0.01, "master volume 0 silenced the tap");
        audio.engine_for_test().tap_set_enabled(false);
    }
}
