use std::time::{Duration, Instant};

use super::ffi;
use super::sound::{self, PlayParams, PlaybackState, SoundHandle};

/// How far before loop_end (in seconds) to trigger the crossfade restart.
/// Sized to at least TWO poll periods (2.4 × `POLL_INTERVAL`) so a single
/// delayed tick can't land the check past loop_end and audibly overshoot the
/// loop point.
/// Shared by the library preview loop and seamless-mode tracks.
/// (Engine-clock loop scheduling — removing the poll dependency entirely —
/// is Plan 3C, alongside the loop-instance preload.)
pub const LOOP_CROSSFADE_SECS: f64 = 0.12;

/// De-click ramp applied to a scheduled trim-end stop. Short enough to be
/// inaudible as a fade (the clip is supposed to end here) but long enough to
/// kill the step-discontinuity click of cutting at a non-zero sample. Mirrors
/// the shim's `LD_MIN_FADE_MS` floor; the shim re-floors anyway.
const TRIM_STOP_FADE_SECS: f64 = 0.005;

/// Wall-clock grace beyond a parked sound's fade deadline before it is
/// force-dropped even if the device still reports it playing. Leak backstop
/// only — normal reaping is driven by the sound's own state (see
/// `drop_finished`); a device stalled longer than this gets a hard cut,
/// which beats leaking sounds forever.
const REAP_BACKSTOP: Duration = Duration::from_secs(2);

/// Fallback de-click fade when neither the caller nor the sound's own
/// `fade_out` supplies one. Used by `Voice::stop` and (with the same value)
/// `orchestrator::stop_track`'s fallback.
pub const DEFAULT_STOP_FADE_SECS: f64 = 0.02;

/// Wall-clock grace added on top of an in-flight fade's own duration before
/// its deadline, absorbing scheduling jitter so `drop_finished`'s backstop
/// doesn't race a fade that's still legitimately ramping.
const FADE_DEADLINE_SLACK_SECS: f64 = 0.05;

/// Everything needed to restart a sound at the start of its loop region for the
/// seamless crossfade. Captured at `play` time from the same `PlayParams` used
/// to start the sound, so the restart is byte-for-byte the same source/trim.
#[derive(Clone)]
struct LoopRestart {
    file_path: String,
    volume: f32,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_region: Option<(f64, f64)>,
    /// Position to restart at: the start of the loop region.
    loop_start_position: f64,
}

/// Whether `poll_loop` should stop a non-looping sound that passed its
/// trim_end (library preview: yes; tracks: no — their transition machine
/// handles it).
#[derive(Clone, Copy, PartialEq)]
pub enum StopAtEnd {
    Yes,
    No,
}

/// Whether seamless-loop restarts are sample-accurately anchored to the
/// engine clock (tracks) or started immediately on detection (library
/// preview).
#[derive(Clone, Copy, PartialEq)]
pub enum LoopAnchor {
    EngineClock,
    Immediate,
}

/// What `poll_loop` did this tick, so the caller can react (e.g. re-emit state,
/// record a found library entry). `Looped` means a seamless-loop crossfade was
/// performed; `Ended` means a non-looping sound hit its trim_end and was stopped.
pub enum PollResult {
    /// Nothing to do (not playing, paused, or mid-clip).
    Idle,
    /// A seamless-loop crossfade restart was performed.
    Looped,
    /// A non-looping sound reached its end and was stopped.
    Ended,
}

