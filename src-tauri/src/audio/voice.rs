use std::time::{Duration, Instant};

use super::ffi;
use super::sound::{self, PlayParams, PlaybackState, SoundHandle};

/// How far before loop_end (in seconds) to trigger the crossfade restart.
/// Must be larger than the poll interval (50ms) to reliably catch it.
/// Shared by the library preview loop and seamless-mode tracks.
pub const LOOP_CROSSFADE_SECS: f64 = 0.06;

/// De-click ramp applied to a scheduled trim-end stop. Short enough to be
/// inaudible as a fade (the clip is supposed to end here) but long enough to
/// kill the step-discontinuity click of cutting at a non-zero sample. Mirrors
/// the shim's `LD_MIN_FADE_MS` floor; the shim re-floors anyway.
const TRIM_STOP_FADE_SECS: f64 = 0.005;

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
    /// Whether the seamless-loop restart should be sample-accurately anchored to
    /// the start of the crossfade window via `schedule_start_frame`. Tracks use
    /// this (engine-clock anchored); the library preview does not (matches the
    /// pre-refactor behavior of each path exactly).
    scheduled_loops: bool,
    /// This Voice's sub-mix group. Declared LAST so it drops after all sounds.
    group: ffi::Group,
}

impl Voice {
    /// Create a Voice with a fresh sub-mix group on `engine`. Loop restarts are
    /// NOT sample-accurately anchored (library-preview behavior).
    pub fn new(engine: &ffi::Engine) -> Result<Self, String> {
        Self::with_scheduled_loops(engine, false)
    }

    /// Like [`Voice::new`] but with sample-accurate loop-restart anchoring
    /// enabled (per-track seamless-mode behavior).
    pub fn with_scheduled_loops(engine: &ffi::Engine, scheduled_loops: bool) -> Result<Self, String> {
        let group = engine.create_group()?;
        Ok(Self {
            current: None,
            fading_out: Vec::new(),
            loop_restart: None,
            scheduled_loops,
            group,
        })
    }

    /// Set this Voice's group (sub-mix) volume. Track/library group node.
    pub fn set_group_volume(&self, linear: f32) {
        self.group.set_volume(linear);
    }

    /// Stop the current sound abruptly (no fade) WITHOUT parking it. Used as the
    /// pre-replace stop inside `play`, mirroring the old `start_clip`.
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
    /// via `fade_out_and_release`/`hold_until_silent`/`begin_transition_handoff` BEFORE calling `play`.
    ///
    /// If `params.loop_region` is `Some`, loop-restart info is captured so
    /// `poll_loop` can perform the seamless crossfade.
    ///
    /// `schedule_trim_stop` requests a sample-accurate, de-clicked stop scheduled
    /// at the clip's `trim_end` (see [`Voice::play_scheduled`] / Fix 1). It is
    /// honored ONLY for a non-looping clip that has a `trim_end`; looping clips
    /// (whose restart is driven by `poll_loop`) and clips that will be handed to
    /// a clip-crossfade (whose own fade governs the outgoing tail) must pass
    /// `false` so the scheduled stop doesn't kill the loop or short-cut the
    /// crossfade.
    pub fn play(&mut self, engine: &ffi::Engine, params: PlayParams) -> Result<(), String> {
        self.play_inner(engine, params, false)
    }

