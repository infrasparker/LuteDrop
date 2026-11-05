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
    /// Names of every tag on the entry, both user-applied and auto-applied.
    pub tags: Vec<String>,
    pub entry_type: EntryType,
    /// The auto-applied tags with the score that earned each one.
    pub auto_tags: Vec<AutoTag>,
    /// Analyzer state for this entry; `None` if it was never submitted.
    pub analysis: Option<EntryAnalysis>,
    /// Combat or non-combat; `None` when unset or when the user cleared it.
    pub role: Option<Role>,
    /// Who set `role`. `Some(User)` with `role: None` means the user cleared
    /// it, so the analyzer leaves it empty.
    pub role_source: Option<TagSource>,
    /// The analyzer's Combat probability, kept while `role_source` is auto.
    pub role_score: Option<f64>,
}

/// Whether a music entry is meant for combat scenes.
///
/// Two string forms: the database stores `"combat"` and `"non_combat"`
/// (`as_db_str` / `from_db_str`), while serde and the generated TypeScript use
/// `"combat"` and `"nonCombat"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Combat,
    NonCombat,
}

impl Role {
    /// The value stored in `library_entries.role`.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Role::Combat => "combat",
            Role::NonCombat => "non_combat",
        }
    }

    /// Parses a stored value; anything unrecognised is `None`.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "combat" => Some(Role::Combat),
            "non_combat" => Some(Role::NonCombat),
            _ => None,
        }
    }
}

/// Who put a tag on an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum TagSource {
    User,
    Auto,
}

impl TagSource {
    /// The value stored in `library_entry_tags.source`.
    pub fn as_str(&self) -> &'static str {
        match self {
            TagSource::User => "user",
            TagSource::Auto => "auto",
        }
    }

    /// Anything other than `"auto"` counts as a user tag.
    pub fn from_str(s: &str) -> Self {
        match s {
            "auto" => TagSource::Auto,
            _ => TagSource::User,
        }
    }
}

/// A tag applied by the analyzer, with the score that earned it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AutoTag {
    pub tag: String,
    pub score: f64,
}

/// Where an entry is in the analysis pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum AnalysisStatus {
    Pending,
    Embedded,
    Failed,
    UnreachablePath,
}

impl AnalysisStatus {
    /// The value stored in `entry_analysis.status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            AnalysisStatus::Pending => "pending",
            AnalysisStatus::Embedded => "embedded",
            AnalysisStatus::Failed => "failed",
            AnalysisStatus::UnreachablePath => "unreachable_path",
        }
    }

    /// Unknown values read back as `Pending`.
    pub fn from_str(s: &str) -> Self {
        match s {
            "embedded" => AnalysisStatus::Embedded,
            "failed" => AnalysisStatus::Failed,
            "unreachable_path" => AnalysisStatus::UnreachablePath,
            _ => AnalysisStatus::Pending,
        }
    }
}

/// Analysis state of one library entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct EntryAnalysis {
    pub status: AnalysisStatus,
    /// When the entry was last scored (milliseconds since the Unix epoch).
    pub scored_at: Option<String>,
    pub error: Option<String>,
}

/// Connection settings for the analyzer service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AutoTagSettings {
    pub enabled: bool,
    pub analyzer_url: String,
    pub token: String,
    /// Start of library file paths on this machine; swapped for `remote_prefix`
    /// when paths are sent to the analyzer.
    pub local_prefix: String,
    /// Where that same folder is mounted on the analyzer host.
    pub remote_prefix: String,
}

/// The tags the analyzer scores on every music entry. Older saved configs
/// also hold `scene`, `gateTag` and `negativeSceneTags`; serde ignores those
/// fields when loading, and a config with no `vocabulary` gets the default one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase", default)]
pub struct SceneConfig {
    /// Tags that can be auto-applied to music entries.
    pub vocabulary: Vec<String>,
}

impl Default for SceneConfig {
    fn default() -> Self {
        Self {
            vocabulary: [
                "brass", "folk", "choir", "synth", "bright", "dark", "heroic", "horror",
                "lively", "peaceful", "ominous", "majestic", "moody", "mischievous", "haunting",
                "somber", "slow", "driving", "frantic",
            ]
            .iter()
            .map(|w| w.to_string())
            .collect(),
        }
    }
}

