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
    /// The mood profile: one value per id of the mood vocabulary, in its
    /// order (the moods, then Energy). Always every id; an id with no stored
    /// row (and every id on ambiance) has neither layer. The value shown is
    /// `user ?? auto`.
    pub moods: Vec<MoodValue>,
    /// When the user last set a mood level or approved the entry in Review
    /// (Unix milliseconds as a string, like `created_at`); `None` until then.
    pub mood_reviewed_at: Option<String>,
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

/// The id of Energy, the one fixed entry of the mood vocabulary. It always
/// comes last, after the user's moods. A mood id is lowercase letters only,
/// so it can never be `energy+` / `energy-` (the analyzer's internal names).
pub const ENERGY_ID: &str = "energy";

/// One mood of the vocabulary: a lens every music track has a 0..1 value
/// for. The id is stored in `library_entry_moods.mood` and never changes once
/// created; the name is what the UI shows; the prompts are the text the
/// analyzer compares tracks with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodDef {
    pub id: String,
    pub name: String,
    pub prompts: Vec<String>,
}

/// Energy's prompts: Energy is scored two-sided, the mean similarity with
/// the positive prompts minus the mean similarity with the negative ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct EnergyPrompts {
    pub positive: Vec<String>,
    pub negative: Vec<String>,
}

/// The app's mood vocabulary, edited in Settings → Moods and stored as JSON
/// in the `mood_vocabulary` meta row: the moods in display order, plus the
/// fixed Energy entry. It is sent to the analyzer with every `/score`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodVocabulary {
    pub moods: Vec<MoodDef>,
    pub energy: EnergyPrompts,
}

