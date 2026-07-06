use std::time::{Duration, Instant};
use crate::models::{TrackMode, TrackTransition};
use super::sound::SoundHandle;
use super::util::CROSSFADE_OVERLAP;

/// Extra time beyond an outgoing crossfade's own duration before its handle is
/// cleaned up. Absorbs poll-tick jitter so cleanup never races a fade that's
/// still legitimately ramping.
const OUTGOING_CLEANUP_GRACE_SECS: f64 = 0.5;

/// Fraction of a short clip's playable duration used as its crossfade window,
/// when the clip is too short for the full `CROSSFADE_OVERLAP` window.
const SHORT_CLIP_XFADE_FRACTION: f64 = 0.5;

/// Floor on the short-clip crossfade window so a very short clip doesn't
/// trigger the crossfade immediately at clip start.
const MIN_EFFECTIVE_XFADE_SECS: f64 = 0.1;

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

/// The transition phase for the current clip. Replaces the previously separate
/// `crossfade_started: bool` + `pending_start: Option<PendingStart>` pair (which
/// were mutually exclusive in practice). `outgoing` stays a separate field: a
/// crossfade's outgoing sound keeps fading past `clip_started()`, so its
/// lifecycle outlives this phase.
enum TransitionState {
    /// No transition armed for the current clip.
    Idle,
    /// A crossfade to the next clip has been started for the current clip.
    Crossfading,
    /// A gap-delayed next-clip start is pending.
    Gap(PendingStart),
}

pub struct TransitionManager {
    outgoing: Option<OutgoingFade>,
    state: TransitionState,
}

/// The per-tick inputs to `TransitionManager::update`. Bundles the state the
/// orchestrator gathers each poll so the call is one argument instead of ~11.
pub struct TickInputs<'a> {
    pub position: f64,
    pub clip_duration: Option<f64>,
    pub clip_start: Option<f64>,
    pub mode: &'a TrackMode,
    pub is_paused: bool,
    pub sound_playing: bool,
    pub sound_stopped: bool,
    pub next_clip_index: Option<usize>,
}

impl TransitionManager {
    pub fn new() -> Self {
        Self {
            outgoing: None,
            state: TransitionState::Idle,
        }
    }