/// Owns the mechanics of playing ONE sound at a time inside a sub-mix group,
/// with click-free stop (the fade-out reaper) and seamless loop crossfading.
///
/// Both the library preview (`AudioEngine.library`) and each playback track
/// (`ManagedTrack.voice`) embed a `Voice`; the multi-clip sequencing logic lives
/// in the orchestrator, which drives the Voice's single-sound operations.
///
/// # Drop order
/// Sounds (`current` + `fading_out`) are declared before `group` so Rust drops
/// them first; the `group` is declared last here so it drops after every sound.
/// The owning struct must in turn declare its `Voice` before the `ffi::Engine`
/// so the whole graph tears down innermost-first (Sounds → Group → Engine), as
/// miniaudio requires.
pub struct Voice {
    /// The sound currently playing (or just stopped, awaiting a poll transition).
    current: Option<SoundHandle>,
    /// De-click holding area: sounds that were stopped/replaced with a fade-out
    /// are parked here until the ramp completes, then reaped. Dropping a sound
    /// mid-fade runs `ma_sound_uninit` → hard cut → click, so we must keep it
    /// alive (mixing) for the whole fade. Declared before `group`.
    fading_out: Vec<(SoundHandle, Instant)>,
    /// Parameters to restart the current sound for a seamless loop crossfade.
    /// `Some` only while a looping sound is playing.
    loop_restart: Option<LoopRestart>,
    /// The preloaded next loop instance (D6). For `EngineClock` it is already
    /// attached and scheduled ahead on the engine clock, awaiting the swap; for
    /// `Immediate` it is attached but unstarted, awaiting the window.
    next_loop: Option<SoundHandle>,
    /// Loop-restart snapshot for the INCOMING staged instance; promoted to
    /// `loop_restart` when the swap completes (`perform_loop_swap`).
    loop_incoming_restart: Option<LoopRestart>,
    /// Whether we have already asked (via `loop_wants_preload`) for the next
    /// instance of the CURRENT looping sound. Reset to false on each swap.
    loop_preload_requested: bool,
    /// Paths for which the slow-drive synchronous fallback has already warned,
    /// so the log warns once per path, not every tick.
    fallback_warned: std::collections::HashSet<String>,
    /// Whether the seamless-loop restart should be sample-accurately anchored to
    /// the start of the crossfade window via `schedule_start_frame`. Tracks use
    /// `EngineClock`; the library preview uses `Immediate` (matches the
    /// pre-refactor behavior of each path exactly).
    anchor: LoopAnchor,
    /// This Voice's sub-mix group. Declared LAST so it drops after all sounds.
    group: ffi::Group,
}

impl Voice {
    /// Create a Voice with a fresh sub-mix group on `engine`. Loop restarts are
    /// NOT sample-accurately anchored (library-preview behavior).
    pub fn new(engine: &ffi::Engine) -> Result<Self, String> {
        Self::with_loop_anchor(engine, LoopAnchor::Immediate)
    }

    /// Like [`Voice::new`] but with sample-accurate loop-restart anchoring
    /// enabled (per-track seamless-mode behavior).
    pub fn with_loop_anchor(engine: &ffi::Engine, anchor: LoopAnchor) -> Result<Self, String> {
        let group = engine.create_group()?;
        Ok(Self {
            current: None,
            fading_out: Vec::new(),
            loop_restart: None,
            next_loop: None,
            loop_incoming_restart: None,
            loop_preload_requested: false,
            fallback_warned: std::collections::HashSet::new(),
            anchor,
            group,
        })
    }

    /// Set this Voice's group (sub-mix) volume. Track/library group node.
    pub fn set_group_volume(&self, linear: f32) {
        self.group.set_volume(linear);
    }

    /// Stop the current sound with a minimal de-click fade (~4 ms floor in the
    /// shim) WITHOUT parking it. Used as the pre-replace stop inside `play`,
    /// mirroring the old `start_clip`.
    fn stop_current_quick(&mut self) {
        if let Some(ref mut snd) = self.current {
            sound::stop_quick(snd);
        }
    }

    /// Start a new sound in this Voice's group, replacing the current one.
    ///
    /// The previously-playing sound (if any) is stopped abruptly (`stop_quick`),
    /// matching the old `start_clip` behavior; callers that need a click-free
    /// hand-off (stop, loop crossfade, clip crossfade) hold the outgoing sound
    /// via `fade_out_and_release`/`hold_until_silent`/`take_current` +
    /// `TransitionManager::begin_crossfade` BEFORE calling `play`.
    ///
    /// If `params.loop_region` is `Some`, loop-restart info is captured so
    /// `poll_loop` can perform the seamless crossfade.
    ///
    /// `schedule_trim_stop` requests a sample-accurate, de-clicked stop at the
    /// clip's `trim_end` (see [`Voice::play_with_trim_stop`]) — replacing the
    /// old late poll-check hard cut. It is honored ONLY for a non-looping clip
    /// that has a `trim_end`; looping clips (whose restart is driven by
    /// `poll_loop`) and clips that will be handed to a clip-crossfade (whose
    /// own fade governs the outgoing tail) must pass
    /// `false` so the scheduled stop doesn't kill the loop or short-cut the
    /// crossfade.
    pub fn play(&mut self, engine: &ffi::Engine, params: PlayParams) -> Result<(), String> {
        self.play_inner(engine, params, false)
    }