/// Most moods a vocabulary may hold (the analyzer accepts as many).
pub const MAX_MOODS: usize = 32;
/// Most prompts per mood, and per side of Energy.
pub const MAX_PROMPTS: usize = 16;
/// Longest prompt, in characters.
pub const MAX_PROMPT_CHARS: usize = 300;
/// Longest mood id, in characters.
pub const MAX_MOOD_ID_CHARS: usize = 32;
/// Longest mood name, in characters.
pub const MAX_MOOD_NAME_CHARS: usize = 40;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Trims every prompt and drops the blank ones (a blank line in the editor).
fn clean_prompts(prompts: &[String]) -> Vec<String> {
    prompts.iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

fn check_prompts(what: &str, prompts: &[String]) -> Result<(), String> {
    if prompts.is_empty() {
        return Err(format!("{what} needs at least one prompt"));
    }
    if prompts.len() > MAX_PROMPTS {
        return Err(format!("{what} has more than {MAX_PROMPTS} prompts"));
    }
    if prompts.iter().any(|p| p.chars().count() > MAX_PROMPT_CHARS) {
        return Err(format!("{what} has a prompt longer than {MAX_PROMPT_CHARS} characters"));
    }
    Ok(())
}

impl MoodVocabulary {
    /// The defaults the database is seeded with (spec "Revision 2"). Once
    /// seeded, the app reads the vocabulary from the database, never from
    /// here.
    pub fn defaults() -> Self {
        let mood = |id: &str, name: &str, prompts: &[&str]| MoodDef {
            id: id.to_string(),
            name: name.to_string(),
            prompts: strings(prompts),
        };
        Self {
            moods: vec![
                mood("peaceful", "Peaceful", &[
                    "benign, pleasant, agreeable music with no threat",
                    "music that signals safety, whatever its tempo",
                    "friendly and harmless",
                ]),
                mood("sorrowful", "Sorrowful", &[
                    "reflective, somber, sorrowful music",
                    "sad, mournful, melancholic music",
                    "wistful, grieving, nostalgic music",
                ]),
                mood("wondrous", "Wondrous", &[
                    "music full of wonder and awe",
                    "sacred, divine, celestial music, a choir in a cathedral",
                    "a breathtaking vista, hopeful and vast, slow and soaring",
                ]),
                mood("joyful", "Joyful", &[
                    "joyful, glad music, any tempo",
                    "bright major-key happiness",
                    "contented and sunny",
                ]),
                mood("quirky", "Quirky", &[
                    "quirky, weird, offbeat music",
                    "groovy, funky, eccentric and strange",
                    "campy, oddball, mischievous tune",
                ]),
                mood("mysterious", "Mysterious", &[
                    "mysterious, unsettling, eerie music",
                    "strange, uncanny, otherworldly sounds",
                    "quiet suspense and looming dread, something is about to happen",
                ]),
                mood("grim", "Grim", &[
                    "horror",
                    "dark",
                    "heavy",
                ]),
                mood("heroic", "Heroic", &[
                    "heroic, triumphant, bold orchestral music",
                    "a rousing victory anthem, brass and choir soaring",
                    "adventurous, noble, uplifting epic music",
                ]),
            ],
            energy: EnergyPrompts {
                positive: strings(&[
                    "energetic, lively, exciting music",
                    "fast, agitated, intense music",
                    "busy, driving, stimulating music",
                ]),
                negative: strings(&[
                    "calm, relaxed, tranquil music",
                    "slow, sleepy, still music",
                    "soothing, gentle, peaceful music",
                ]),
            },
        }
    }

    /// Every id in profile order: the moods, then Energy.
    pub fn ids(&self) -> Vec<String> {
        self.moods.iter().map(|m| m.id.clone()).chain([ENERGY_ID.to_string()]).collect()
    }

    /// True if `id` is one of the moods or Energy.
    pub fn contains(&self, id: &str) -> bool {
        id == ENERGY_ID || self.moods.iter().any(|m| m.id == id)
    }

    /// What one id's score is computed from, for telling whether a
    /// calibration still matches: a mood's prompts, or Energy's prompts with
    /// `+ ` / `- ` in front of each side's. `None` for an unknown id.
    pub fn prompt_signature(&self, id: &str) -> Option<Vec<String>> {
        if id == ENERGY_ID {
            let pos = self.energy.positive.iter().map(|p| format!("+ {p}"));
            let neg = self.energy.negative.iter().map(|p| format!("- {p}"));
            return Some(pos.chain(neg).collect());
        }
        self.moods.iter().find(|m| m.id == id).map(|m| m.prompts.clone())
    }

    /// Trims ids, names and prompts, drops blank prompt lines, and checks
    /// the rules: 1 to `MAX_MOODS` moods; each id lowercase letters only,
    /// unique and not `energy`; each name non-empty and unique (ignoring
    /// case, and not "Energy"); at least one prompt per mood and per side of
    /// Energy.
    pub fn normalized(&self) -> Result<Self, String> {
        if self.moods.is_empty() {
            return Err("Keep at least one mood".to_string());
        }
        if self.moods.len() > MAX_MOODS {
            return Err(format!("At most {MAX_MOODS} moods"));
        }
        let mut moods = Vec::with_capacity(self.moods.len());
        let mut ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for m in &self.moods {
            let id = m.id.trim().to_string();
            if id.is_empty() || id.len() > MAX_MOOD_ID_CHARS || !id.bytes().all(|b| b.is_ascii_lowercase()) {
                return Err(format!(
                    "The mood id \"{id}\" must be 1 to {MAX_MOOD_ID_CHARS} lowercase letters (a-z)"
                ));
            }
            if id == ENERGY_ID {
                return Err("The id \"energy\" is kept for Energy".to_string());
            }
            if !ids.insert(id.clone()) {
                return Err(format!("The mood id \"{id}\" is used twice"));
            }
            let name = m.name.trim().to_string();
            if name.is_empty() || name.chars().count() > MAX_MOOD_NAME_CHARS {
                return Err(format!("The mood \"{id}\" needs a name of 1 to {MAX_MOOD_NAME_CHARS} characters"));
            }
            if name.eq_ignore_ascii_case(ENERGY_ID) || !names.insert(name.to_lowercase()) {
                return Err(format!("The name \"{name}\" is used twice"));
            }
            let prompts = clean_prompts(&m.prompts);
            check_prompts(&name, &prompts)?;
            moods.push(MoodDef { id, name, prompts });
        }
        let energy = EnergyPrompts {
            positive: clean_prompts(&self.energy.positive),
            negative: clean_prompts(&self.energy.negative),
        };
        check_prompts("Energy (energetic side)", &energy.positive)?;
        check_prompts("Energy (calm side)", &energy.negative)?;
        Ok(Self { moods, energy })
    }
}

/// When the frozen mood calibration was taken and from how many tracks, for
/// "Last calibrated: <date>, N tracks" in Settings → Moods.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodCalibrationInfo {
    /// Unix milliseconds as a string, like `created_at`.
    pub calibrated_at: String,
    pub tracks: u32,
}

