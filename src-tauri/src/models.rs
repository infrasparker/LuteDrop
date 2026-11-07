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
    /// The entry's tags, all set by hand.
    pub tags: Vec<String>,
    pub entry_type: EntryType,
    /// Analyzer state for this entry; `None` if it was never submitted.
    pub analysis: Option<EntryAnalysis>,
    /// The role shown (see `effective_role`): for music `role_user`, else
    /// Non-combat, so never `None`; for ambiance `None`.
    pub role: Option<Role>,
    /// The role the user set; `None` shows as Non-combat.
    pub role_user: Option<Role>,
    /// The mood profile: one value per id of the mood vocabulary, in its
    /// order (the moods, then Energy). Always every id; an id with no stored
    /// row (and every id on ambiance) has neither layer. The value shown is
    /// `user ?? auto`.
    pub moods: Vec<MoodValue>,
    /// When the user last set a mood level on the entry
    /// (Unix milliseconds as a string, like `created_at`); `None` until then.
    pub mood_reviewed_at: Option<String>,
    /// The file's `REPLAYGAIN_TRACK_GAIN` in dB; `None` if it has none or it
    /// was not read yet.
    pub rg_track_gain_db: Option<f64>,
    /// What "Even out music loudness" multiplies this entry's volume by on
    /// playback (0 < f ≤ 1; 1 = unchanged). See `replaygain.rs`.
    pub loudness_factor: f64,
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