    /// Like [`Voice::play`] but schedules a de-clicked faded stop at the clip's
    /// `trim_end` (non-looping clips only). Used by the library preview and by
    /// non-crossfade track clips so the trim-end stop ramps to silence exactly at
    /// the trim point instead of being hard-cut by a late poll check.
    pub fn play_with_trim_stop(&mut self, engine: &ffi::Engine, params: PlayParams) -> Result<(), String> {
        self.play_inner(engine, params, true)
    }

    fn play_inner(
        &mut self,
        engine: &ffi::Engine,
        params: PlayParams,
        schedule_trim_stop: bool,
    ) -> Result<(), String> {
        let restart = params.loop_region.map(|(ls, _)| LoopRestart {
            file_path: params.file_path.clone(),
            volume: params.volume,
            trim_start: params.trim_start,
            trim_end: params.trim_end,
            loop_region: params.loop_region,
            loop_start_position: ls,
        });

        // Capture what we need for the scheduled trim-end stop before `params` is
        // moved into `sound::play`. Only for a non-looping clip with a trim_end.
        let scheduled_stop = if schedule_trim_stop && params.loop_region.is_none() {
            params.trim_end.map(|te| {
                let start = params.start_position.max(params.trim_start.unwrap_or(0.0));
                (te - start).max(0.0)
            })
        } else {
            None
        };

        // Replacing `current`: any staged next loop instance belongs to the OLD
        // sound and (EngineClock) may already be scheduled live — discard it.
        self.discard_loop_staging();
        self.stop_current_quick();

        let snd = sound::play(engine, &self.group, params)?;
        if let Some(secs_from_now) = scheduled_stop {
            snd.sound.schedule_stop_secs(secs_from_now, TRIM_STOP_FADE_SECS);
        }
        self.current = Some(snd);
        self.loop_restart = restart;
        Ok(())
    }

    /// Stop the current sound with a fade-out, holding it until silent so the
    /// tail is audible (de-click). `fade` is the fade-out duration in seconds;
    /// if `None`, the sound's own `fade_out` is used, else a small de-click default.
    pub fn stop(&mut self, fade: Option<f64>) {
        if let Some(snd) = self.current.take() {
            let f = fade.or(snd.fade_out).unwrap_or(DEFAULT_STOP_FADE_SECS);
            self.fade_out_and_release(snd, f);
        }
        self.discard_loop_staging();
        self.fallback_warned.clear();
        self.loop_restart = None;
    }

    /// Stop the current sound with a minimal de-click fade (~4 ms floor in the
    /// shim), without parking it. Cursor is discarded.
    pub fn stop_quick(&mut self) {
        self.stop_current_quick();
        self.current = None;
        self.discard_loop_staging();
        self.fallback_warned.clear();
        self.loop_restart = None;
    }

    /// Take the current sound out without stopping it (used when the orchestrator
    /// hands the outgoing sound to a clip-crossfade transition). Loop-restart
    /// info is cleared. Does NOT touch the parked-sound reaper.
    // INVARIANT: the escaped `SoundHandle` is still bound to the mutex contract
    // documented in `ffi.rs` — it is only safe to call further methods on it
    // while holding the same `AudioEngine` lock this `Voice` is called under.
    // Escaping the Voice does not grant it independent thread-safety.
    pub fn take_current(&mut self) -> Option<SoundHandle> {
        self.discard_loop_staging();
        self.loop_restart = None;
        self.current.take()
    }

    /// Clear the current sound slot without stopping anything (used on gap
    /// scheduling where the sound has already ended). Loop-restart cleared.
    pub fn clear_current(&mut self) {
        self.current = None;
        self.discard_loop_staging();
        self.loop_restart = None;
    }