/// How well the analyzer can predict one tag, from leave-one-out testing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TagQuality {
    pub tag: String,
    pub auc: Option<f64>,
    pub pos: u32,
    pub neg: u32,
    pub trusted: bool,
    pub threshold: Option<f64>,
}

/// Summary of the most recent auto-tagging run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AutoTagReport {
    pub finished_at: Option<String>,
    pub analyzed: u32,
    pub added: u32,
    pub removed: u32,
    pub failed: u32,
    pub unreachable: u32,
    pub gate: Option<TagQuality>,
    pub tags: Vec<TagQuality>,
    pub error: Option<String>,
    /// Up to 5 "entry name: error" lines from entries that failed or could
    /// not be reached in this run.
    pub sample_errors: Vec<String>,
}

/// Emitted to the frontend while an auto-tagging run is in progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AutoTagProgress {
    /// One of `"embedding"`, `"scoring"`, `"done"`, `"error"`.
    pub phase: String,
    pub processed: u32,
    pub total: u32,
    pub message: Option<String>,
}

/// One entry in a Vibe ranking (similar tracks or text search), with the
/// analyzer's similarity score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct VibeHit {
    pub entry_id: String,
    pub score: f64,
}

/// One entry's position on the Vibe map. `x` and `y` are in [0, 1].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct VibePoint {
    pub entry_id: String,
    pub x: f64,
    pub y: f64,
}

/// One tag's score for one entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TagScore {
    pub tag: String,
    pub score: f64,
    pub threshold: Option<f64>,
    pub trusted: bool,
    /// Who currently has this tag on the entry, if anyone.
    pub applied: Option<TagSource>,
}

/// The gate score and per-tag scores for one entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct EntryTagScores {
    /// The Combat probability, with `tag` `"combat"`. Its `applied` is the
    /// source of the entry's role, or `None` when the entry has no role.
    pub gate: TagScore,
    pub tags: Vec<TagScore>,
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

/// How closely a file's name matched a missing entry; the dialog shows it as a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum MatchKind {
    Exact,
    Contains,
    Similar,
}

/// One file the "Fix missing files" dialog offers as a replacement for a missing entry.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct RelinkCandidate {
    pub file_path: String,
    /// Path relative to the search folder, shown in the candidate list.
    pub relative_path: String,
    pub kind: MatchKind,
    pub score: f64,
    /// Name of another library entry already using this file, if any.
    pub used_by_entry_name: Option<String>,
}

/// A missing library entry together with its candidate replacement files.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MissingEntryCandidates {
    pub entry: LibraryEntry,
    pub candidates: Vec<RelinkCandidate>,
}

/// Everything the "Fix missing files" dialog needs after a scan.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct FixMissingReport {
    pub search_root: String,
    pub search_root_exists: bool,
    pub files_scanned: u32,
    pub entries: Vec<MissingEntryCandidates>,
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
            // First clip mirrors the real reorder payload: TrackClipList.vue
            // sends FULL Clip objects, so ClipRef must tolerate the extra
            // fields (locks out a future `deny_unknown_fields` regression).
            "clips": [
                { "id": "c1", "name": "Rain", "filePath": "/z/rain.ogg", "volume": 0.8,
                  "libraryEntryId": "le1", "libraryVolume": 1.0,
                  "trimStart": null, "trimEnd": null },
                { "id": "c2" }
            ]
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

/// What the Broadcast dialog shows: the saved settings, the links listeners
/// use, how many are connected, and why the last attempt to turn it on
/// failed (if it did).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct BroadcastStatus {
    /// Whether the stream is running right now.
    pub enabled: bool,
    /// TCP port the stream server listens on.
    pub port: u16,
    /// Secret part of the listener link.
    pub key: String,
    /// User-entered public address (e.g. a reverse-proxy URL); empty if unset.
    pub public_base: String,
    /// Link for listeners on the same network.
    pub lan_url: String,
    /// Link for listeners elsewhere, built from `public_base`; `None` if unset.
    pub public_url: Option<String>,
    /// Listeners connected right now (0 when off).
    pub listener_count: u32,
    /// Why turning the stream on just failed; `None` otherwise.
    pub error: Option<String>,
}
