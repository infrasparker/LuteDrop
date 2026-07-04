use std::time::{Duration, Instant};
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
            let should_cleanup = outgoing.started.elapsed()
                >= Duration::from_secs_f64(outgoing.duration + 0.5);
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
                // use half the playable duration (minimum 0.1s) to avoid triggering
                // crossfade immediately at clip start (the All's Well bug).
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
/// Moved from engine.rs.
///
/// - Sequence: advance to next; wrap to 0 if repeat, else None.
/// - Shuffle: random index, avoids repeating the same clip when count > 1.
/// - Seamless: always None (the orchestrator loops the single clip itself).
pub fn pick_next_clip_index(
    mode: &TrackMode,
    current_index: Option<usize>,
    clip_count: usize,
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
}
