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

    /// Main polling tick. Called every `audio::POLL_INTERVAL`.
    /// Returns the action the orchestrator should take.
    pub fn update(
        &self,
        position: f64,
        clip_duration: Option<f64>,
        clip_start: Option<f64>,
        mode: &TrackMode,
        // Superseded by `next_clip_index` (D7 pre-pick moved the compute-time
        // pick to `start_clip`); kept in the signature so callers don't need to
        // restructure their state-gathering, but no longer read here.
        _current_clip_index: Option<usize>,
        _clip_count: usize,
        _clips_played: usize,
        is_paused: bool,
        sound_playing: bool,
        sound_stopped: bool,
        next_clip_index: Option<usize>,
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
            let should_cleanup = outgoing.started.elapsed()
                >= Duration::from_secs_f64(outgoing.duration + OUTGOING_CLEANUP_GRACE_SECS);
            if should_cleanup {
                return TransitionAction::CleanupOutgoing;
            }
        }

        // Check for sound finished naturally
        if sound_stopped && !is_paused {
            let next = next_clip_index;
            return match next {
                Some(idx) => {
                    // Determine transition type
                    let transition = match mode {
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
        if !self.crossfade_started && sound_playing {
            let crossfade_duration = match mode {
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

            if let Some(clip_dur) = clip_duration {
                let start = clip_start.unwrap_or(0.0);
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

                if position >= clip_dur - effective_xfade {
                    if let Some(idx) = next_clip_index {
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
        self.crossfade_started = true;
    }

    /// Schedule a delayed clip start after a gap transition.
    pub fn schedule_gap(&mut self, clip_index: usize, delay: f64) {
        self.pending_start = Some(PendingStart {
            clip_index,
            start_at: Instant::now() + Duration::from_secs_f64(delay),
        });
    }

    /// Reset crossfade and pending state after a new clip has started.
    pub fn clip_started(&mut self) {
        self.crossfade_started = false;
        self.pending_start = None;
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
        self.crossfade_started = false;
        self.pending_start = None;
    }

    /// Returns true if there is a gap-delayed clip start pending.
    pub fn has_pending(&self) -> bool {
        self.pending_start.is_some()
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
        let action = tm.update(
            10.0, Some(10.0), Some(0.0), &seq(true),
            Some(1), 3, 2, false, /*playing*/ false, /*stopped*/ true,
            /*next_clip_index*/ Some(2),
        );
        match action {
            TransitionAction::AdvanceClip { next_clip_index } => assert_eq!(next_clip_index, 2),
            _ => panic!("expected AdvanceClip{{2}}"),
        }
    }

    #[test]
    fn update_ends_track_when_prepicked_index_is_none() {
        let tm = TransitionManager::new();
        let action = tm.update(
            10.0, Some(10.0), Some(0.0), &seq(false),
            Some(2), 3, 3, false, false, true,
            None,
        );
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
}