/// The role a music entry shows: the user's, else Non-combat (the default;
/// there is no "no role" state). Ambiance has no role.
pub fn effective_role(entry_type: &EntryType, role_user: Option<Role>) -> Option<Role> {
    match entry_type {
        EntryType::Music => Some(role_user.unwrap_or(Role::NonCombat)),
        EntryType::Ambiance => None,
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
    /// The defaults the database is seeded with (since schema v13, each mood
    /// and each side of Energy is one prompt: a cluster of tag words; the
    /// set was settled track by track against a real library on 2026-11-06,
    /// see docs/superpowers/research/2026-11-05-feel-lab/AUDIT-moods-2026-11-06.md),
    /// and what Reset to defaults writes. Once seeded, the app reads the
    /// vocabulary from the database, never from here. Keep these identical
    /// to `MOODS` / `ENERGY_POS` / `ENERGY_NEG` in `tagger/tagger/moods.py`.
    pub fn defaults() -> Self {
        let mood = |id: &str, name: &str, prompts: &[&str]| MoodDef {
            id: id.to_string(),
            name: name.to_string(),
            prompts: strings(prompts),
        };
        Self {
            moods: vec![
                mood("peaceful", "Peaceful", &["relaxing, positive"]),
                mood("sorrowful", "Sorrowful", &["sad, melancholic, somber, funeral, lament, tragic, heartbroken music"]),
                mood("wondrous", "Wondrous", &["angelic, divine, sacred, celestial, awe, wonder, inspiring"]),
                mood("joyful", "Joyful", &["joyful, happy, cheerful, merry, bright, glad, festive, sunny, drinking song"]),
                mood("quirky", "Quirky", &["quirky, offbeat, groovy, funky, oddball, mischievous, funny, silly, goofy, zany music"]),
                mood("mysterious", "Mysterious", &["mysterious, enigmatic, puzzling, curious, intrigue, chimes, bells, celesta, music box, sparse isolated notes, high ringing"]),
                mood("grim", "Grim", &["ominous, dread, fearful, scary, horror, terror, sinister, eldritch, unnatural music"]),
                mood("heroic", "Heroic", &["heroic, triumphant, bold, rousing, victorious, adventurous, noble, uplifting, epic orchestral music"]),
                mood("battle", "Battle", &["combat, battle, warlike, fierce, clash, war drums, fantasy battle music"]),
            ],
            energy: EnergyPrompts {
                // One cluster of tag words per side; never the bare word
                // "fast", which the model reads against energy.
                positive: strings(&["energetic, exciting, intense, upbeat, powerful"]),
                negative: strings(&["slow, relaxing, calm"]),
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
/// "Calibrated against the reference set: ..." or "Calibrated against your
/// library: <date>, N tracks" in Settings → Moods.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodCalibrationInfo {
    /// Unix milliseconds as a string, like `created_at`.
    pub calibrated_at: String,
    pub tracks: u32,
    /// `"library"` (statistics from the user's library) or `"reference"`
    /// (the analyzer's fixed reference set).
    pub source: String,
    /// Id of the reference set when `source` is `"reference"`.
    pub reference_set: Option<String>,
}

/// What Reset to defaults (Settings → Moods) would delete: the stored mood
/// rows whose id is not a default mood, and how many of them hold a level
/// set by the user. Read from the rows themselves, so it is right even while
/// the stored vocabulary is unreadable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodResetImpact {
    /// The ids that would go, sorted.
    pub moods: Vec<String>,
    pub rows: u32,
    pub user_levels: u32,
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
    /// The user's note on this song and mood, a tuning target; it never
    /// changes the shown value. `None` when there is no flag.
    pub flag: Option<MoodFlag>,
}

/// What a flag says: the bar belongs in `band`, or should be `higher` or
/// `lower` than it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "snake_case")]
pub enum FlagKind {
    Band,
    Higher,
    Lower,
}

/// The bands a `band` flag can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "snake_case")]
pub enum FlagBand {
    Definitely,
    Understandable,
    Stretch,
    No,
}

/// Whether the band is exact (`in`) or a limit (`at_least`, `at_most`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "snake_case")]
pub enum FlagBound {
    In,
    AtLeast,
    AtMost,
}

/// Why a flag is stale. `Wording`: the mood's prompt lines changed since the
/// flag was set or last kept. `Calibration`: a higher / lower flag whose
/// mood has been recalibrated since, so the bar it was made at is on another
/// scale. A flag stale both ways reads as `Calibration`, the one that changes
/// how Tune reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "snake_case")]
pub enum StaleReason {
    Wording,
    Calibration,
}

/// A flag on one song and mood. `stale` is true when `stale_reason` is set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodFlag {
    pub kind: FlagKind,
    pub band: Option<FlagBand>,
    pub bound: FlagBound,
    /// The bar the song showed when flagged.
    pub bar_at_flag: Option<f64>,
    /// For a higher / lower flag, its mood's calibration when flagged or last
    /// kept, as "mean|sd" (see `mood_flags::calibration_mark`).
    pub calibrated_at: Option<String>,
    pub stale: bool,
    pub stale_reason: Option<StaleReason>,
}

/// What the one-off case-study import did: how many flags it wrote, the
/// tracks it could not find in the library, the moods in the file that are
/// not in the vocabulary, and how many (track, mood) labels it left out
/// because the case lists that mood as `settled` (a target let go of).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct CaseImportReport {
    pub imported: usize,
    pub unmatched: Vec<String>,
    pub skipped_moods: Vec<String>,
    pub settled_skipped: usize,
}

/// A song in a Tune review: the analyzer's content hash resolved to a
/// library entry. `album` is "Artist / Album" from the file's two parent
/// folders, as the Moods panel previews show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneSong {
    pub entry_id: String,
    pub name: String,
    pub album: String,
}

/// A flagged song's bar under the mood's current lines (`before`) and under
/// the reviewed lines (`after`), and whether its flag holds under them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneFlagOutcome {
    pub song: TuneSong,
    pub before: f64,
    pub after: f64,
    pub holds: bool,
    /// False for a flag made under older wording or calibration.
    pub fresh: bool,
    /// The flag Tune focused on (counted three times over in the search).
    pub focus: bool,
}

/// One of the songs a set of lines ranks highest, with its bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneTopSong {
    pub song: TuneSong,
    pub bar: f64,
}