    /// Pause the current sound, preserving its cursor. NOT a retire.
    pub fn pause(&mut self) {
        // A staged EngineClock incoming fires on the audio clock, which keeps
        // running while this sound is paused — discard it or it plays as a
        // ghost under the pause. Re-requested automatically on resume.
        self.discard_loop_staging();
        if let Some(ref mut snd) = self.current {
            sound::pause(snd);
        }
    }

    /// Resume the current sound from its preserved cursor.
    pub fn resume(&mut self) {
        if let Some(ref mut snd) = self.current {
            sound::resume(snd);
        }
    }

    /// Current playback position in seconds (0.0 if nothing playing).
    pub fn position(&self) -> f64 {
        self.current.as_ref().map(sound::position).unwrap_or(0.0)
    }

    /// Playback state of the current sound (Stopped if none).
    pub fn state(&self) -> PlaybackState {
        self.current.as_ref().map(sound::state).unwrap_or(PlaybackState::Stopped)
    }

    /// Whether a sound is currently in the slot (playing, paused, or just-stopped
    /// awaiting a poll transition).
    pub fn has_current(&self) -> bool {
        self.current.is_some()
    }

    /// Effective duration (trim_end or file length) of the current sound.
    pub fn duration(&self) -> Option<f64> {
        self.current.as_ref().and_then(|s| s.duration)
    }

    /// Start position the current sound was started at, if any.
    pub fn start_position(&self) -> Option<f64> {
        self.current.as_ref().and_then(|s| s.start_position)
    }

    /// Set the current sound's (node) volume. Also updates the stored
    /// loop-restart volume so the next seamless-loop instance starts at the new
    /// level (mirrors the pre-refactor track behavior, where the loop restart
    /// recomputed volume from live clip config each iteration).
    pub fn set_volume(&mut self, linear: f32) {
        if let Some(ref mut snd) = self.current {
            sound::set_volume(snd, linear);
        }
        if let Some(ref mut r) = self.loop_restart {
            r.volume = linear;
        }
        // The staged next loop instance is already attached (and, for
        // EngineClock, scheduled live) with its node volume set at accept —
        // keep it and its restart snapshot in step so the post-swap instance
        // doesn't come in at the stale level.
        if let Some(ref mut staged) = self.next_loop {
            sound::set_volume(staged, linear);
        }
        if let Some(ref mut r) = self.loop_incoming_restart {
            r.volume = linear;
        }
    }

    // --- Voice reaper (de-click) ---

    /// Deferred destruction: request a fade-out on `snd`, then hold it until the
    /// ramp completes. The reaper drops it afterwards. Keeps the sound mixing
    /// (in miniaudio's node graph) for the whole ramp so the fade is actually
    /// audible (dropping it immediately runs `ma_sound_uninit` mid-fade → hard
    /// cut → click).
    pub fn fade_out_and_release(&mut self, mut snd: SoundHandle, fade_secs: f64) {
        let fade = fade_secs.max(0.01);
        sound::stop(&mut snd, Some(Duration::from_secs_f64(fade)));
        let deadline = Instant::now() + Duration::from_secs_f64(fade + FADE_DEADLINE_SLACK_SECS);
        self.fading_out.push((snd, deadline));
    }

    /// Hold an already-fading handle (its fade was requested elsewhere, e.g. the
    /// seamless-loop crossfade or a clip-crossfade hand-off) so it isn't dropped
    /// mid-fade. `fade_secs` is the in-flight fade duration used to size the
    /// deadline.
    pub fn hold_until_silent(&mut self, snd: SoundHandle, fade_secs: f64) {
        let deadline = Instant::now() + Duration::from_secs_f64(fade_secs.max(0.01) + FADE_DEADLINE_SLACK_SECS);
        self.fading_out.push((snd, deadline));
    }

    /// Drop any held sounds whose fade-out has completed. Must be called every
    /// poll tick so nothing leaks.
    ///
    /// Reaps on the sound's own state (engine-clock truth): once the fade ran
    /// to completion miniaudio reports the sound stopped, and it can be
    /// dropped immediately. The wall-clock deadline (+`REAP_BACKSTOP` grace)
    /// is only a leak backstop for a stalled device — previously it was the
    /// SOLE criterion, so a stall could let the deadline pass mid-fade and
    /// the drop's `ma_sound_uninit` hard-cut a still-audible sound.
    pub fn drop_finished(&mut self) {
        let now = Instant::now();
        self.fading_out.retain(|(snd, deadline)| {
            let still_audible = snd.sound.is_playing() && !snd.sound.at_end();
            still_audible && now < *deadline + REAP_BACKSTOP
        });
    }

