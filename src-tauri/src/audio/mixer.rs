use std::collections::HashMap;

/// Mute intent, tracked separately from the actual node volumes. Each mute level
/// maps to a distinct miniaudio node (master → engine, track → group, clip →
/// sound), so the engine multiplies them automatically; we only need to set the
/// affected node's volume to 0 (or restore it) on toggle.
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

    pub fn track_muted(&self, track_id: &str) -> bool {
        self.tracks.get(track_id).copied().unwrap_or(false)
    }

    pub fn clip_muted(&self, clip_id: &str) -> bool {
        self.clips.get(clip_id).copied().unwrap_or(false)
    }

    pub fn set_track_muted(&mut self, track_id: &str, muted: bool) {
        self.tracks.insert(track_id.to_string(), muted);
    }

    pub fn set_clip_muted(&mut self, clip_id: &str, muted: bool) {
        self.clips.insert(clip_id.to_string(), muted);
    }
}

/// Linear volume for a clip's sound node: clip gain × library gain, or 0 if the
/// clip itself is muted. Master and track mute/volume are applied by the engine
/// and group nodes respectively, so they are intentionally not folded in here.
pub fn clip_sound_volume(clip: f64, library: f64, clip_muted: bool) -> f32 {
    if clip_muted {
        0.0
    } else {
        (clip * library).clamp(0.0, 1.0) as f32
    }
}

/// Linear volume for a track's group node: the track gain, or 0 if muted.
pub fn track_group_volume(track: f64, track_muted: bool) -> f32 {
    if track_muted {
        0.0
    } else {
        track.clamp(0.0, 1.0) as f32
    }
}
