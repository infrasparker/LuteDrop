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