    // --- Seamless loop crossfade ---

    /// Drive the seamless-loop crossfade and end-of-clip stop for the current
    /// sound. Call once per poll tick (after `drop_finished`, before
    /// higher-level transition logic). Returns what happened so the caller can
    /// react.
    ///
    /// - If the current sound is looping and within `LOOP_CROSSFADE_SECS` of its
    ///   loop_end, start the next loop instance anchored to the START of the
    ///   crossfade window with a fade-in, while the outgoing fades out over the
    ///   same window and is parked (overlap → no seam/pop).
    /// - If the current sound is NOT looping and has passed its trim_end, stop it
    ///   abruptly (the clip is done; higher-level logic decides what's next).
    ///
    /// `should_run` gates the whole tick (e.g. tracks skip it while paused).
    /// `stop_at_end` controls whether a NON-looping sound past its trim_end is
    /// stopped here: the library preview wants this (`StopAtEnd::Yes`); tracks
    /// do NOT (`StopAtEnd::No`) because their natural end is detected by the
    /// orchestrator's transition state machine instead — matching the
    /// pre-refactor behavior of each path exactly.
    pub fn poll_loop(&mut self, engine: &ffi::Engine, should_run: bool, stop_at_end: StopAtEnd) -> PollResult {
        let (st, pos) = match &self.current {
            Some(snd) => (sound::state(snd), sound::position(snd)),
            None => return PollResult::Idle,
        };

        // Staged-swap detection runs BEFORE the Playing/should_run gate: the
        // EngineClock swap trigger's disjuncts (`!is_playing() || at_end()`)
        // are the logical complement of that gate, so behind it they are
        // unreachable — the outgoing's SCHEDULED stop at loop_end makes it
        // report Stopped, and the very tick that sees it stopped must reach
        // `perform_loop_swap` so poll_track returns Looped before the
        // transition machine mistakes the scheduled stop for a natural end
        // and kills the track. Pause safety holds because `pause()` discards
        // staging, so `next_loop` is None whenever this Voice is paused.
        if self.next_loop.is_some() {
            if let Some((_ls, le)) = self.current.as_ref().and_then(|s| s.loop_region) {
                // A staged incoming instance exists: EngineClock swaps when the
                // engine clock has crossed the loop point; Immediate starts it
                // when the window arrives.
                let swap_now = match self.anchor {
                    // EngineClock: both sides already scheduled. The outgoing was
                    // told to stop, faded, at loop_end; once it is no longer
                    // playing (past the loop point) the swap is done.
                    LoopAnchor::EngineClock => self
                        .current
                        .as_ref()
                        .map(|c| !c.sound.is_playing() || c.sound.at_end() || pos >= le)
                        .unwrap_or(true),
                    // Immediate: start the staged instance exactly at the window.
                    LoopAnchor::Immediate => pos >= le - LOOP_CROSSFADE_SECS,
                };
                if swap_now {
                    self.perform_loop_swap();
                    return PollResult::Looped;
                }
                return PollResult::Idle;
            }
        }

        if !matches!(st, PlaybackState::Playing) || !should_run {
            return PollResult::Idle;
        }

        let loop_region = self.current.as_ref().and_then(|s| s.loop_region);

        if let Some((_ls, le)) = loop_region {
            // No staged incoming (a staged one was handled above). If we have
            // reached the crossfade window and the
            // preload still isn't ready (slow drive), fall back to the old
            // synchronous restart so the loop never silently stops.
            if pos >= le - LOOP_CROSSFADE_SECS {
                if let Some(r) = &self.loop_restart {
                    if self.fallback_warned.insert(r.file_path.clone()) {
                        log::warn!(
                            "Seamless loop preload not ready in time for '{}' — synchronous restart (slow drive?)",
                            r.file_path
                        );
                    }
                }
                self.do_loop_crossfade(engine, pos, le);
                self.loop_preload_requested = false; // re-request for the new instance
                return PollResult::Looped;
            }
            PollResult::Idle
        } else if stop_at_end == StopAtEnd::Yes {
            let end = self.current.as_ref().and_then(|s| s.duration);
            if let Some(end) = end {
                if pos >= end {
                    self.stop_quick();
                    return PollResult::Ended;
                }
            }
            PollResult::Idle
        } else {
            PollResult::Idle
        }
    }