    /// Main polling tick. Called every `audio::POLL_INTERVAL`.
    /// Returns the action the orchestrator should take.
    pub fn update(&self, t: &TickInputs) -> TransitionAction {
        if t.is_paused {
            return TransitionAction::None;
        }

        // Check for pending gap start
        if let TransitionState::Gap(ref pending) = self.state {
            if Instant::now() >= pending.start_at {
                return TransitionAction::StartPendingClip { clip_index: pending.clip_index };
            }
            return TransitionAction::None;
        }

        // Check for outgoing fade cleanup
        if let Some(ref outgoing) = self.outgoing {
            let should_cleanup = outgoing.started.elapsed()
                >= Duration::from_secs_f64(outgoing.duration + OUTGOING_CLEANUP_GRACE_SECS);
            if should_cleanup {
                return TransitionAction::CleanupOutgoing;
            }
        }

        // Check for sound finished naturally
        if t.sound_stopped && !t.is_paused {
            let next = t.next_clip_index;
            return match next {
                Some(idx) => {
                    // Determine transition type
                    let transition = match t.mode {
                        TrackMode::Sequence { transition, .. } => transition.clone(),
                        TrackMode::Shuffle { transition, .. } => transition.clone(),
                        TrackMode::Seamless { .. } => TrackTransition::Gapless,
                    };
                    match transition {
                        TrackTransition::Gapless => {
                            TransitionAction::AdvanceClip { next_clip_index: idx }
                        }
                        TrackTransition::Gap { min, max } => {
                            TransitionAction::ScheduleGap {
                                next_clip_index: idx,
                                delay_seconds: gap_delay(min, max),
                            }
                        }
                        TrackTransition::Crossfade => {
                            TransitionAction::AdvanceClip { next_clip_index: idx }
                        }
                    }
                }
                None => TransitionAction::None, // orchestrator will stop the track
            };
        }

        // Check for crossfade trigger
        if !matches!(self.state, TransitionState::Crossfading) && t.sound_playing {
            let crossfade_duration = match t.mode {
                TrackMode::Sequence {
                    transition: TrackTransition::Crossfade,
                    ..
                } => CROSSFADE_OVERLAP,
                TrackMode::Shuffle {
                    transition: TrackTransition::Crossfade,
                    ..
                } => CROSSFADE_OVERLAP,
                _ => return TransitionAction::None,
            };

            if let Some(clip_dur) = t.clip_duration {
                let start = t.clip_start.unwrap_or(0.0);
                let playable = clip_dur - start;
                // Short-clip fix: if the clip is shorter than the full crossfade window,
                // use half the playable duration (minimum 0.1s) to avoid triggering the
                // crossfade immediately at clip start when the clip is shorter than the
                // window.
                let effective_xfade = if playable > crossfade_duration {
                    crossfade_duration
                } else {
                    (playable * SHORT_CLIP_XFADE_FRACTION).max(MIN_EFFECTIVE_XFADE_SECS)
                };

                if t.position >= clip_dur - effective_xfade {
                    if let Some(idx) = t.next_clip_index {
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

    /// Store the outgoing sound handle and record the crossfade start time.
    pub fn begin_crossfade(&mut self, outgoing_handle: SoundHandle, duration: f64) {
        self.outgoing = Some(OutgoingFade {
            handle: outgoing_handle,
            started: Instant::now(),
            duration,
        });
        self.state = TransitionState::Crossfading;
    }

    /// Schedule a delayed clip start after a gap transition.
    pub fn schedule_gap(&mut self, clip_index: usize, delay: f64) {
        self.state = TransitionState::Gap(PendingStart {
            clip_index,
            start_at: Instant::now() + Duration::from_secs_f64(delay),
        });
    }

    /// Reset crossfade and pending state after a new clip has started.
    pub fn clip_started(&mut self) {
        self.state = TransitionState::Idle;
    }

    /// Stop and drop the outgoing fading sound.
    pub fn cleanup_outgoing(&mut self) {
        if let Some(ref mut outgoing) = self.outgoing {
            super::sound::stop_quick(&mut outgoing.handle);
        }
        self.outgoing = None;
    }

    /// Clear all transition state (called on track stop/reset).
    pub fn reset(&mut self) {
        self.cleanup_outgoing();
        self.state = TransitionState::Idle;
    }

    /// Returns true if there is a gap-delayed clip start pending.
    pub fn has_pending(&self) -> bool {
        matches!(self.state, TransitionState::Gap(_))
    }
}

/// Determine the next clip index given the current playback mode.
///
/// - Sequence: advance to next; wrap to 0 if repeat, else None.
/// - Shuffle: random index avoiding an immediate repeat. Without `repeat`,
///   stops after `clip_count` plays (counted by the orchestrator) — clips may
///   repeat/skip within those plays; exact-once shuffle is a future feature.
/// - Seamless: always None (the Voice loops the single clip itself).
pub fn pick_next_clip_index(
    mode: &TrackMode,
    current_index: Option<usize>,
    clip_count: usize,
    clips_played: usize,
) -> Option<usize> {
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
            if !*repeat && clips_played >= clip_count {
                return None;
            }
            if clip_count == 1 {
                return Some(0);
            }
            use rand::Rng;
            let mut rng = rand::thread_rng();
            // Uniform over the clip_count-1 indices != current (bounded; the
            // old unbounded reroll loop could in principle spin).
            match current_index {
                Some(cur) if cur < clip_count => {
                    let idx = rng.gen_range(0..clip_count - 1);
                    Some(if idx >= cur { idx + 1 } else { idx })
                }
                _ => Some(rng.gen_range(0..clip_count)),
            }
        }
    }
}

/// Random delay for a Gap transition. Tolerates inverted (min > max) and
/// non-finite config: `gen_range` panics on an empty range, and a panic on
/// the poll thread poisons the audio mutex.
fn gap_delay(min: f64, max: f64) -> f64 {
    use rand::Rng;
    let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
    if lo.is_finite() && hi.is_finite() && lo <= hi {
        rand::thread_rng().gen_range(lo..=hi)
    } else {
        0.0
    }
}

/// How long after a failed clip start to suppress transition-driven retries
/// (D8). Without this, a disconnected drive triggers a ~20 Hz blocking-open
/// retry storm on the poll thread.
pub const START_RETRY_BACKOFF: Duration = Duration::from_secs(2);

/// Whether a clip start should be suppressed because a recent start failed and
/// the backoff window has not elapsed. Pure decision (testable without a device).
pub fn retry_suppressed(last_failure: Option<Instant>, now: Instant) -> bool {
    match last_failure {
        Some(t) => now.duration_since(t) < START_RETRY_BACKOFF,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_delay_tolerates_inverted_range() {
        let d = gap_delay(5.0, 2.0); // must not panic
        assert!((2.0..=5.0).contains(&d));
    }

    #[test]
    fn gap_delay_handles_degenerate_and_non_finite() {
        assert_eq!(gap_delay(3.0, 3.0), 3.0);
        assert_eq!(gap_delay(f64::NAN, 3.0), 0.0);
        assert_eq!(gap_delay(1.0, f64::INFINITY), 0.0);
    }

    fn shuffle(repeat: bool) -> TrackMode {
        TrackMode::Shuffle { repeat, transition: TrackTransition::Gapless }
    }

    #[test]
    fn shuffle_without_repeat_stops_after_clip_count_plays() {
        assert!(pick_next_clip_index(&shuffle(false), Some(1), 3, 3).is_none());
        assert!(pick_next_clip_index(&shuffle(false), Some(1), 3, 2).is_some());
    }

    #[test]
    fn shuffle_with_repeat_never_stops() {
        assert!(pick_next_clip_index(&shuffle(true), Some(0), 3, 300).is_some());
        assert_eq!(pick_next_clip_index(&shuffle(true), Some(0), 1, 5), Some(0));
    }

    #[test]
    fn shuffle_single_clip_no_repeat_plays_once() {
        assert!(pick_next_clip_index(&shuffle(false), Some(0), 1, 1).is_none());
    }

    #[test]
    fn shuffle_avoids_immediate_repeat() {
        for _ in 0..50 {
            let next = pick_next_clip_index(&shuffle(true), Some(2), 4, 1);
            assert_ne!(next, Some(2));
        }
    }

    #[test]
    fn sequence_semantics_unchanged() {
        let seq = |repeat| TrackMode::Sequence { repeat, transition: TrackTransition::Gapless };
        assert_eq!(pick_next_clip_index(&seq(false), Some(0), 3, 1), Some(1));
        assert!(pick_next_clip_index(&seq(false), Some(2), 3, 3).is_none());
        assert_eq!(pick_next_clip_index(&seq(true), Some(2), 3, 3), Some(0));
    }

    fn seq(repeat: bool) -> TrackMode {
        TrackMode::Sequence { repeat, transition: TrackTransition::Gapless }
    }

    #[test]
    fn update_uses_prepicked_next_index_on_natural_end() {
        let tm = TransitionManager::new();
        // sound_stopped = true, not paused: the natural-end branch fires and must
        // use the PASSED next index, not recompute it.
        let action = tm.update(&TickInputs {
            position: 10.0, clip_duration: Some(10.0), clip_start: Some(0.0), mode: &seq(true),
            is_paused: false, sound_playing: false, sound_stopped: true, next_clip_index: Some(2),
        });
        match action {
            TransitionAction::AdvanceClip { next_clip_index } => assert_eq!(next_clip_index, 2),
            _ => panic!("expected AdvanceClip{{2}}"),
        }
    }

    #[test]
    fn update_ends_track_when_prepicked_index_is_none() {
        let tm = TransitionManager::new();
        let action = tm.update(&TickInputs {
            position: 10.0, clip_duration: Some(10.0), clip_start: Some(0.0), mode: &seq(false),
            is_paused: false, sound_playing: false, sound_stopped: true, next_clip_index: None,
        });
        assert!(matches!(action, TransitionAction::None));
    }

    fn seq_crossfade() -> TrackMode {
        TrackMode::Sequence { repeat: true, transition: TrackTransition::Crossfade }
    }

    #[test]
    fn update_crossfade_trigger_uses_prepicked_next_index() {
        let tm = TransitionManager::new();
        // clip_dur=10.0, clip_start=0.0: playable (10.0) > CROSSFADE_OVERLAP
        // (4.0), so effective_xfade == CROSSFADE_OVERLAP and the window opens at
        // position 6.0. position=7.0 is inside it; sound still playing, not
        // stopped, not paused: the crossfade-trigger branch fires and must use
        // the PASSED next index, not recompute it.
        let action = tm.update(&TickInputs {
            position: 7.0, clip_duration: Some(10.0), clip_start: Some(0.0), mode: &seq_crossfade(),
            is_paused: false, sound_playing: true, sound_stopped: false, next_clip_index: Some(2),
        });
        match action {
            TransitionAction::StartCrossfade { next_clip_index, duration } => {
                assert_eq!(next_clip_index, 2);
                assert_eq!(duration, CROSSFADE_OVERLAP);
            }
            _ => panic!("expected StartCrossfade{{next_clip_index: 2}}"),
        }
    }

    #[test]
    fn update_crossfade_trigger_does_not_fire_without_prepicked_index() {
        let tm = TransitionManager::new();
        // Same in-window position/state as above, but no pre-picked next clip
        // (e.g. a non-repeating track on its last clip): no crossfade fires.
        let action = tm.update(&TickInputs {
            position: 7.0, clip_duration: Some(10.0), clip_start: Some(0.0), mode: &seq_crossfade(),
            is_paused: false, sound_playing: true, sound_stopped: false, next_clip_index: None,
        });
        assert!(matches!(action, TransitionAction::None));
    }

    #[test]
    fn retry_suppressed_only_within_backoff() {
        let now = Instant::now();
        assert!(!retry_suppressed(None, now));
        assert!(retry_suppressed(Some(now), now)); // just failed
        let old = now - (START_RETRY_BACKOFF + Duration::from_millis(1));
        assert!(!retry_suppressed(Some(old), now)); // past the window
    }

    /// I-1: `request_next_clip_preloads` gates on this same pure function via
    /// `ManagedTrack.last_preload_failure`. Pins the three decision points that
    /// matter for that gate: suppressed right after a stamped failure, not
    /// suppressed once the backoff window has elapsed, and not suppressed once
    /// the stamp has been cleared (e.g. by a successful preload or a track
    /// reset/re-pick).
    #[test]
    fn next_clip_preload_retry_gate_suppressed_then_clears() {
        let now = Instant::now();
        // Just stamped a failure: suppressed.
        assert!(retry_suppressed(Some(now), now));
        // Past the backoff window: not suppressed.
        let old = now - (START_RETRY_BACKOFF + Duration::from_millis(1));
        assert!(!retry_suppressed(Some(old), now));
        // Stamp cleared (None): not suppressed.
        assert!(!retry_suppressed(None, now));
    }
}