/// An unflagged song whose bar moves the most between the current and the
/// reviewed lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneMovedSong {
    pub song: TuneSong,
    pub before: f64,
    pub after: f64,
}

/// How alike another mood's ranking is to this one's (rank correlation),
/// under the current and under the reviewed lines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneOverlap {
    pub mood_id: String,
    pub before: f64,
    pub after: f64,
}

/// Leave-one-out: how many flags still hold when each is left out of the
/// search in turn. `0 / 0` on the baseline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneLeaveOneOut {
    pub held: u32,
    pub total: u32,
}

/// A flag row as stored, whole: what Use's undo puts back for the direction
/// flags the accept turned into band flags (see `tune::accept_lines`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct MoodFlagRow {
    pub entry_id: String,
    pub mood: String,
    pub kind: FlagKind,
    pub band: Option<FlagBand>,
    pub bound: FlagBound,
    pub bar_at_flag: Option<f64>,
    pub calibrated_at: Option<String>,
    pub lines_sig: String,
    /// When the flag was set (Unix milliseconds as a string, like
    /// `created_at` elsewhere); `None` keeps the row's own (or now).
    #[serde(default)]
    pub created_at: Option<String>,
}

/// What removing a flag returns: the new state, and the row removed, whole,
/// so Undo can put it back exactly (see `restore_mood_flags`).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct ClearedFlag {
    pub state: AppState,
    pub removed: Option<MoodFlagRow>,
}

/// What accepting tuned lines returns: the new state, and the flag rows the
/// direction-flag ratchet changed, as they were before.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AcceptedLines {
    pub state: AppState,
    pub converted: Vec<MoodFlagRow>,
}

/// The Moods page's per-mood preferences for Tune: words never to suggest
/// (sent as `exclude` for Arrange and Suggest). Stored as JSON in the meta
/// key `mood_tune_prefs`, a map from mood id to this.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase", default)]
pub struct MoodTunePrefs {
    pub never_suggest: Vec<String>,
}

/// A suggested name word, with how closely its own ranking follows the lines'.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneNameIdea {
    pub word: String,
    pub rho: f64,
}

/// The review of one set of lines (the mood's current ones, a proposal, or a
/// split half). Lower `loss` is better; it includes `stability`, what the
/// lines cost for moving unflagged songs (0 for the current lines).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneReview {
    pub lines: Vec<String>,
    /// Where each line came from ("word bank", "phrase list", "Mysterious
    /// words", "blend" then its parts' sources, ...), aligned with `lines`.
    pub line_sources: Vec<Vec<String>>,
    pub loss: f64,
    pub stability: f64,
    pub flags: Vec<TuneFlagOutcome>,
    pub top: Vec<TuneTopSong>,
    pub top_before: Vec<TuneTopSong>,
    pub risers: Vec<TuneMovedSong>,
    pub fallers: Vec<TuneMovedSong>,
    /// How many songs reach 0.5 under the current and the reviewed lines.
    pub fires_before: u32,
    pub fires_after: u32,
    pub overlap: Vec<TuneOverlap>,
    pub loo: TuneLeaveOneOut,
    pub names: Vec<TuneNameIdea>,
    /// Fresh flags that do not hold under the reviewed lines.
    pub unsatisfied: Vec<TuneSong>,
    /// The songs that left the top list, with their rank in the whole new
    /// ranking (empty from an older analyzer).
    pub out_ranks: Vec<TuneOutRank>,
}

/// A song that left the top list, and where it ranks now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneOutRank {
    pub song: TuneSong,
    pub rank: u32,
    /// Its new bar, when the analyzer reports it.
    pub bar: Option<f64>,
}

/// A suggested split of the mood in two: `keep` stays the mood, `new` becomes
/// a new one, each with the flagged songs that go with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneSplitReview {
    pub keep: TuneReview,
    pub new: TuneReview,
    pub keep_songs: Vec<TuneSong>,
    pub new_songs: Vec<TuneSong>,
    /// Rank correlation between the two halves' rankings.
    pub rho: f64,
}