    /// Like [`Voice::play`] but schedules a de-clicked faded stop at the clip's
    /// `trim_end` (non-looping clips only). Used by the library preview and by
    /// non-crossfade track clips so the trim-end stop ramps to silence exactly at
    /// the trim point instead of being hard-cut by a late poll check (Fix 1).
    pub fn play_scheduled(&mut self, engine: &ffi::Engine, params: PlayParams) -> Result<(), String> {
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
            let f = fade.or(snd.fade_out).unwrap_or(0.02);
            self.fade_out_and_release(snd, f);
        }
        self.loop_restart = None;
    }

    /// Stop the current sound abruptly (no fade, no park). Cursor is discarded.
    /// Caller is responsible for any de-click need.
    pub fn stop_quick(&mut self) {
        self.stop_current_quick();
        self.current = None;
        self.loop_restart = None;
    }

    /// Take the current sound out without stopping it (used when the orchestrator
    /// hands the outgoing sound to a clip-crossfade transition). Loop-restart
    /// info is cleared. Does NOT touch the parked-sound reaper.
    pub fn take_current(&mut self) -> Option<SoundHandle> {
        self.loop_restart = None;
        self.current.take()
    }

    /// Clear the current sound slot without stopping anything (used on gap
    /// scheduling where the sound has already ended). Loop-restart cleared.
    pub fn clear_current(&mut self) {
        self.current = None;
        self.loop_restart = None;
    }

    /// Pause the current sound, preserving its cursor. NOT a retire.
    pub fn pause(&mut self) {
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
    }

    // --- Voice reaper (de-click) ---

    /// Deferred destruction: request a fade-out on `snd`, then hold it until the
    /// ramp completes. The reaper drops it afterwards. Keeps the sound in the
    /// mixer for the whole ramp so the fade is actually audible (dropping it
    /// immediately runs `ma_sound_uninit` mid-fade → hard cut → click).
    pub fn fade_out_and_release(&mut self, mut snd: SoundHandle, fade_secs: f64) {
        let fade = fade_secs.max(0.01);
        sound::stop(&mut snd, Some(Duration::from_secs_f64(fade)));
        let deadline = Instant::now() + Duration::from_secs_f64(fade + 0.05);
        self.fading_out.push((snd, deadline));
    }

    /// Hold an already-fading handle (its fade was requested elsewhere, e.g. the
    /// seamless-loop crossfade or a clip-crossfade hand-off) so it isn't dropped
    /// mid-fade. `fade_secs` is the in-flight fade duration used to size the
    /// deadline.
    pub fn hold_until_silent(&mut self, snd: SoundHandle, fade_secs: f64) {
        let deadline = Instant::now() + Duration::from_secs_f64(fade_secs.max(0.01) + 0.05);
        self.fading_out.push((snd, deadline));
    }

    /// Drop any held sounds whose fade-out has completed. Must be called every
    /// poll tick so nothing leaks.
    pub fn drop_finished(&mut self) {
        let now = Instant::now();
        self.fading_out.retain(|(_, deadline)| now < *deadline);
    }

    // --- Seamless loop crossfade ---

    /// Drive the seamless-loop crossfade and end-of-clip stop for the current
    /// sound. Call once per poll tick (after `reap`, before higher-level
    /// transition logic). Returns what happened so the caller can react.
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
    /// stopped here: the library preview wants this (`true`); tracks do NOT
    /// (`false`) because their natural end is detected by the orchestrator's
    /// transition state machine instead — matching the pre-refactor behavior of
    /// each path exactly.
    pub fn poll_loop(&mut self, engine: &ffi::Engine, should_run: bool, stop_at_end: bool) -> PollResult {
        let (st, pos) = match &self.current {
            Some(snd) => (sound::state(snd), sound::position(snd)),
            None => return PollResult::Idle,
        };

        if !matches!(st, PlaybackState::Playing) || !should_run {
            return PollResult::Idle;
        }

        let loop_region = self.current.as_ref().and_then(|s| s.loop_region);

        if let Some((_ls, le)) = loop_region {
            if pos >= le - LOOP_CROSSFADE_SECS {
                self.do_loop_crossfade(engine, pos, le);
                return PollResult::Looped;
            }
            PollResult::Idle
        } else if stop_at_end {
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

    /// Perform the seamless-loop crossfade: fade the outgoing instance out over
    /// the crossfade window and park it, then start a fresh instance at the loop
    /// start with a matching fade-in. The incoming instance is anchored to the
    /// START of the crossfade window (when `anchor_loop_restart` is set) so the
    /// two overlap across the whole window and sum smoothly through the loop
    /// point (no butt-join seam / pop).
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
        let start_frame = if self.scheduled_loops {
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
            Err(_) => {
                self.current = None;
                self.loop_restart = None;
            }
        }
    }
}