/// The levels a user can set a mood to (a 1-5 rating).
pub const MOOD_LEVELS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// One mood of an entry's profile, both layers. The value shown is
/// `user ?? auto`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodValue {
    /// A mood id of the vocabulary, or `energy`.
    pub mood: String,
    /// The analyzer's suggestion, 0..1, from the frozen calibration. `None`
    /// until a run with a calibration scored the entry.
    pub auto: Option<f64>,
    /// The level the user set, one of `MOOD_LEVELS`; `None` means the
    /// suggestion shows.
    pub user: Option<f64>,
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

/// One entry in a Scry ranking (similar tracks or text search), with the
/// analyzer's similarity score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct ScryHit {
    pub entry_id: String,
    pub score: f64,
}

/// One entry's position on the Scry map. `x` and `y` are in [0, 1].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct ScryPoint {
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
    /// The mood vocabulary every entry's `moods` follows.
    pub mood_vocabulary: MoodVocabulary,
    /// The frozen mood calibration's date and size; `None` until a run took
    /// one (or after Recalibrate cleared it).
    pub mood_calibration: Option<MoodCalibrationInfo>,
    /// True when a calibration exists but some mood's prompts changed (or a
    /// mood was added) since it was taken: those moods are not scored until
    /// Recalibrate runs.
    pub moods_need_recalibration: bool,
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
            mood_vocabulary: MoodVocabulary::defaults(),
            mood_calibration: None,
            moods_need_recalibration: false,
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
    fn default_vocabulary_is_eight_moods_then_energy_and_valid() {
        let v = MoodVocabulary::defaults();
        assert_eq!(
            v.ids(),
            ["peaceful", "sorrowful", "wondrous", "joyful", "quirky", "mysterious", "grim", "heroic", "energy"]
        );
        assert_eq!(v.normalized().unwrap(), v);
        assert!(v.contains("energy") && v.contains("grim") && !v.contains("menacing"));
        assert_eq!(v.prompt_signature("joyful").unwrap().len(), 3);
        let energy = v.prompt_signature("energy").unwrap();
        assert_eq!(energy[0], "+ energetic, lively, exciting music");
        assert_eq!(energy[5], "- soothing, gentle, peaceful music");
        assert_eq!(v.prompt_signature("menacing"), None);
    }

    #[test]
    fn vocabulary_normalizes_and_rejects_bad_input() {
        let mut v = MoodVocabulary::defaults();
        v.moods[0].name = "  Calm  ".into();
        v.moods[0].prompts = vec!["  still water ".into(), "".into(), "   ".into()];
        let n = v.normalized().unwrap();
        assert_eq!(n.moods[0].name, "Calm");
        assert_eq!(n.moods[0].prompts, ["still water"]);

        let bad = |f: &dyn Fn(&mut MoodVocabulary)| {
            let mut v = MoodVocabulary::defaults();
            f(&mut v);
            v.normalized().unwrap_err()
        };
        bad(&|v| v.moods.clear());
        bad(&|v| v.moods[0].id = "Calm".into());
        bad(&|v| v.moods[0].id = "calm2".into());
        bad(&|v| v.moods[0].id = "energy".into());
        bad(&|v| v.moods[0].id = "grim".into());
        bad(&|v| v.moods[0].name = " ".into());
        bad(&|v| v.moods[0].name = "GRIM".into());
        bad(&|v| v.moods[0].name = "Energy".into());
        bad(&|v| v.moods[0].prompts = vec!["  ".into()]);
        bad(&|v| v.moods[0].prompts = vec!["x".repeat(MAX_PROMPT_CHARS + 1)]);
        bad(&|v| v.energy.negative.clear());
        bad(&|v| v.energy.positive = vec!["p".into(); MAX_PROMPTS + 1]);
    }

    #[test]
    fn vocabulary_json_is_camel_case_moods_and_energy() {
        let json = serde_json::to_value(MoodVocabulary::defaults()).unwrap();
        assert_eq!(json["moods"][3]["id"], "joyful");
        assert_eq!(json["moods"][3]["name"], "Joyful");
        assert_eq!(json["energy"]["negative"].as_array().unwrap().len(), 3);
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