/// How Tune works: compare reviews the user's own versions of the lines as
/// written; arrange searches only the words the user typed and the current
/// lines; suggest also searches the word bank, other moods' words, the
/// phrase list and the analyzer's built-in vocabulary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "lowercase")]
pub enum TuneMode {
    Compare,
    Arrange,
    #[default]
    Suggest,
}

/// What Tune found for a mood: the current lines' review, the proposals
/// that beat it (best first), an optional split, and the flagged songs
/// left out. `lines_tuned_from` goes back with an accept, which is refused
/// if the mood's lines have changed since.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TuneResult {
    pub baseline: TuneReview,
    pub proposals: Vec<TuneReview>,
    pub split: Option<TuneSplitReview>,
    /// Flagged songs the analyzer has no embedding for.
    pub skipped: Vec<TuneSong>,
    pub lines_tuned_from: Vec<String>,
    /// Flags left out of the request: songs never analyzed (no hash), and
    /// flags that cannot be read or ask nothing of the bar.
    pub skipped_flags: usize,
    /// The song whose flag the search focused on, if any.
    pub focus_entry_id: Option<String>,
    /// The mode Tune ran in; in compare mode `proposals` are the user's
    /// versions, in order, whether or not they beat the current lines.
    pub mode: TuneMode,
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

/// Summary of the most recent analysis run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AutoTagReport {
    pub finished_at: Option<String>,
    pub analyzed: u32,
    pub failed: u32,
    pub unreachable: u32,
    pub error: Option<String>,
    /// Up to 5 "entry name: error" lines from entries that failed or could
    /// not be reached in this run.
    pub sample_errors: Vec<String>,
}

/// Emitted to the frontend while an analysis run is in progress.
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

/// The two ends of an Energy preview: the most and the least energetic
/// cached tracks under a pair of prompt sides, each best-first for its end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct EnergyPreview {
    pub most: Vec<ScryHit>,
    pub least: Vec<ScryHit>,
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
    /// The entry's loudness factor ("Even out music loudness"), 1 when off
    /// or not applicable. Playback volume = clip × library volume × this.
    pub loudness_factor: f64,
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
    /// True when the stored mood vocabulary is unreadable: `mood_vocabulary`
    /// then holds the defaults, and saving a new one is refused.
    pub mood_vocabulary_unreadable: bool,
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
            mood_vocabulary_unreadable: false,
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
    fn effective_role_is_user_then_non_combat() {
        use Role::{Combat, NonCombat};
        let music = EntryType::Music;
        assert_eq!(effective_role(&music, Some(Combat)), Some(Combat));
        assert_eq!(effective_role(&music, Some(NonCombat)), Some(NonCombat));
        assert_eq!(effective_role(&music, None), Some(NonCombat));
        // Ambiance has no role, whatever is stored.
        assert_eq!(effective_role(&EntryType::Ambiance, Some(Combat)), None);
        assert_eq!(effective_role(&EntryType::Ambiance, None), None);
    }

    #[test]
    fn default_vocabulary_is_nine_moods_then_energy_and_valid() {
        let v = MoodVocabulary::defaults();
        assert_eq!(
            v.ids(),
            ["peaceful", "sorrowful", "wondrous", "joyful", "quirky", "mysterious", "grim", "heroic", "battle", "energy"]
        );
        assert_eq!(v.normalized().unwrap(), v);
        assert!(v.contains("energy") && v.contains("grim") && !v.contains("menacing"));
        assert_eq!(v.prompt_signature("joyful").unwrap(), ["joyful, happy, cheerful, merry, bright, glad, festive, sunny, drinking song"]);
        let energy = v.prompt_signature("energy").unwrap();
        assert_eq!(energy, ["+ energetic, exciting, intense, upbeat, powerful", "- slow, relaxing, calm"]);
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
        assert_eq!(json["energy"]["negative"], serde_json::json!(["slow, relaxing, calm"]));
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
