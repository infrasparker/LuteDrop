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
    /// The tags the entry shows: those the user voted yes on, plus the
    /// analyzer's tags the user has not voted on.
    pub tags: Vec<String>,
    pub entry_type: EntryType,
    /// The tags shown because of the analyzer (the user has not voted on
    /// them), each with the analyzer's score.
    pub auto_tags: Vec<AutoTag>,
    /// Tags the user voted no on. They are not shown, whatever the analyzer
    /// says.
    pub rejected_tags: Vec<String>,
    /// Analyzer state for this entry; `None` if it was never submitted.
    pub analysis: Option<EntryAnalysis>,
    /// The role shown: `role_user` if set, otherwise `role_auto`.
    pub role: Option<Role>,
    /// The analyzer's role, written by every analysis run.
    pub role_auto: Option<Role>,
    /// The analyzer's Combat probability behind `role_auto`.
    pub role_auto_score: Option<f64>,
    /// The user's role. When set it is shown instead of `role_auto`.
    pub role_user: Option<Role>,
    /// The analyzer's Bright ↔ Dark position as a z-score across the
    /// library's analyzed music; positive is Bright.
    pub mood_bright: Option<f64>,
    /// The user's Bright ↔ Dark step; when set it overrides `mood_bright`.
    pub mood_bright_user: Option<MoodStep>,
    /// The analyzer's Intimate ↔ Grand position as a z-score; positive is
    /// Grand.
    pub mood_grand: Option<f64>,
    /// The user's Intimate ↔ Grand step; when set it overrides `mood_grand`.
    pub mood_grand_user: Option<MoodStep>,
    /// The analyzer's Calm ↔ Energetic position as a z-score; positive is
    /// Energetic.
    pub mood_energy: Option<f64>,
    /// The user's Calm ↔ Energetic step; when set it overrides `mood_energy`.
    pub mood_energy_user: Option<MoodStep>,
    /// The analyzer's Sad score as a z-score across the library's analyzed
    /// music. Only meaningful on a track whose shown Bright ↔ Dark step is
    /// Dark.
    pub mood_sad: Option<f64>,
    /// The analyzer's Scary score as a z-score, like `mood_sad`.
    pub mood_scary: Option<f64>,
    /// The analyzer's dark kind: whichever of `mood_sad` and `mood_scary` is
    /// higher (Sad on a tie). `None` until both are scored. Derived when
    /// loading, not stored.
    pub dark_kind_auto: Option<DarkKind>,
    /// The user's dark kind; when set it overrides `dark_kind_auto`.
    pub dark_kind_user: Option<DarkKind>,
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

/// One of the three steps on a mood scale. `High` is the positive pole
/// (Bright on Bright ↔ Dark, Grand on Intimate ↔ Grand, Energetic on
/// Calm ↔ Energetic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum MoodStep {
    Low,
    Mid,
    High,
}

/// Which of the three mood scales a user step applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum MoodScale {
    /// Bright <-> Dark.
    Bright,
    /// Intimate <-> Grand.
    Grand,
    /// Calm <-> Energetic.
    Energy,
}

/// What kind of dark a Dark track is. Shown and filtered on only when the
/// track's shown Bright ↔ Dark step is Dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum DarkKind {
    /// Sorrow, grief, loss.
    Sad,
    /// Threat: dread or menace.
    Scary,
}

impl DarkKind {
    /// The value stored in `library_entries.dark_kind_user`.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            DarkKind::Sad => "sad",
            DarkKind::Scary => "scary",
        }
    }

    /// Parses a stored value; anything unrecognised is `None`.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "sad" => Some(DarkKind::Sad),
            "scary" => Some(DarkKind::Scary),
            _ => None,
        }
    }

    /// The analyzer's kind from the two z-scores: the higher one, Sad on a
    /// tie. `None` unless both are present.
    pub fn from_scores(sad: Option<f64>, scary: Option<f64>) -> Option<Self> {
        let (sad, scary) = (sad?, scary?);
        Some(if scary > sad { DarkKind::Scary } else { DarkKind::Sad })
    }
}

/// The analyzer's position on each mood scale for one entry, as z-scores
/// across the library's analyzed music. Positive is Bright, Grand and
/// Energetic; `sad` and `scary` are the two dark kinds' own z-scores.
/// Backend-only: the frontend reads these from `LibraryEntry`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoodZ {
    pub bright: f64,
    pub grand: f64,
    pub energy: f64,
    pub sad: f64,
    pub scary: f64,
}

impl MoodStep {
    /// The value stored in the `mood_*_user` columns.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            MoodStep::Low => "low",
            MoodStep::Mid => "mid",
            MoodStep::High => "high",
        }
    }

    /// Parses a stored value; anything unrecognised is `None`.
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "low" => Some(MoodStep::Low),
            "mid" => Some(MoodStep::Mid),
            "high" => Some(MoodStep::High),
            _ => None,
        }
    }
}

/// Which layer a shown value comes from: the user's, or the analyzer's.
/// Used by `TagScore::applied`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub enum TagSource {
    User,
    Auto,
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
            vocabulary: ["folk", "electronic", "playful"]
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
    /// Why the entry shows this tag: `User` when the user voted yes, `Auto`
    /// when only the analyzer put it there, `None` when it is not shown
    /// (absent, or the user voted no).
    pub applied: Option<TagSource>,
}

/// The gate score and per-tag scores for one entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct EntryTagScores {
    /// The Combat probability, with `tag` `"combat"`. Its `applied` is the
    /// layer the shown role comes from (`User` when the user set one, else
    /// `Auto` when the analyzer did), or `None` when the entry shows no role.
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
    /// True when a sound is loaded (playing, paused, or just ended). False
    /// while a Gap transition waits to start the next clip: `is_playing` is
    /// still true then, but `current_time` is 0 and `current_clip_id` is the
    /// clip that just finished, so there is nothing to show a position for.
    pub has_sound: bool,
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
    fn dark_kind_db_strings_round_trip() {
        for kind in [DarkKind::Sad, DarkKind::Scary] {
            assert_eq!(DarkKind::from_db_str(kind.as_db_str()), Some(kind));
        }
        assert_eq!([DarkKind::Sad.as_db_str(), DarkKind::Scary.as_db_str()], ["sad", "scary"]);
        assert_eq!(DarkKind::from_db_str("angry"), None);
    }

    #[test]
    fn dark_kind_auto_is_the_higher_score() {
        assert_eq!(DarkKind::from_scores(Some(0.4), Some(1.1)), Some(DarkKind::Scary));
        assert_eq!(DarkKind::from_scores(Some(-0.2), Some(-0.9)), Some(DarkKind::Sad));
        assert_eq!(DarkKind::from_scores(Some(0.5), Some(0.5)), Some(DarkKind::Sad));
        assert_eq!(DarkKind::from_scores(None, Some(1.0)), None);
        assert_eq!(DarkKind::from_scores(Some(1.0), None), None);
    }

    #[test]
    fn mood_step_db_strings_round_trip() {
        for step in [MoodStep::Low, MoodStep::Mid, MoodStep::High] {
            assert_eq!(MoodStep::from_db_str(step.as_db_str()), Some(step));
        }
        assert_eq!(
            [MoodStep::Low.as_db_str(), MoodStep::Mid.as_db_str(), MoodStep::High.as_db_str()],
            ["low", "mid", "high"]
        );
        assert_eq!(MoodStep::from_db_str(""), None);
    }

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