    /// If the current sound is looping, playing, and has not yet had its next
    /// instance requested (and none is already scheduled), return the params to
    /// preload it and mark it requested. Called by `AudioEngine::poll` under the
    /// mutex; the returned params are handed to the loader thread.
    pub fn loop_wants_preload(&mut self) -> Option<PlayParams> {
        if self.loop_preload_requested || self.next_loop.is_some() {
            return None;
        }
        let restart = self.loop_restart.clone()?;
        let playing = matches!(self.state(), PlaybackState::Playing);
        if !playing {
            return None;
        }
        self.loop_preload_requested = true;
        Some(PlayParams {
            file_path: restart.file_path,
            start_position: restart.loop_start_position,
            volume: restart.volume,
            trim_start: restart.trim_start,
            trim_end: restart.trim_end,
            loop_region: restart.loop_region,
            fade_in: Some(LOOP_CROSSFADE_SECS),
            fade_out: None,
        })
    }

    /// Take a preloaded next loop instance (D6). Attaches it to this Voice's
    /// group and sets its node volume. For `EngineClock`, schedules the incoming
    /// to START at the crossfade-window frame and the OUTGOING (`current`) to
    /// stop, faded, at loop_end — both on the engine clock, so a poll stall
    /// can't cause a seam. For `Immediate` (library preview), leaves it attached
    /// but unstarted; `poll_loop` starts it when the window arrives.
    pub fn accept_loop_incoming(
        &mut self,
        engine: &ffi::Engine,
        handle: SoundHandle,
        params: &PlayParams,
        volume: f32,
    ) {
        // Recompute the loop-restart snapshot for the INCOMING instance so it
        // loops in turn (same source/trim, current volume).
        let restart = params.loop_region.map(|(ls, _)| LoopRestart {
            file_path: params.file_path.clone(),
            volume,
            trim_start: params.trim_start,
            trim_end: params.trim_end,
            loop_region: params.loop_region,
            loop_start_position: ls,
        });

        handle.sound.attach_to_group(&self.group);
        handle.sound.set_volume(volume);

        if self.anchor == LoopAnchor::EngineClock {
            let (le, pos) = match (params.loop_region, self.current.as_ref()) {
                (Some((_ls, le)), Some(cur)) => (le, sound::position(cur)),
                _ => {
                    // No current or no loop region: nothing to crossfade against;
                    // hold it unstarted and let the window path (or a later
                    // request) sort it out.
                    self.next_loop = Some(handle);
                    self.loop_incoming_restart = restart;
                    return;
                }
            };
            // Late completion: the crossfade window is already (nearly) entered,
            // so scheduling would place the incoming's start in the past →
            // mis-timed overlap. Discard the handle — it is ALREADY ATTACHED to
            // this group, so stop + park it in the reaper (a bare drop would
            // `ma_sound_uninit` an attached node) — clear the request flag, and
            // return; `poll_loop`'s synchronous fallback covers this iteration.
            if le - pos <= LOOP_CROSSFADE_SECS {
                let mut handle = handle;
                sound::stop(&mut handle, Some(Duration::from_secs_f64(DEFAULT_STOP_FADE_SECS)));
                self.hold_until_silent(handle, DEFAULT_STOP_FADE_SECS);
                self.loop_preload_requested = false;
                return;
            }
            let sr = engine.sample_rate() as f64;
            let now = engine.time_frames();
            // Incoming starts at the window start (loop_end - crossfade).
            let seconds_until_window = (le - LOOP_CROSSFADE_SECS - pos).max(0.0);
            let start_frame = now + (seconds_until_window * sr) as u64;
            // Start (from the pre-seeked cursor, no re-seek) with a fade-in, then
            // pin the start to the window frame.
            let _ = handle.sound.play_no_seek(LOOP_CROSSFADE_SECS);
            handle.sound.schedule_start_frame(start_frame);
            // Outgoing: faded stop ending at loop_end.
            let seconds_until_end = (le - pos).max(0.0);
            if let Some(cur) = self.current.as_ref() {
                cur.sound.schedule_stop_secs(seconds_until_end, LOOP_CROSSFADE_SECS);
            }
        }

        self.next_loop = Some(handle);
        self.loop_incoming_restart = restart;
    }

