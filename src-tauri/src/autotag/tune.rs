//! Mood auto-tune, the app's side: builds the analyzer's `/tune` request
//! from a mood's flags, turns the analyzer's review (content hashes) into
//! songs the UI can show, and accepts a proposal or a split.
//!
//! The analyzer does the search; it only knows songs by content hash and
//! bars by the calibration the app sends. So the app decides here what each
//! flag asks of a bar (a floor, a ceiling or both), how much it counts, and
//! which mapping turns raw scores into bars.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::client::AnalyzerClient;
use super::plan::{midpoint_z, slope, CalibrationSource};
use super::scry::MusicIndex;
use crate::database::mood_flags::{self, lines_sig, StoredFlag};
use crate::database::{library, settings};
use crate::models::{
    FlagBand, FlagBound, FlagKind, MoodDef, MoodVocabulary, TuneFlagOutcome, TuneLeaveOneOut, TuneMovedSong,
    TuneNameIdea, TuneOverlap, TuneResult, TuneReview, TuneSong, TuneSplitReview, TuneTopSong,
};

// --- The `/tune` HTTP contract (snake_case on the wire) ---

/// Which way a flag pulls, for the analyzer's split search: `high` flags are
/// the songs the mood is about, `low` ones are songs it must leave out, and
/// `mid` ones sit inside a band with both a floor and a ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagSide {
    High,
    Low,
    Mid,
}

/// One flag as the analyzer grades it: the bar `hash` should reach (`min`)
/// and stay under (`max`), either of which may be absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuneFlag {
    pub hash: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub weight: f64,
    pub fresh: bool,
    pub side: FlagSide,
}

