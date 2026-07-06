use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum EntryType {
    Music,
    Ambiance,
}

impl EntryType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "ambiance" => EntryType::Ambiance,
            _ => EntryType::Music,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub id: String,
    pub file_path: String,
    pub content_hash: Option<String>,
    pub name: String,
    pub duration: Option<f64>,
    pub sample_rate: Option<u32>,
    pub file_size: Option<u64>,
    pub volume: f64,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub loop_start: Option<f64>,
    pub loop_end: Option<f64>,
    pub created_at: String,
    pub tags: Vec<String>,
    pub entry_type: EntryType,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub volume: f64,
    pub library_entry_id: String,
    pub library_volume: f64,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrackTransition {
    Gapless,
    Gap { min: f64, max: f64 },
    Crossfade,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrackMode {
    Seamless {
        #[serde(rename = "loopStart")]
        loop_start: f64,
        #[serde(rename = "loopEnd")]
        loop_end: f64,
    },
    Sequence {
        repeat: bool,
        transition: TrackTransition,
    },
    Shuffle {
        repeat: bool,
        transition: TrackTransition,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub clips: Vec<Clip>,
    pub volume: f64,
    pub mode: TrackMode,
    pub fade_in: f64,
    pub fade_out: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
pub struct Comp {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub comps: Vec<Comp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub groups: Vec<Group>,
    pub library_entries: Vec<LibraryEntry>,
    pub tags: Vec<String>,
    pub configurations: Vec<Configuration>,
    pub active_configuration_id: String,
    pub missing_entry_ids: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            library_entries: Vec::new(),
            tags: Vec::new(),
            configurations: Vec::new(),
            active_configuration_id: String::new(),
            missing_entry_ids: Vec::new(),
        }
    }
}

/// Sent from Rust to the frontend when track playback state changes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TrackPlaybackState {
    pub track_id: String,
    pub is_playing: bool,
    pub is_paused: bool,
    pub current_clip_id: Option<String>,
    pub current_time: f64,
}

/// One location (clip) where a library entry is referenced.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntryUsage {
    pub clip_id: String,
    pub comp_name: String,
    pub track_name: String,
}

/// Emitted to the frontend during a background library import. The same
/// payload is used for the `library-import-progress` (per-file tick) and
/// `library-import-complete` (final) events; on completion `processed == total`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct BulkImportProgress {
    pub processed: u32,
    pub imported: u32,
    pub skipped: u32,
    pub total: u32,
}

/// Partial-update payload for `update_track`. Every field is optional; only the
/// present ones are applied. Deserialized directly from the frontend's
/// `updates` object (camelCase), replacing hand-parsed `serde_json::Value`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackUpdates {
    pub name: Option<String>,
    pub volume: Option<f64>,
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
    pub mode: Option<TrackModeUpdate>,
    pub clips: Option<Vec<ClipRef>>,
}

/// Mode sub-object of `TrackUpdates`. `type` + optional repeat/transition.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackModeUpdate {
    pub r#type: String,
    #[serde(default)]
    pub repeat: Option<bool>,
    #[serde(default)]
    pub transition: Option<TransitionUpdate>,
}

/// Transition sub-object: `type` plus optional gap bounds.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionUpdate {
    pub r#type: Option<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// A clip reference in a reorder payload — only the id is used.
#[derive(Debug, Deserialize)]
pub struct ClipRef {
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_updates_deserializes_camel_case_and_defaults_absent() {
        let json = serde_json::json!({
            "name": "New",
            "fadeIn": 1.0,
            "mode": { "type": "sequence", "repeat": true,
                      "transition": { "type": "gap", "min": 2.0, "max": 5.0 } },
            "clips": [{ "id": "c1" }, { "id": "c2" }]
        });
        let u: TrackUpdates = serde_json::from_value(json).unwrap();
        assert_eq!(u.name.as_deref(), Some("New"));
        assert_eq!(u.fade_in, Some(1.0));
        assert!(u.volume.is_none() && u.fade_out.is_none());
        let m = u.mode.unwrap();
        assert_eq!(m.r#type, "sequence");
        assert_eq!(m.repeat, Some(true));
        assert_eq!(m.transition.unwrap().min, Some(2.0));
        assert_eq!(u.clips.unwrap().iter().map(|c| c.id.clone()).collect::<Vec<_>>(), ["c1", "c2"]);

        // A bare empty object is a valid no-op update.
        let empty: TrackUpdates = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(empty.name.is_none() && empty.mode.is_none() && empty.clips.is_none());
    }
}