    /// Start a preloaded (group-less, pre-seeked) sound as the current sound:
    /// attach to this group, set volume, start from the pre-seeked cursor with
    /// `fade_in`, capture loop-restart if looping, and optionally schedule the
    /// de-clicked trim-end stop (non-looping clips — same rule as
    /// `play_with_trim_stop`). Does NOT stop/park `current`; the caller owns
    /// the outgoing handoff.
    pub fn start_preloaded(
        &mut self,
        handle: SoundHandle,
        params: &PlayParams,
        volume: f32,
        fade_in: Option<f64>,
        schedule_trim_stop: bool,
    ) -> Result<(), String> {
        // Replacing `current`: any staged next loop instance belongs to the OLD
        // sound and (EngineClock) may already be scheduled live — discard it
        // (see `discard_loop_staging`).
        self.discard_loop_staging();

        let restart = params.loop_region.map(|(ls, _)| LoopRestart {
            file_path: params.file_path.clone(),
            volume,
            trim_start: params.trim_start,
            trim_end: params.trim_end,
            loop_region: params.loop_region,
            loop_start_position: ls,
        });

        let scheduled_stop = if schedule_trim_stop && params.loop_region.is_none() {
            params.trim_end.map(|te| {
                let start = params.start_position.max(params.trim_start.unwrap_or(0.0));
                (te - start).max(0.0)
            })
        } else {
            None
        };

        handle.sound.attach_to_group(&self.group);
        handle.sound.set_volume(volume);
        handle
            .sound
            .play_no_seek(fade_in.unwrap_or(0.0))
            .map_err(|e| format!("Failed to start preloaded '{}': {}", params.file_path, e))?;
        if let Some(secs_from_now) = scheduled_stop {
            handle.sound.schedule_stop_secs(secs_from_now, TRIM_STOP_FADE_SECS);
        }
        self.current = Some(handle);
        self.loop_restart = restart;
        Ok(())
    }

    /// Complete a ping-pong swap: park the outgoing `current` in the reaper so
    /// it finishes its already-scheduled fade, promote the staged `next_loop`
    /// to current, and clear the request flag so the next iteration re-requests.
    fn perform_loop_swap(&mut self) {
        if let Some(outgoing) = self.current.take() {
            // For EngineClock the fade was scheduled on the engine clock; for
            // Immediate we fade it here so it overlaps the incoming.
            if self.anchor == LoopAnchor::Immediate {
                let mut outgoing = outgoing;
                sound::stop(&mut outgoing, Some(Duration::from_secs_f64(LOOP_CROSSFADE_SECS)));
                self.hold_until_silent(outgoing, LOOP_CROSSFADE_SECS);
            } else {
                self.hold_until_silent(outgoing, LOOP_CROSSFADE_SECS);
            }
        }
        if let Some(incoming) = self.next_loop.take() {
            // Immediate: never started at accept time — start it now at the
            // window. EngineClock: normally already started + frame-scheduled at
            // accept, but `accept_loop_incoming`'s no-current/no-loop-region arm
            // can stash it UNSTARTED; start it defensively on promote so a
            // promoted instance is never silent.
            if self.anchor == LoopAnchor::Immediate || !incoming.sound.is_playing() {
                if self.anchor != LoopAnchor::Immediate {
                    // Reachable via a device hiccup: `accept_loop_incoming`'s
                    // no-current/no-loop-region arm stashes the incoming
                    // instance unstarted, and `accept_loop_incoming`'s
                    // `play_no_seek` call for the normal EngineClock path can
                    // also fail silently (its `Err` is ignored there). Either
                    // way, starting it now (unanchored) beats leaving it silent.
                    log::warn!("loop swap promoted an unstarted incoming instance — starting now (device hiccup or failed scheduled start)");
                }
                let _ = incoming.sound.play_no_seek(LOOP_CROSSFADE_SECS);
            }
            self.current = Some(incoming);
            self.loop_restart = self.loop_incoming_restart.take();
        }
        self.loop_preload_requested = false;
    }