/// The bar mapping the app uses, so the analyzer grades what the user sees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuneCalibration {
    pub source: CalibrationSource,
    pub midpoint_z: f64,
    pub slope: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuneRequest {
    pub mood_id: String,
    pub name: String,
    pub current_lines: Vec<String>,
    pub flags: Vec<TuneFlag>,
    /// Every music content hash (what `/score` scores).
    pub targets: Vec<String>,
    /// Words and lines the search may use, besides the moods' own lines.
    pub bank: Vec<String>,
    /// Every other mood's current lines, by id, for the overlap notes.
    pub other_moods: BTreeMap<String, Vec<String>>,
    pub calibration: TuneCalibration,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneFlagResult {
    pub hash: String,
    pub before: f64,
    pub after: f64,
    pub holds: bool,
    pub fresh: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneBar {
    pub hash: String,
    pub bar: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneMover {
    pub hash: String,
    pub before: f64,
    pub after: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneOverlapRaw {
    pub mood_id: String,
    pub before: f64,
    pub after: f64,
}

/// One review report, as the analyzer's `tune.build_report` writes it.
// `loo` and `names` reuse the UI types: their fields are single words, so
// the wire (snake_case) and the UI (camelCase) spell them the same.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneReport {
    pub lines: Vec<String>,
    pub loss: f64,
    pub flags: Vec<TuneFlagResult>,
    pub top: Vec<TuneBar>,
    pub top_before: Vec<TuneBar>,
    pub risers: Vec<TuneMover>,
    pub fallers: Vec<TuneMover>,
    pub fires_before: u32,
    pub fires_after: u32,
    pub overlap: Vec<TuneOverlapRaw>,
    pub loo: TuneLeaveOneOut,
    pub names: Vec<TuneNameIdea>,
    pub unsatisfied: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneSplit {
    pub keep: TuneReport,
    pub new: TuneReport,
    pub keep_hashes: Vec<String>,
    pub new_hashes: Vec<String>,
    pub rho: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneResponse {
    pub baseline: TuneReport,
    /// Only combinations that beat the baseline, best first.
    pub proposals: Vec<TuneReport>,
    pub split: Option<TuneSplit>,
    /// Flag hashes the analyzer has no embedding for.
    pub skipped: Vec<String>,
}

// --- Building the request ---

/// A direction flag aims this far past the bar it was made at.
const DIRECTION_MARGIN: f64 = 0.1;
/// How much a stale flag (older wording or calibration) counts.
const STALE_WEIGHT: f64 = 0.5;

/// Where a band starts. No has no floor.
fn band_floor(band: FlagBand) -> Option<f64> {
    match band {
        FlagBand::Definitely => Some(0.9),
        FlagBand::Understandable => Some(0.75),
        FlagBand::Stretch => Some(0.5),
        FlagBand::No => None,
    }
}

/// Where a band ends. Definitely has no ceiling.
fn band_ceiling(band: FlagBand) -> Option<f64> {
    match band {
        FlagBand::Definitely => None,
        FlagBand::Understandable => Some(0.9),
        FlagBand::Stretch => Some(0.75),
        FlagBand::No => Some(0.5),
    }
}

/// What one flag asks of the bar: (min, max, side). A band flag reads its
/// bound: `at_least` keeps only the band's floor (side high), `at_most` only
/// its ceiling (side low), `in` both (side mid; but No, which has no floor,
/// reads low, and Definitely, which has no ceiling, reads high). A direction
/// flag aims `DIRECTION_MARGIN` past `bar`, within 0..1. `None` for a flag
/// that cannot be read (a band flag with no band, a direction flag with no
/// bar).
fn flag_target(flag: &StoredFlag, bar: Option<f64>) -> Option<(Option<f64>, Option<f64>, FlagSide)> {
    match flag.kind {
        FlagKind::Higher => bar.map(|b| (Some((b + DIRECTION_MARGIN).min(1.0)), None, FlagSide::High)),
        FlagKind::Lower => bar.map(|b| (None, Some((b - DIRECTION_MARGIN).max(0.0)), FlagSide::Low)),
        FlagKind::Band => {
            let band = flag.band?;
            Some(match flag.bound {
                FlagBound::AtLeast => (band_floor(band), None, FlagSide::High),
                FlagBound::AtMost => (None, band_ceiling(band), FlagSide::Low),
                FlagBound::In => {
                    let side = match band {
                        FlagBand::No => FlagSide::Low,
                        FlagBand::Definitely => FlagSide::High,
                        FlagBand::Understandable | FlagBand::Stretch => FlagSide::Mid,
                    };
                    (band_floor(band), band_ceiling(band), side)
                }
            })
        }
    }
}

/// The band a hand-set level stands for: 1 → Definitely, 0.75 →
/// Understandable, 0.5 → Stretch, 0 or 0.25 → No.
fn level_band(level: f64) -> FlagBand {
    if level >= 1.0 {
        FlagBand::Definitely
    } else if level >= 0.75 {
        FlagBand::Understandable
    } else if level >= 0.5 {
        FlagBand::Stretch
    } else {
        FlagBand::No
    }
}

/// Builds the `/tune` request for one mood, and counts the flags left out
/// because their song was never analyzed (no content hash).
///
/// The flags are the mood's stored flags plus, for each song with a hand-set
/// level and no flag, a fresh band flag for that level (`level_band`, with
/// the bound a flag set in the app gets). See `flag_target` for what each
/// asks of the bar. A flag made under other lines, or a direction flag made
/// under another calibration, is stale: it counts `STALE_WEIGHT`, and a
/// stale direction flag aims past the song's current bar instead of the bar
/// it was made at. `extra_words` (typed in the Tune dialog) join `bank`.
pub fn build_tune_request(
    conn: &Connection,
    mood_id: &str,
    extra_words: &[String],
    bank: &[String],
) -> Result<(TuneRequest, usize), String> {
    let vocabulary = settings::load_mood_vocabulary(conn)?;
    let mood = vocabulary
        .moods
        .iter()
        .find(|m| m.id == mood_id)
        .ok_or_else(|| format!("There is no mood \"{mood_id}\" to tune"))?;
    let calibration = settings::load_mood_calibration(conn)?
        .ok_or("Run an analysis first so the moods are calibrated")?;
    let current_sig = lines_sig(&mood.prompts);

    let mut flags = mood_flags::flags_for_mood(conn, mood_id)?;
    let flagged: HashSet<String> = flags.iter().map(|f| f.entry_id.clone()).collect();
    for (entry_id, level) in user_levels(conn, mood_id)? {
        if flagged.contains(&entry_id) {
            continue;
        }
        let band = level_band(level);
        flags.push(StoredFlag {
            entry_id,
            mood: mood_id.to_string(),
            kind: FlagKind::Band,
            band: Some(band),
            bound: mood_flags::app_bound(band),
            bar_at_flag: None,
            calibrated_at: None,
            lines_sig: current_sig.clone(),
        });
    }
    if flags.is_empty() {
        return Err("Flag a few songs on this mood first: Tune needs to know which songs belong".to_string());
    }

    let mut out = Vec::new();
    let mut skipped = 0;
    for f in &flags {
        let (hash, bar_now) = hash_and_bar(conn, &f.entry_id, mood_id)?;
        let Some(hash) = hash else {
            skipped += 1;
            continue;
        };
        let direction = matches!(f.kind, FlagKind::Higher | FlagKind::Lower);
        let stale = f.lines_sig != current_sig
            || (direction && f.calibrated_at.as_deref() != Some(calibration.calibrated_at.as_str()));
        let bar = if stale { bar_now.or(f.bar_at_flag) } else { f.bar_at_flag };
        // A flag that cannot be read (see `flag_target`) is not sent; the
        // flag commands never store one.
        let Some((min, max, side)) = flag_target(f, bar) else { continue };
        out.push(TuneFlag {
            hash,
            min,
            max,
            weight: if stale { STALE_WEIGHT } else { 1.0 },
            fresh: !stale,
            side,
        });
    }
    if out.is_empty() {
        return Err("None of the songs flagged on this mood have been analyzed yet".to_string());
    }

    // Typed words after the bank, trimmed, blanks and repeats dropped.
    let mut seen = HashSet::new();
    let bank: Vec<String> = bank
        .iter()
        .chain(extra_words)
        .map(|w| w.trim().to_string())
        .filter(|w| !w.is_empty() && seen.insert(w.clone()))
        .collect();

    let music = MusicIndex::read(conn)?;
    let request = TuneRequest {
        mood_id: mood.id.clone(),
        name: mood.name.clone(),
        current_lines: mood.prompts.clone(),
        flags: out,
        targets: music.hashes.clone(),
        bank,
        other_moods: vocabulary
            .moods
            .iter()
            .filter(|m| m.id != mood_id)
            .map(|m| (m.id.clone(), m.prompts.clone()))
            .collect(),
        calibration: TuneCalibration {
            source: calibration.source,
            midpoint_z: midpoint_z(mood_id, calibration.source),
            slope: slope(mood_id, calibration.source),
        },
    };
    Ok((request, skipped))
}

/// Songs with a hand-set level for the mood, by entry id.
fn user_levels(conn: &Connection, mood: &str) -> Result<Vec<(String, f64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT library_entry_id, user_level FROM library_entry_moods
             WHERE mood = ?1 AND user_level IS NOT NULL ORDER BY library_entry_id",
        )
        .map_err(|e| format!("Failed to prepare level query: {e}"))?;
    let rows = stmt
        .query_map(params![mood], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("Failed to query levels: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("Failed to read levels: {e}"))
}

/// An entry's content hash and its current bar (the analyzer's value) for a mood.
fn hash_and_bar(conn: &Connection, entry_id: &str, mood: &str) -> Result<(Option<String>, Option<f64>), String> {
    conn.query_row(
        "SELECT e.content_hash, m.auto_score FROM library_entries e
         LEFT JOIN library_entry_moods m ON m.library_entry_id = e.id AND m.mood = ?2
         WHERE e.id = ?1",
        params![entry_id, mood],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map(Option::unwrap_or_default)
    .map_err(|e| format!("Failed to read a flagged song: {e}"))
}

// --- Running Tune ---

/// "Artist / Album" from a file's two parent folders (as the Moods panel
/// shows it), ignoring a drive letter.
fn album_of(path: &str) -> String {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && (p.len() != 2 || !p.ends_with(':')))
        .collect();
    let n = parts.len();
    [n.checked_sub(3), n.checked_sub(2)]
        .into_iter()
        .flatten()
        .map(|i| parts[i])
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Content hash → the song to show for it. Two entries can share a hash (the
/// same file imported twice): a hash resolves to the entry flagged (or
/// levelled) on the mood if there is one, otherwise to the first music entry
/// by name, as Scry does.
struct Songs(HashMap<String, TuneSong>);

impl Songs {
    fn read(conn: &Connection, mood_id: &str) -> Result<Self, String> {
        let mut stmt = conn
            .prepare("SELECT id, name, file_path, content_hash FROM library_entries WHERE content_hash IS NOT NULL")
            .map_err(|e| format!("Failed to prepare entry query: {e}"))?;
        let entries: HashMap<String, (String, TuneSong)> = stmt
            .query_map([], |r| {
                let (entry_id, name, path, hash): (String, String, String, String) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?);
                Ok((entry_id.clone(), (hash, TuneSong { entry_id, name, album: album_of(&path) })))
            })
            .and_then(|rows| rows.collect::<Result<_, _>>())
            .map_err(|e| format!("Failed to read entries: {e}"))?;
        let music = MusicIndex::read(conn)?;
        let first_music = music.hashes.iter().filter_map(|h| music.entries(h).first().cloned());
        let flagged = mood_flags::flags_for_mood(conn, mood_id)?.into_iter().map(|f| f.entry_id);
        let levelled = user_levels(conn, mood_id)?.into_iter().map(|(id, _)| id);
        // Flagged and levelled entries come last, so they win.
        let mut songs = HashMap::new();
        for id in first_music.chain(levelled).chain(flagged) {
            if let Some((hash, song)) = entries.get(&id) {
                songs.insert(hash.clone(), song.clone());
            }
        }
        Ok(Songs(songs))
    }

    fn song(&self, hash: &str) -> Option<TuneSong> {
        self.0.get(hash).cloned()
    }

    /// The songs for `hashes`; a hash no entry has any more is left out.
    fn all(&self, hashes: &[String]) -> Vec<TuneSong> {
        hashes.iter().filter_map(|h| self.song(h)).collect()
    }

    fn top(&self, bars: Vec<TuneBar>) -> Vec<TuneTopSong> {
        bars.into_iter()
            .filter_map(|b| Some(TuneTopSong { song: self.song(&b.hash)?, bar: b.bar }))
            .collect()
    }

    fn moved(&self, movers: Vec<TuneMover>) -> Vec<TuneMovedSong> {
        movers
            .into_iter()
            .filter_map(|m| Some(TuneMovedSong { song: self.song(&m.hash)?, before: m.before, after: m.after }))
            .collect()
    }

    fn review(&self, r: TuneReport) -> TuneReview {
        TuneReview {
            flags: r
                .flags
                .into_iter()
                .filter_map(|f| {
                    Some(TuneFlagOutcome {
                        song: self.song(&f.hash)?,
                        before: f.before,
                        after: f.after,
                        holds: f.holds,
                        fresh: f.fresh,
                    })
                })
                .collect(),
            top: self.top(r.top),
            top_before: self.top(r.top_before),
            risers: self.moved(r.risers),
            fallers: self.moved(r.fallers),
            overlap: r
                .overlap
                .into_iter()
                .map(|o| TuneOverlap { mood_id: o.mood_id, before: o.before, after: o.after })
                .collect(),
            unsatisfied: self.all(&r.unsatisfied),
            lines: r.lines,
            loss: r.loss,
            fires_before: r.fires_before,
            fires_after: r.fires_after,
            loo: r.loo,
            names: r.names,
        }
    }
}

/// Runs Tune for one mood: builds the request, asks the analyzer (which can
/// take tens of seconds), and resolves every hash in its review to a song.
/// Saves nothing. The database lock is held only to read, never during the
/// analyzer call.
pub fn tune(
    db: &Mutex<Connection>,
    client: &dyn AnalyzerClient,
    mood_id: &str,
    extra_words: &[String],
    bank: &[String],
) -> Result<TuneResult, String> {
    let (request, skipped_flags, songs) = {
        let conn = db.lock().map_err(|e| format!("Database lock poisoned: {e}"))?;
        let (request, skipped) = build_tune_request(&conn, mood_id, extra_words, bank)?;
        (request, skipped, Songs::read(&conn, mood_id)?)
    };
    let response = client.tune(&request)?;
    Ok(TuneResult {
        baseline: songs.review(response.baseline),
        proposals: response.proposals.into_iter().map(|r| songs.review(r)).collect(),
        split: response.split.map(|s| TuneSplitReview {
            keep_songs: songs.all(&s.keep_hashes),
            new_songs: songs.all(&s.new_hashes),
            keep: songs.review(s.keep),
            new: songs.review(s.new),
            rho: s.rho,
        }),
        skipped: songs.all(&response.skipped),
        lines_tuned_from: request.current_lines,
        skipped_flags,
    })
}

// --- Accepting ---

/// The vocabulary and the index of `mood_id` in it, if the mood's lines are
/// still `lines_tuned_from`. Refused if the mood is gone or its lines changed
/// since Tune ran: the review was made against those lines.
fn tuned_mood(conn: &Connection, mood_id: &str, lines_tuned_from: &[String]) -> Result<(MoodVocabulary, usize), String> {
    let vocabulary = settings::load_mood_vocabulary(conn)?;
    let i = vocabulary
        .moods
        .iter()
        .position(|m| m.id == mood_id)
        .ok_or_else(|| format!("The mood \"{mood_id}\" no longer exists"))?;
    if vocabulary.moods[i].prompts != lines_tuned_from {
        return Err("This mood's lines have changed since Tune ran; run Tune again".to_string());
    }
    Ok((vocabulary, i))
}

/// Accepts a proposal: the mood's lines become `lines`, saved the way an
/// edit in Settings → Moods is (`library::save_mood_vocabulary`), so the
/// mood then needs a recalibration just as after a manual edit.
pub fn accept_lines(conn: &Connection, mood_id: &str, lines_tuned_from: &[String], lines: &[String]) -> Result<(), String> {
    let (mut vocabulary, i) = tuned_mood(conn, mood_id, lines_tuned_from)?;
    vocabulary.moods[i].prompts = lines.to_vec();
    library::save_mood_vocabulary(conn, &vocabulary).map(|_| ())
}

/// Accepts a split, in one transaction: the mood keeps `keep_lines`, a new
/// mood is added after the others, the listed songs' flags move to it, and
/// the mood's "not this mood" flags are copied to it (they hold for both
/// halves). The vocabulary is saved as `accept_lines` saves it.
#[allow(clippy::too_many_arguments)]
pub fn accept_split(
    conn: &Connection,
    mood_id: &str,
    lines_tuned_from: &[String],
    keep_lines: &[String],
    new_id: &str,
    new_name: &str,
    new_lines: &[String],
    move_entry_ids: &[String],
) -> Result<(), String> {
    let (mut vocabulary, i) = tuned_mood(conn, mood_id, lines_tuned_from)?;
    vocabulary.moods[i].prompts = keep_lines.to_vec();
    vocabulary.moods.push(MoodDef {
        id: new_id.to_string(),
        name: new_name.to_string(),
        prompts: new_lines.to_vec(),
    });
    // Dropped without a commit (any error below), the transaction rolls back.
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin split transaction: {e}"))?;
    let saved = library::write_mood_vocabulary(conn, &vocabulary)?;
    // The saved id is the normalized (trimmed) one.
    let new_id = &saved.moods.last().ok_or("The split mood was not saved")?.id;
    mood_flags::move_flags(conn, mood_id, new_id, move_entry_ids)?;
    mood_flags::copy_low_flags(conn, mood_id, new_id)?;
    tx.commit().map_err(|e| format!("Failed to commit split transaction: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotag::plan::{needs_recalibration, reference_slope, CalibrationSource, MoodCalibration, MoodStat};
    use crate::autotag::tests::{add, setup, FakeAnalyzer};
    use crate::database::mood_flags::{flags_for_mood, lines_sig, set_flag};
    use crate::database::{library, settings};
    use crate::models::{FlagBand, FlagBound, FlagKind};
    use rusqlite::{params, Connection};
    use std::sync::Mutex;

    const MOOD: &str = "peaceful";

    fn calibrate(db: &Mutex<Connection>, source: CalibrationSource) {
        let conn = db.lock().unwrap();
        let vocabulary = settings::load_mood_vocabulary(&conn).unwrap();
        // Covers every id with the current prompts, so nothing needs recalibrating yet.
        let ids = vocabulary.ids();
        settings::save_mood_calibration(
            &conn,
            &MoodCalibration {
                calibrated_at: "cal-1".into(),
                tracks: 50,
                prompts: ids.iter().map(|id| (id.clone(), vocabulary.prompt_signature(id).unwrap())).collect(),
                stats: ids.iter().map(|id| (id.clone(), MoodStat { mean: 0.0, sd: 1.0 })).collect(),
                source,
                reference_set: None,
            },
        )
        .unwrap();
    }

    fn current_sig(db: &Mutex<Connection>) -> String {
        let conn = db.lock().unwrap();
        lines_sig(&settings::load_mood_vocabulary(&conn).unwrap().prompt_signature(MOOD).unwrap())
    }

    fn band(db: &Mutex<Connection>, id: &str, band: FlagBand, bound: FlagBound, sig: &str) {
        let conn = db.lock().unwrap();
        set_flag(&conn, id, MOOD, FlagKind::Band, Some(band), bound, None, None, sig).unwrap();
    }

    fn direction(db: &Mutex<Connection>, id: &str, kind: FlagKind, bar: f64, cal: &str, sig: &str) {
        let bound = if kind == FlagKind::Higher { FlagBound::AtLeast } else { FlagBound::AtMost };
        let conn = db.lock().unwrap();
        set_flag(&conn, id, MOOD, kind, None, bound, Some(bar), Some(cal), sig).unwrap();
    }

    fn set_bar(db: &Mutex<Connection>, id: &str, bar: f64) {
        db.lock()
            .unwrap()
            .execute(
                "INSERT INTO library_entry_moods (library_entry_id, mood, auto_score) VALUES (?1, ?2, ?3)
                 ON CONFLICT (library_entry_id, mood) DO UPDATE SET auto_score = excluded.auto_score",
                params![id, MOOD, bar],
            )
            .unwrap();
    }

    fn set_level(db: &Mutex<Connection>, id: &str, level: f64) {
        library::set_user_mood_level(&db.lock().unwrap(), &[id.to_string()], MOOD, Some(level)).unwrap();
    }

    fn request(db: &Mutex<Connection>) -> (TuneRequest, usize) {
        build_tune_request(&db.lock().unwrap(), MOOD, &[], &[]).unwrap()
    }

    /// (min, max, weight, fresh, side) of the flag on `hash`.
    fn target(req: &TuneRequest, hash: &str) -> (Option<f64>, Option<f64>, f64, bool, FlagSide) {
        let f = req.flags.iter().find(|f| f.hash == hash).unwrap_or_else(|| panic!("no flag on {hash}"));
        let round = |v: Option<f64>| v.map(|v| (v * 1e9).round() / 1e9);
        (round(f.min), round(f.max), f.weight, f.fresh, f.side)
    }

    use FlagSide::{High, Low, Mid};

    #[test]
    fn request_maps_bands_bounds_and_directions() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let sig = current_sig(&db);
        let bands = [
            ("d", FlagBand::Definitely, FlagBound::AtLeast, (Some(0.9), None, High)),
            ("u", FlagBand::Understandable, FlagBound::AtLeast, (Some(0.75), None, High)),
            ("si", FlagBand::Stretch, FlagBound::In, (Some(0.5), Some(0.75), Mid)),
            ("sl", FlagBand::Stretch, FlagBound::AtLeast, (Some(0.5), None, High)),
            ("sm", FlagBand::Stretch, FlagBound::AtMost, (None, Some(0.75), Low)),
            ("n", FlagBand::No, FlagBound::In, (None, Some(0.5), Low)),
            ("nm", FlagBand::No, FlagBound::AtMost, (None, Some(0.5), Low)),
            ("di", FlagBand::Definitely, FlagBound::In, (Some(0.9), None, High)),
            ("ui", FlagBand::Understandable, FlagBound::In, (Some(0.75), Some(0.9), Mid)),
            ("um", FlagBand::Understandable, FlagBound::AtMost, (None, Some(0.9), Low)),
        ];
        for (name, b, bound, _) in &bands {
            let id = add(&db, name, "music");
            band(&db, &id, *b, *bound, &sig);
        }
        let directions = [
            ("h", FlagKind::Higher, 0.62, (Some(0.72), None, High)),
            ("hcap", FlagKind::Higher, 0.95, (Some(1.0), None, High)),
            ("l", FlagKind::Lower, 0.3, (None, Some(0.2), Low)),
            ("lfloor", FlagKind::Lower, 0.05, (None, Some(0.0), Low)),
        ];
        for (name, kind, bar, _) in &directions {
            let id = add(&db, name, "music");
            direction(&db, &id, *kind, *bar, "cal-1", &sig);
        }

        let (req, skipped) = request(&db);
        assert_eq!(skipped, 0);
        assert_eq!(req.flags.len(), bands.len() + directions.len());
        let want = bands.iter().map(|(n, _, _, w)| (*n, *w)).chain(directions.iter().map(|(n, _, _, w)| (*n, *w)));
        for (name, (min, max, side)) in want {
            assert_eq!(target(&req, &format!("h-{name}")), (min, max, 1.0, true, side), "flag {name}");
        }
    }

    #[test]
    fn request_treats_user_levels_as_flags_and_real_flags_win() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let levels = [
            ("one", 1.0, (Some(0.9), None, High)),
            ("threequarters", 0.75, (Some(0.75), None, High)),
            ("half", 0.5, (Some(0.5), Some(0.75), Mid)),
            ("quarter", 0.25, (None, Some(0.5), Low)),
            ("zero", 0.0, (None, Some(0.5), Low)),
        ];
        for (name, level, _) in &levels {
            let id = add(&db, name, "music");
            set_level(&db, &id, *level);
        }
        // A real flag on a song with a level wins over the level.
        let both = add(&db, "both", "music");
        set_level(&db, &both, 1.0);
        band(&db, &both, FlagBand::No, FlagBound::In, &current_sig(&db));

        let (req, _) = request(&db);
        assert_eq!(req.flags.len(), levels.len() + 1);
        for (name, _, (min, max, side)) in levels {
            assert_eq!(target(&req, &format!("h-{name}")), (min, max, 1.0, true, side), "level {name}");
        }
        assert_eq!(target(&req, "h-both"), (None, Some(0.5), 1.0, true, Low));
    }

    #[test]
    fn request_stale_direction_flag_uses_current_bar() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let sig = current_sig(&db);
        // Made under an older calibration: read against today's bar, half weight.
        let old_cal = add(&db, "oldcal", "music");
        direction(&db, &old_cal, FlagKind::Higher, 0.3, "cal-0", &sig);
        set_bar(&db, &old_cal, 0.6);
        // Made under older wording.
        let old_words = add(&db, "oldwords", "music");
        direction(&db, &old_words, FlagKind::Lower, 0.8, "cal-1", "older lines");
        set_bar(&db, &old_words, 0.4);
        // A stale band flag keeps its band, at half weight.
        let old_band = add(&db, "oldband", "music");
        band(&db, &old_band, FlagBand::Definitely, FlagBound::AtLeast, "older lines");
        // A fresh one keeps the bar it was made at, whatever the bar is now.
        let fresh = add(&db, "fresh", "music");
        direction(&db, &fresh, FlagKind::Higher, 0.5, "cal-1", &sig);
        set_bar(&db, &fresh, 0.9);

        let (req, _) = request(&db);
        assert_eq!(target(&req, "h-oldcal"), (Some(0.7), None, 0.5, false, High));
        assert_eq!(target(&req, "h-oldwords"), (None, Some(0.3), 0.5, false, Low));
        assert_eq!(target(&req, "h-oldband"), (Some(0.9), None, 0.5, false, High));
        assert_eq!(target(&req, "h-fresh"), (Some(0.6), None, 1.0, true, High));
    }

    #[test]
    fn request_uses_calibration_source_and_mapping() {
        let db = setup();
        let id = add(&db, "a", "music");
        band(&db, &id, FlagBand::No, FlagBound::In, "x");

        calibrate(&db, CalibrationSource::Library);
        let c = request(&db).0.calibration;
        assert_eq!((c.source, c.midpoint_z, c.slope), (CalibrationSource::Library, 0.5, 1.5));

        calibrate(&db, CalibrationSource::Reference);
        let c = request(&db).0.calibration;
        assert_eq!(c.source, CalibrationSource::Reference);
        assert_eq!(c.midpoint_z, 0.674);
        assert_eq!(c.slope, reference_slope());
        assert!((c.slope - 3f64.ln() / (1.2816 - 0.674)).abs() < 1e-12);
    }

    #[test]
    fn request_carries_lines_targets_bank_and_other_moods() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let a = add(&db, "a", "music");
        add(&db, "b", "music");
        add(&db, "amb", "ambience");
        band(&db, &a, FlagBand::No, FlagBound::In, "x");
        let vocabulary = settings::load_mood_vocabulary(&db.lock().unwrap()).unwrap();

        let extra = vec![" mournful ".to_string(), "".to_string(), "calm".to_string()];
        let bank = vec!["calm".to_string(), "gentle".to_string()];
        let (req, _) = build_tune_request(&db.lock().unwrap(), MOOD, &extra, &bank).unwrap();

        let mood = vocabulary.moods.iter().find(|m| m.id == MOOD).unwrap();
        assert_eq!(req.mood_id, MOOD);
        assert_eq!(req.name, mood.name);
        assert_eq!(req.current_lines, mood.prompts);
        assert_eq!(req.targets, vec!["h-a".to_string(), "h-b".to_string()]);
        assert_eq!(req.bank, vec!["calm", "gentle", "mournful"]);
        let others: Vec<&String> = req.other_moods.keys().collect();
        let want: Vec<&String> = vocabulary.moods.iter().map(|m| &m.id).filter(|id| *id != MOOD).collect();
        assert_eq!(others.len(), want.len());
        assert!(want.iter().all(|id| req.other_moods.contains_key(*id)));
        assert!(!req.other_moods.contains_key("energy"));
    }

    #[test]
    fn request_skips_unanalyzed_flags() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let sig = current_sig(&db);
        let analyzed = add(&db, "a", "music");
        band(&db, &analyzed, FlagBand::No, FlagBound::In, &sig);
        let unhashed = library::insert_library_entry(
            &db.lock().unwrap(), "Z:\\media\\audio\\u.opus", None, "u", None, None, None, "music",
        )
        .unwrap();
        band(&db, &unhashed, FlagBand::Definitely, FlagBound::AtLeast, &sig);

        let (req, skipped) = request(&db);
        assert_eq!(skipped, 1);
        assert_eq!(req.flags.iter().map(|f| f.hash.as_str()).collect::<Vec<_>>(), vec!["h-a"]);
    }

    #[test]
    fn request_refuses_without_calibration_or_mood() {
        let db = setup();
        let a = add(&db, "a", "music");
        band(&db, &a, FlagBand::No, FlagBound::In, "x");
        let err = build_tune_request(&db.lock().unwrap(), MOOD, &[], &[]).unwrap_err();
        assert_eq!(err, "Run an analysis first so the moods are calibrated");
        calibrate(&db, CalibrationSource::Library);
        assert!(build_tune_request(&db.lock().unwrap(), "energy", &[], &[]).is_err());
        assert!(build_tune_request(&db.lock().unwrap(), "nosuchmood", &[], &[]).is_err());
    }

    #[test]
    fn tune_refuses_without_flags() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        add(&db, "a", "music");
        let fake = FakeAnalyzer::default();
        let err = tune(&db, &fake, MOOD, &[], &[]).unwrap_err();
        assert!(err.contains("Flag a few songs on this mood first"), "{err}");
        assert!(fake.last_tune.lock().unwrap().is_none(), "nothing is sent");
    }

    #[test]
    fn tune_error_leaves_vocabulary_untouched() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let a = add(&db, "a", "music");
        band(&db, &a, FlagBand::No, FlagBound::In, &current_sig(&db));
        let before = settings::load_mood_vocabulary(&db.lock().unwrap()).unwrap();
        let fake = FakeAnalyzer { down: Some("analyzer down".into()), ..Default::default() };

        assert_eq!(tune(&db, &fake, MOOD, &[], &[]).unwrap_err(), "analyzer down");
        assert_eq!(settings::load_mood_vocabulary(&db.lock().unwrap()).unwrap(), before);
        assert_eq!(flags_for_mood(&db.lock().unwrap(), MOOD).unwrap().len(), 1);
    }

    #[test]
    fn tune_resolves_hashes_to_entries() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let sig = current_sig(&db);
        let a = add(&db, "a", "music");
        let b = add(&db, "b", "music");
        band(&db, &a, FlagBand::Definitely, FlagBound::AtLeast, &sig);
        let unhashed = library::insert_library_entry(
            &db.lock().unwrap(), "Z:\\media\\audio\\u.opus", None, "u", None, None, None, "music",
        )
        .unwrap();
        band(&db, &unhashed, FlagBand::No, FlagBound::In, &sig);

        let report = |lines: &[&str]| TuneReport {
            lines: lines.iter().map(|s| s.to_string()).collect(),
            loss: 0.1,
            flags: vec![TuneFlagResult { hash: "h-a".into(), before: 0.4, after: 0.95, holds: true, fresh: true }],
            top: vec![TuneBar { hash: "h-b".into(), bar: 0.8 }, TuneBar { hash: "h-gone".into(), bar: 0.7 }],
            risers: vec![TuneMover { hash: "h-b".into(), before: 0.2, after: 0.8 }],
            unsatisfied: vec!["h-a".into()],
            ..Default::default()
        };
        let fake = FakeAnalyzer {
            tune: Some(TuneResponse {
                baseline: report(&["old"]),
                proposals: vec![report(&["new line"])],
                split: Some(TuneSplit {
                    keep: report(&["k"]),
                    new: report(&["n"]),
                    keep_hashes: vec!["h-a".into()],
                    new_hashes: vec!["h-b".into()],
                    rho: 0.2,
                }),
                skipped: vec!["h-a".into()],
            }),
            ..Default::default()
        };

        let result = tune(&db, &fake, MOOD, &[], &[]).unwrap();
        let sent = fake.last_tune.lock().unwrap().clone().unwrap();
        assert_eq!(sent.flags.len(), 1);
        assert_eq!(result.lines_tuned_from, sent.current_lines);
        assert_eq!(result.skipped_flags, 1);
        let song = |id: &str, name: &str| TuneSong { entry_id: id.to_string(), name: name.to_string(), album: "media / audio".into() };
        assert_eq!(result.skipped, vec![song(&a, "a")]);
        let p = &result.proposals[0];
        assert_eq!(p.lines, vec!["new line"]);
        assert_eq!(p.flags[0].song, song(&a, "a"));
        assert_eq!((p.flags[0].before, p.flags[0].after, p.flags[0].holds), (0.4, 0.95, true));
        // A hash no entry has any more is left out.
        assert_eq!(p.top.len(), 1);
        assert_eq!(p.top[0].song, song(&b, "b"));
        assert_eq!(p.risers[0].song, song(&b, "b"));
        assert_eq!(p.unsatisfied, vec![song(&a, "a")]);
        let split = result.split.unwrap();
        assert_eq!(split.keep.lines, vec!["k"]);
        assert_eq!(split.keep_songs, vec![song(&a, "a")]);
        assert_eq!(split.new_songs, vec![song(&b, "b")]);
    }

    #[test]
    fn wire_shapes_match_the_analyzer() {
        // The shape tune.build_report writes, with no split (null) as the analyzer sends it.
        let report = r#"{"lines":["a"],"loss":0.25,
            "flags":[{"hash":"h","before":0.4,"after":0.8,"holds":true,"fresh":false}],
            "top":[{"hash":"h","bar":0.8}],"top_before":[{"hash":"h","bar":0.4}],
            "risers":[{"hash":"g","before":0.1,"after":0.3}],"fallers":[],
            "fires_before":3,"fires_after":5,
            "overlap":[{"mood_id":"grim","before":0.2,"after":0.4}],
            "loo":{"held":0,"total":0},"names":[{"word":"calm","rho":0.7}],"unsatisfied":["h"]}"#;
        let json = format!(r#"{{"baseline":{report},"proposals":[{report}],"split":null,"skipped":["x"]}}"#);
        let parsed: TuneResponse = serde_json::from_str(&json).unwrap();
        let r = &parsed.proposals[0];
        assert_eq!((r.fires_before, r.fires_after, r.loo.held, r.loo.total), (3, 5, 0, 0));
        assert_eq!(r.top_before[0].bar, 0.4);
        assert_eq!(r.overlap[0].mood_id, "grim");
        assert_eq!(r.names[0].word, "calm");
        assert!(!r.flags[0].fresh);
        assert_eq!((parsed.split, parsed.skipped), (None, vec!["x".to_string()]));

        let split = format!(
            r#"{{"keep":{report},"new":{report},"keep_hashes":["h"],"new_hashes":["g"],"rho":0.1}}"#
        );
        let split: TuneSplit = serde_json::from_str(&split).unwrap();
        assert_eq!((split.keep_hashes, split.new_hashes), (vec!["h".to_string()], vec!["g".to_string()]));

        let req = TuneRequest {
            mood_id: "sad".into(),
            name: "Sad".into(),
            current_lines: vec!["sad".into()],
            flags: vec![TuneFlag { hash: "h".into(), min: Some(0.5), max: None, weight: 1.0, fresh: true, side: Mid }],
            targets: vec!["h".into()],
            bank: vec![],
            other_moods: BTreeMap::new(),
            calibration: TuneCalibration { source: CalibrationSource::Reference, midpoint_z: 0.674, slope: 1.7 },
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["mood_id"], "sad");
        assert_eq!(v["current_lines"][0], "sad");
        assert_eq!(v["flags"][0]["side"], "mid");
        assert_eq!(v["flags"][0]["max"], serde_json::Value::Null);
        assert_eq!(v["other_moods"], serde_json::json!({}));
        assert_eq!(v["calibration"]["source"], "reference");
        assert_eq!(v["calibration"]["midpoint_z"], 0.674);
    }

    fn prompts_of(conn: &Connection, id: &str) -> Option<Vec<String>> {
        settings::load_mood_vocabulary(conn).unwrap().prompt_signature(id)
    }

    #[test]
    fn accept_refuses_when_lines_changed() {
        let db = setup();
        let conn = db.lock().unwrap();
        let before = settings::load_mood_vocabulary(&conn).unwrap();
        let stale = vec!["not the current line".to_string()];
        let err = accept_lines(&conn, MOOD, &stale, &["new".to_string()]).unwrap_err();
        assert!(err.contains("changed since Tune ran"), "{err}");
        let err = accept_split(&conn, MOOD, &stale, &["k".into()], "fresh", "Fresh", &["n".into()], &[]).unwrap_err();
        assert!(err.contains("changed since Tune ran"), "{err}");
        let err = accept_lines(&conn, "nosuchmood", &stale, &["new".to_string()]).unwrap_err();
        assert!(err.contains("no longer"), "{err}");
        assert_eq!(settings::load_mood_vocabulary(&conn).unwrap(), before);
    }

    #[test]
    fn accept_saves_lines_like_a_manual_edit() {
        let db = setup();
        calibrate(&db, CalibrationSource::Library);
        let conn = db.lock().unwrap();
        let current = prompts_of(&conn, MOOD).unwrap();
        assert!(!needs_recalibration(settings::load_mood_calibration(&conn).unwrap().as_ref(), &settings::load_mood_vocabulary(&conn).unwrap()));

        let lines = vec!["the calmest music".to_string(), "a quiet lake".to_string()];
        accept_lines(&conn, MOOD, &current, &lines).unwrap();

        assert_eq!(prompts_of(&conn, MOOD).unwrap(), lines);
        assert!(needs_recalibration(settings::load_mood_calibration(&conn).unwrap().as_ref(), &settings::load_mood_vocabulary(&conn).unwrap()));
    }

    #[test]
    fn accept_split_adds_mood_moves_flags_copies_low_flags() {
        let db = setup();
        let sig = current_sig(&db);
        let (high, moved, low) = (add(&db, "high", "music"), add(&db, "moved", "music"), add(&db, "low", "music"));
        band(&db, &high, FlagBand::Definitely, FlagBound::AtLeast, &sig);
        band(&db, &moved, FlagBand::Understandable, FlagBound::AtLeast, &sig);
        band(&db, &low, FlagBand::No, FlagBound::In, &sig);
        let conn = db.lock().unwrap();
        let current = prompts_of(&conn, MOOD).unwrap();
        let before = settings::load_mood_vocabulary(&conn).unwrap();

        accept_split(
            &conn,
            MOOD,
            &current,
            &["serene".into()],
            "pastoral",
            "Pastoral",
            &["rolling fields".into(), "shepherd's pipe".into()],
            std::slice::from_ref(&moved),
        )
        .unwrap();

        let after = settings::load_mood_vocabulary(&conn).unwrap();
        assert_eq!(after.moods.len(), before.moods.len() + 1);
        assert_eq!(prompts_of(&conn, MOOD).unwrap(), vec!["serene"]);
        let new = after.moods.last().unwrap();
        assert_eq!((new.id.as_str(), new.name.as_str()), ("pastoral", "Pastoral"));
        assert_eq!(new.prompts, vec!["rolling fields", "shepherd's pipe"]);

        let ids = |mood: &str| {
            let mut v: Vec<String> = flags_for_mood(&conn, mood).unwrap().into_iter().map(|f| f.entry_id).collect();
            v.sort();
            v
        };
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        assert_eq!(ids(MOOD), sorted(vec![high, low.clone()]));
        assert_eq!(ids("pastoral"), sorted(vec![moved, low]));
    }

    #[test]
    fn accept_split_refuses_an_invalid_new_mood() {
        let db = setup();
        let conn = db.lock().unwrap();
        let current = prompts_of(&conn, MOOD).unwrap();
        let before = settings::load_mood_vocabulary(&conn).unwrap();
        // An id that is not lowercase letters fails validation: nothing is written.
        let err = accept_split(&conn, MOOD, &current, &["k".into()], "Bad Id", "Bad", &["n".into()], &[]);
        assert!(err.is_err());
        assert_eq!(settings::load_mood_vocabulary(&conn).unwrap(), before);
    }
}