    /// Discard any staged next loop instance and its bookkeeping. MUST be
    /// called on EVERY path that pauses or replaces `current`: an
    /// EngineClock-scheduled staged incoming is live from the moment it was
    /// accepted (its start frame fires on the audio clock regardless of what
    /// Rust does next), so leaving it staged makes it start as a ghost under a
    /// pause or over a replacement sound. The staged handle is already attached
    /// to this group, so it is stopped and parked in the reaper (a bare drop
    /// would `ma_sound_uninit` an attached, possibly-scheduled node).
    fn discard_loop_staging(&mut self) {
        if let Some(mut staged) = self.next_loop.take() {
            sound::stop(&mut staged, Some(Duration::from_secs_f64(DEFAULT_STOP_FADE_SECS)));
            self.hold_until_silent(staged, DEFAULT_STOP_FADE_SECS);
        }
        self.loop_incoming_restart = None;
        self.loop_preload_requested = false;
    }

    /// Perform the seamless-loop crossfade: fade the outgoing instance out over
    /// the crossfade window and park it, then start a fresh instance at the loop
    /// start with a matching fade-in. The incoming instance is anchored to the
    /// START of the crossfade window (when `anchor` is `LoopAnchor::EngineClock`)
    /// so the two overlap across the whole window and sum smoothly through the
    /// loop point (no butt-join seam / pop).
    fn do_loop_crossfade(&mut self, engine: &ffi::Engine, position: f64, le: f64) {
        let restart = match self.loop_restart.clone() {
            Some(r) => r,
            None => {
                // Looping sound with no restart info should not happen; fail safe
                // by stopping cleanly rather than dropping mid-fade.
                self.stop(Some(LOOP_CROSSFADE_SECS));
                return;
            }
        };

        // Compute the sample-accurate window-start anchor in ENGINE frames. Lead
        // is ~0 (clamped so we never schedule in the past) but computed explicitly
        // to stay robust to small poll-timing jitter. Only used when anchoring.
        let start_frame = if self.anchor == LoopAnchor::EngineClock {
            let seconds_until_window_start = (le - LOOP_CROSSFADE_SECS - position).max(0.0);
            let lead = (seconds_until_window_start * engine.sample_rate() as f64) as u64;
            Some(engine.time_frames() + lead)
        } else {
            None
        };

        // Fade the outgoing instance OUT over the crossfade window (do NOT stop it
        // abruptly) so it overlaps the incoming fade-in. Take it out and park it
        // so the upcoming `play` (which calls stop_quick on `current`) doesn't
        // see it — dropping/uniniting it mid-fade was the residual loop pop.
        if let Some(mut snd) = self.current.take() {
            sound::stop(&mut snd, Some(Duration::from_secs_f64(LOOP_CROSSFADE_SECS)));
            self.hold_until_silent(snd, LOOP_CROSSFADE_SECS);
        }

        let params = PlayParams {
            file_path: restart.file_path.clone(),
            start_position: restart.loop_start_position,
            volume: restart.volume,
            trim_start: restart.trim_start,
            trim_end: restart.trim_end,
            loop_region: restart.loop_region,
            fade_in: Some(LOOP_CROSSFADE_SECS),
            fade_out: None,
        };

        match sound::play(engine, &self.group, params) {
            Ok(new_snd) => {
                if let Some(frame) = start_frame {
                    new_snd.sound.schedule_start_frame(frame);
                }
                self.current = Some(new_snd);
                self.loop_restart = Some(restart);
            }
            Err(e) => {
                // A silent stop here is the "my ambience just stopped and I
                // don't know why" bug — most likely a network-drive hiccup
                // reopening the file. Say so.
                log::warn!(
                    "Seamless loop restart failed for '{}': {} — stopping loop",
                    restart.file_path, e
                );
                self.current = None;
                self.loop_restart = None;
            }
        }
    }
}
