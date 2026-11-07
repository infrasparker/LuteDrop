//! Flags: the user's note on one song and one mood ("this bar belongs in
//! band X", or "higher" / "lower"). A flag is a tuning target for the mood
//! auto-tune; it never changes the bar the song shows.
//!
//! Each flag remembers the mood's prompt lines (`lines_sig`) from when it was
//! set or last kept. If the wording changes later the flag is `stale`: it
//! was made against different lines, and the user can keep or drop it.
//!
//! A higher / lower flag also remembers its mood's calibration (the mean and
//! sd its bar was read with) in the `calibrated_at` column, as "mean|sd" (see
//! `calibration_mark`; the column name predates this and once held the
//! calibration's timestamp). Once that mood is recalibrated, the bar the flag
//! was made at is on another scale, so the flag is stale by calibration and
//! Tune reads "higher" / "lower" against the song's current bar instead.
//! Recalibrating other moods does not touch it.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use super::settings;
use crate::autotag::plan::MoodStat;
use crate::models::{FlagBand, FlagBound, FlagKind, MoodFlag, MoodVocabulary, StaleReason, ENERGY_ID};

impl FlagKind {
    fn as_db_str(self) -> &'static str {
        match self {
            FlagKind::Band => "band",
            FlagKind::Higher => "higher",
            FlagKind::Lower => "lower",
        }
    }

    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "band" => Some(FlagKind::Band),
            "higher" => Some(FlagKind::Higher),
            "lower" => Some(FlagKind::Lower),
            _ => None,
        }
    }
}

impl FlagBand {
    fn as_db_str(self) -> &'static str {
        match self {
            FlagBand::Definitely => "definitely",
            FlagBand::Understandable => "understandable",
            FlagBand::Stretch => "stretch",
            FlagBand::No => "no",
        }
    }

    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "definitely" => Some(FlagBand::Definitely),
            "understandable" => Some(FlagBand::Understandable),
            "stretch" => Some(FlagBand::Stretch),
            "no" => Some(FlagBand::No),
            _ => None,
        }
    }
}

impl FlagBound {
    fn as_db_str(self) -> &'static str {
        match self {
            FlagBound::In => "in",
            FlagBound::AtLeast => "at_least",
            FlagBound::AtMost => "at_most",
        }
    }

    fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "in" => Some(FlagBound::In),
            "at_least" => Some(FlagBound::AtLeast),
            "at_most" => Some(FlagBound::AtMost),
            _ => None,
        }
    }
}

/// A flag as stored, for the tuner: the same fields as `MoodFlag` plus whose
/// flag it is and the lines it was made against.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredFlag {
    pub entry_id: String,
    pub mood: String,
    pub kind: FlagKind,
    pub band: Option<FlagBand>,
    pub bound: FlagBound,
    pub bar_at_flag: Option<f64>,
    pub calibrated_at: Option<String>,
    pub lines_sig: String,
}

/// A mood's prompt lines as one string, what `lines_sig` stores.
pub fn lines_sig(prompts: &[String]) -> String {
    prompts.join("\n")
}

/// What a direction flag stores in `calibrated_at`: its mood's calibration
/// statistics as "mean|sd". Rust prints the shortest text that reads back as
/// the same f64, so the mark round-trips exactly.
pub fn calibration_mark(stat: MoodStat) -> String {
    format!("{}|{}", stat.mean, stat.sd)
}

fn read_mark(mark: &str) -> Option<MoodStat> {
    let (mean, sd) = mark.split_once('|')?;
    Some(MoodStat { mean: mean.parse().ok()?, sd: sd.parse().ok()? })
}

/// Statistics closer than this are the same calibration (float noise, not a
/// recalibration; a real one moves the mean by far more).
const MARK_TOLERANCE: f64 = 1e-9;

/// One mood's current calibration statistics, if it has been calibrated.
pub fn current_stat(conn: &Connection, mood: &str) -> Result<Option<MoodStat>, String> {
    Ok(settings::load_mood_calibration(conn)?.and_then(|c| c.stats.get(mood).copied()))
}

/// Why a flag is stale, if it is, against the mood's current lines
/// (`current_sig`) and calibration (`current_stat`). Only a higher / lower
/// flag can be stale by calibration: when its mood has a calibration now and
/// the flag's mark is missing, unreadable (an old timestamp) or differs.
pub fn stale_reason(f: &StoredFlag, current_sig: &str, current_stat: Option<MoodStat>) -> Option<StaleReason> {
    let direction = matches!(f.kind, FlagKind::Higher | FlagKind::Lower);
    if let (true, Some(now)) = (direction, current_stat) {
        let same = f.calibrated_at.as_deref().and_then(read_mark).is_some_and(|then| {
            (then.mean - now.mean).abs() <= MARK_TOLERANCE && (then.sd - now.sd).abs() <= MARK_TOLERANCE
        });
        if !same {
            return Some(StaleReason::Calibration);
        }
    }
    (f.lines_sig != current_sig).then_some(StaleReason::Wording)
}

fn now_millis_i64() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Sets the flag for a song and mood, replacing any it already had.
#[allow(clippy::too_many_arguments)]
pub fn set_flag(
    conn: &Connection,
    entry_id: &str,
    mood: &str,
    kind: FlagKind,
    band: Option<FlagBand>,
    bound: FlagBound,
    bar_at_flag: Option<f64>,
    calibrated_at: Option<&str>,
    lines_sig: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO mood_flags
             (library_entry_id, mood, kind, band, bound, bar_at_flag, calibrated_at, lines_sig, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (library_entry_id, mood) DO UPDATE SET
             kind = excluded.kind, band = excluded.band, bound = excluded.bound,
             bar_at_flag = excluded.bar_at_flag, calibrated_at = excluded.calibrated_at,
             lines_sig = excluded.lines_sig, created_at = excluded.created_at",
        params![
            entry_id,
            mood,
            kind.as_db_str(),
            band.map(FlagBand::as_db_str),
            bound.as_db_str(),
            bar_at_flag,
            calibrated_at,
            lines_sig,
            now_millis_i64(),
        ],
    )
    .map_err(|e| format!("Failed to set flag: {e}"))?;
    Ok(())
}

/// Removes the flag for a song and mood, if there is one.
pub fn clear_flag(conn: &Connection, entry_id: &str, mood: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM mood_flags WHERE library_entry_id = ?1 AND mood = ?2",
        params![entry_id, mood],
    )
    .map_err(|e| format!("Failed to clear flag: {e}"))?;
    Ok(())
}

/// "Keep": the flag stays as it is but now counts as made against `lines_sig`,
/// so it is no longer stale.
pub fn refresh_flag(conn: &Connection, entry_id: &str, mood: &str, lines_sig: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE mood_flags SET lines_sig = ?3 WHERE library_entry_id = ?1 AND mood = ?2",
        params![entry_id, mood, lines_sig],
    )
    .map_err(|e| format!("Failed to refresh flag: {e}"))?;
    Ok(())
}

fn read_row(row: &rusqlite::Row) -> rusqlite::Result<StoredFlag> {
    let bad = |what: &str, v: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown flag {what} '{v}'").into(),
        )
    };
    let kind: String = row.get(2)?;
    let band: Option<String> = row.get(3)?;
    let bound: String = row.get(4)?;
    Ok(StoredFlag {
        entry_id: row.get(0)?,
        mood: row.get(1)?,
        kind: FlagKind::from_db_str(&kind).ok_or_else(|| bad("kind", &kind))?,
        band: match band {
            None => None,
            Some(b) => Some(FlagBand::from_db_str(&b).ok_or_else(|| bad("band", &b))?),
        },
        bound: FlagBound::from_db_str(&bound).ok_or_else(|| bad("bound", &bound))?,
        bar_at_flag: row.get(5)?,
        calibrated_at: row.get(6)?,
        lines_sig: row.get(7)?,
    })
}

const SELECT_FLAGS: &str =
    "SELECT library_entry_id, mood, kind, band, bound, bar_at_flag, calibrated_at, lines_sig FROM mood_flags";

/// Every flag of one mood.
pub fn flags_for_mood(conn: &Connection, mood: &str) -> Result<Vec<StoredFlag>, String> {
    let mut stmt = conn
        .prepare(&format!("{SELECT_FLAGS} WHERE mood = ?1 ORDER BY created_at, library_entry_id"))
        .map_err(|e| format!("Failed to prepare flag query: {e}"))?;
    let rows = stmt
        .query_map(params![mood], read_row)
        .map_err(|e| format!("Failed to query flags: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("Failed to read flags: {e}"))
}

/// The entries flagged on `mood`, the flag set or last changed most recently
/// first (`set_flag` moves `created_at` on every change; "keep" does not).
pub fn flags_newest_first(conn: &Connection, mood: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT library_entry_id FROM mood_flags WHERE mood = ?1 ORDER BY created_at DESC, rowid DESC")
        .map_err(|e| format!("Failed to prepare flag query: {e}"))?;
    let rows = stmt
        .query_map(params![mood], |r| r.get(0))
        .map_err(|e| format!("Failed to query flags: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("Failed to read flags: {e}"))
}

/// All flags keyed by (entry id, mood), with `stale` worked out against the
/// vocabulary's current lines and each mood's current calibration (see
/// `stale_reason`). Flags on moods no longer in the vocabulary are left out.
pub fn load_flags(
    conn: &Connection,
    vocabulary: &MoodVocabulary,
) -> Result<HashMap<(String, String), MoodFlag>, String> {
    let mut stmt = conn
        .prepare(SELECT_FLAGS)
        .map_err(|e| format!("Failed to prepare flag query: {e}"))?;
    let rows = stmt
        .query_map([], read_row)
        .map_err(|e| format!("Failed to query flags: {e}"))?;
    let stats = settings::load_mood_calibration(conn)?.map(|c| c.stats).unwrap_or_default();
    let mut out = HashMap::new();
    for row in rows {
        let f = row.map_err(|e| format!("Failed to read flag: {e}"))?;
        let Some(current) = vocabulary.prompt_signature(&f.mood) else { continue };
        let reason = stale_reason(&f, &lines_sig(&current), stats.get(&f.mood).copied());
        out.insert(
            (f.entry_id, f.mood),
            MoodFlag {
                kind: f.kind,
                band: f.band,
                bound: f.bound,
                bar_at_flag: f.bar_at_flag,
                calibrated_at: f.calibrated_at,
                stale: reason.is_some(),
                stale_reason: reason,
            },
        );
    }
    Ok(out)
}

/// The bound a band flag set in the app gets: Definitely and Understandable
/// mean "at least" that band, Stretch and No mean "in" it.
pub fn app_bound(band: FlagBand) -> FlagBound {
    match band {
        FlagBand::Definitely | FlagBand::Understandable => FlagBound::AtLeast,
        FlagBand::Stretch | FlagBand::No => FlagBound::In,
    }
}

/// The flag command's work: validates and stores one flag the user made on a
/// song, against the current vocabulary and calibration.
///
/// A `band` flag needs a band (Definitely and Understandable mean "at least"
/// that band, Stretch and No mean "in" it). `higher` and `lower` carry no band
/// and record the song's suggested bar for the mood (the analyzer's value, not
/// the user's level) and the mood's calibration (`calibration_mark`), so the
/// tuner can read them against the bar as it was.
pub fn set_flag_for_entry(
    conn: &Connection,
    entry_id: &str,
    mood: &str,
    kind: FlagKind,
    band: Option<FlagBand>,
) -> Result<(), String> {
    let vocabulary = settings::load_mood_vocabulary(conn)?;
    if mood == ENERGY_ID || !vocabulary.contains(mood) {
        return Err(format!("There is no mood \"{mood}\" to flag"));
    }
    let lines = lines_sig(&vocabulary.prompt_signature(mood).unwrap_or_default());
    match (kind, band) {
        (FlagKind::Band, None) => Err("Pick a band for this flag".to_string()),
        (FlagKind::Higher | FlagKind::Lower, Some(_)) => {
            Err("A higher or lower flag does not take a band".to_string())
        }
        (FlagKind::Band, Some(band)) => {
            set_flag(conn, entry_id, mood, kind, Some(band), app_bound(band), None, None, &lines)
        }
        (FlagKind::Higher | FlagKind::Lower, None) => {
            let bar: Option<f64> = conn
                .query_row(
                    "SELECT auto_score FROM library_entry_moods WHERE library_entry_id = ?1 AND mood = ?2",
                    params![entry_id, mood],
                    |r| r.get(0),
                )
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    e => Err(e),
                })
                .map_err(|e| format!("Failed to read the suggested value: {e}"))?;
            let bar = bar.ok_or("This song has no suggested value for that mood yet")?;
            let calibrated_at = current_stat(conn, mood)?.map(calibration_mark);
            let bound = if kind == FlagKind::Higher { FlagBound::AtLeast } else { FlagBound::AtMost };
            set_flag(conn, entry_id, mood, kind, None, bound, Some(bar), calibrated_at.as_deref(), &lines)
        }
    }
}

/// "Keep" on a stale flag: it now counts as made against the mood's current
/// lines and, for a higher / lower flag, its current calibration (the bar it
/// was made at stays). A mood no longer in the vocabulary is refused.
pub fn keep_flag(conn: &Connection, entry_id: &str, mood: &str) -> Result<(), String> {
    let lines = settings::load_mood_vocabulary(conn)?
        .prompt_signature(mood)
        .ok_or_else(|| format!("There is no mood \"{mood}\""))?;
    refresh_flag(conn, entry_id, mood, &lines_sig(&lines))?;
    if let Some(stat) = current_stat(conn, mood)? {
        conn.execute(
            "UPDATE mood_flags SET calibrated_at = ?3
             WHERE library_entry_id = ?1 AND mood = ?2 AND kind IN ('higher', 'lower')",
            params![entry_id, mood, calibration_mark(stat)],
        )
        .map_err(|e| format!("Failed to refresh flag: {e}"))?;
    }
    Ok(())
}

/// Moves the listed songs' flags from one mood to another (for a mood split
/// or merge). A flag already on the target mood is replaced.
pub fn move_flags(conn: &Connection, from: &str, to: &str, entry_ids: &[String]) -> Result<(), String> {
    for id in entry_ids {
        conn.execute(
            "UPDATE OR REPLACE mood_flags SET mood = ?2 WHERE mood = ?1 AND library_entry_id = ?3",
            params![from, to, id],
        )
        .map_err(|e| format!("Failed to move flags: {e}"))?;
    }
    Ok(())
}

/// Copies the "not this mood" flags (band `no`, bound `at_most`, kind
/// `lower`) from one mood to another, leaving any flag the target already
/// has alone.
pub fn copy_low_flags(conn: &Connection, from: &str, to: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO mood_flags
             (library_entry_id, mood, kind, band, bound, bar_at_flag, calibrated_at, lines_sig, created_at)
         SELECT library_entry_id, ?2, kind, band, bound, bar_at_flag, calibrated_at, lines_sig, created_at
         FROM mood_flags
         WHERE mood = ?1 AND (band = 'no' OR bound = 'at_most' OR kind = 'lower')",
        params![from, to],
    )
    .map_err(|e| format!("Failed to copy flags: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotag::plan::{MoodCalibration, MoodStat};
    use crate::database::{library, open_memory_db};
    use crate::models::StaleReason;

    fn entry(conn: &Connection, name: &str) -> String {
        library::insert_library_entry(conn, &format!("/{name}.mp3"), None, name, None, None, None, "music").unwrap()
    }

    fn vocab_with_lines(lines: &[&str]) -> MoodVocabulary {
        let mut v = MoodVocabulary::defaults();
        v.moods[0].prompts = lines.iter().map(|s| s.to_string()).collect();
        v
    }

    fn count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM mood_flags", [], |r| r.get(0)).unwrap()
    }

    fn band_flag(conn: &Connection, id: &str, mood: &str, band: FlagBand, sig: &str) {
        set_flag(conn, id, mood, FlagKind::Band, Some(band), FlagBound::In, Some(0.5), None, sig).unwrap();
    }

    #[test]
    fn set_flag_replaces_existing_flag_for_same_song_and_mood() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        band_flag(&conn, &a, "peaceful", FlagBand::Stretch, "x");
        set_flag(&conn, &a, "peaceful", FlagKind::Higher, None, FlagBound::AtLeast, Some(0.7), Some("cal"), "y").unwrap();
        assert_eq!(count(&conn), 1);
        let flags = flags_for_mood(&conn, "peaceful").unwrap();
        assert_eq!(
            flags,
            vec![StoredFlag {
                entry_id: a,
                mood: "peaceful".into(),
                kind: FlagKind::Higher,
                band: None,
                bound: FlagBound::AtLeast,
                bar_at_flag: Some(0.7),
                calibrated_at: Some("cal".into()),
                lines_sig: "y".into(),
            }]
        );
    }

    #[test]
    fn changing_a_flag_makes_it_the_newest() {
        let conn = open_memory_db();
        let (a, b) = (entry(&conn, "a"), entry(&conn, "b"));
        assert!(flags_newest_first(&conn, "peaceful").unwrap().is_empty());
        band_flag(&conn, &a, "peaceful", FlagBand::Stretch, "x");
        std::thread::sleep(std::time::Duration::from_millis(5));
        band_flag(&conn, &b, "peaceful", FlagBand::Stretch, "x");
        assert_eq!(flags_newest_first(&conn, "peaceful").unwrap(), vec![b.clone(), a.clone()]);
        // Changing a's flag (an upsert) moves its created_at on, so it is the newest again.
        std::thread::sleep(std::time::Duration::from_millis(5));
        band_flag(&conn, &a, "peaceful", FlagBand::No, "x");
        assert_eq!(flags_newest_first(&conn, "peaceful").unwrap(), vec![a.clone(), b.clone()]);
        // Keeping a stale flag is not a change: b stays behind a.
        refresh_flag(&conn, &b, "peaceful", "y").unwrap();
        assert_eq!(flags_newest_first(&conn, "peaceful").unwrap(), vec![a, b]);
        assert!(flags_newest_first(&conn, "grim").unwrap().is_empty());
    }

    #[test]
    fn flag_goes_stale_when_mood_lines_change() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        let mood = MoodVocabulary::defaults().moods[0].id.clone();
        band_flag(&conn, &a, &mood, FlagBand::Definitely, "a\nb");

        let same = load_flags(&conn, &vocab_with_lines(&["a", "b"])).unwrap();
        assert!(!same[&(a.clone(), mood.clone())].stale);
        let changed = load_flags(&conn, &vocab_with_lines(&["a", "c"])).unwrap();
        assert!(changed[&(a, mood)].stale);
    }

    #[test]
    fn refresh_flag_clears_staleness() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        let mood = MoodVocabulary::defaults().moods[0].id.clone();
        band_flag(&conn, &a, &mood, FlagBand::Definitely, "a\nb");
        let v = vocab_with_lines(&["a", "c"]);
        assert!(load_flags(&conn, &v).unwrap()[&(a.clone(), mood.clone())].stale);
        refresh_flag(&conn, &a, &mood, &lines_sig(&["a".to_string(), "c".to_string()])).unwrap();
        assert!(!load_flags(&conn, &v).unwrap()[&(a, mood)].stale);
    }

    #[test]
    fn clear_flag_removes_only_that_flag() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        band_flag(&conn, &a, "peaceful", FlagBand::No, "x");
        band_flag(&conn, &a, "grim", FlagBand::No, "x");
        clear_flag(&conn, &a, "peaceful").unwrap();
        assert!(flags_for_mood(&conn, "peaceful").unwrap().is_empty());
        assert_eq!(flags_for_mood(&conn, "grim").unwrap().len(), 1);
    }

    #[test]
    fn deleting_entry_deletes_its_flags() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        band_flag(&conn, &a, "peaceful", FlagBand::No, "x");
        library::delete_library_entry(&conn, &a).unwrap();
        assert_eq!(count(&conn), 0);
    }

    #[test]
    fn move_flags_moves_only_listed_entries() {
        let conn = open_memory_db();
        let (a, b) = (entry(&conn, "a"), entry(&conn, "b"));
        band_flag(&conn, &a, "peaceful", FlagBand::Stretch, "x");
        band_flag(&conn, &b, "peaceful", FlagBand::Stretch, "x");
        move_flags(&conn, "peaceful", "grim", &[a.clone()]).unwrap();
        let moved = flags_for_mood(&conn, "grim").unwrap();
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].entry_id, a);
        let left = flags_for_mood(&conn, "peaceful").unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].entry_id, b);
    }

    #[test]
    fn copy_low_flags_copies_no_at_most_and_lower_only() {
        let conn = open_memory_db();
        let ids: Vec<String> = (0..5).map(|i| entry(&conn, &format!("s{i}"))).collect();
        band_flag(&conn, &ids[0], "peaceful", FlagBand::No, "x");
        set_flag(&conn, &ids[1], "peaceful", FlagKind::Band, Some(FlagBand::Stretch), FlagBound::AtMost, None, None, "x").unwrap();
        set_flag(&conn, &ids[2], "peaceful", FlagKind::Lower, None, FlagBound::In, None, None, "x").unwrap();
        band_flag(&conn, &ids[3], "peaceful", FlagBand::Definitely, "x");
        set_flag(&conn, &ids[4], "peaceful", FlagKind::Higher, None, FlagBound::AtLeast, None, None, "x").unwrap();

        copy_low_flags(&conn, "peaceful", "grim").unwrap();

        let mut copied: Vec<String> = flags_for_mood(&conn, "grim").unwrap().into_iter().map(|f| f.entry_id).collect();
        copied.sort();
        let mut want = vec![ids[0].clone(), ids[1].clone(), ids[2].clone()];
        want.sort();
        assert_eq!(copied, want);
        assert_eq!(flags_for_mood(&conn, "peaceful").unwrap().len(), 5);
    }

    fn set_auto(conn: &Connection, id: &str, mood: &str, score: f64) {
        conn.execute(
            "INSERT INTO library_entry_moods (library_entry_id, mood, auto_score, user_level) VALUES (?1, ?2, ?3, 1.0)",
            params![id, mood, score],
        )
        .unwrap();
    }

    #[test]
    fn set_mood_flag_band_definitely_is_at_least() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        let want = [
            (FlagBand::Definitely, FlagBound::AtLeast),
            (FlagBand::Understandable, FlagBound::AtLeast),
            (FlagBand::Stretch, FlagBound::In),
            (FlagBand::No, FlagBound::In),
        ];
        for (band, bound) in want {
            set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Band, Some(band)).unwrap();
            let f = &flags_for_mood(&conn, "peaceful").unwrap()[0];
            assert_eq!((f.band, f.bound, f.bar_at_flag), (Some(band), bound, None));
        }
        let sig = lines_sig(&MoodVocabulary::defaults().prompt_signature("peaceful").unwrap());
        assert_eq!(flags_for_mood(&conn, "peaceful").unwrap()[0].lines_sig, sig);
    }

    fn calibrate(conn: &Connection, stats: &[(&str, f64, f64)]) {
        settings::save_mood_calibration(
            conn,
            &MoodCalibration {
                calibrated_at: "1".into(),
                tracks: 50,
                prompts: Default::default(),
                stats: stats.iter().map(|(id, mean, sd)| (id.to_string(), MoodStat { mean: *mean, sd: *sd })).collect(),
                source: Default::default(),
                reference_set: None,
            },
        )
        .unwrap();
    }

    fn reason(conn: &Connection, id: &str, mood: &str) -> Option<StaleReason> {
        let flags = load_flags(conn, &settings::load_mood_vocabulary(conn).unwrap()).unwrap();
        let f = &flags[&(id.to_string(), mood.to_string())];
        assert_eq!(f.stale, f.stale_reason.is_some());
        f.stale_reason
    }

    #[test]
    fn direction_flag_goes_stale_only_when_its_own_moods_calibration_changes() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        set_auto(&conn, &a, "peaceful", 0.4);
        calibrate(&conn, &[("peaceful", 0.3, 0.05), ("grim", 0.2, 0.04)]);
        set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Higher, None).unwrap();
        assert_eq!(reason(&conn, &a, "peaceful"), None);

        // Another mood recalibrated: this flag still reads against the same yardstick.
        calibrate(&conn, &[("peaceful", 0.3, 0.05), ("grim", 0.25, 0.03)]);
        assert_eq!(reason(&conn, &a, "peaceful"), None);
        // Float noise from a JSON round trip is not a new calibration.
        calibrate(&conn, &[("peaceful", 0.3 + 1e-13, 0.05), ("grim", 0.25, 0.03)]);
        assert_eq!(reason(&conn, &a, "peaceful"), None);
        // Its own mood recalibrated: the bar it was made at means something else now.
        calibrate(&conn, &[("peaceful", 0.31, 0.05), ("grim", 0.25, 0.03)]);
        assert_eq!(reason(&conn, &a, "peaceful"), Some(StaleReason::Calibration));
    }

    #[test]
    fn band_flags_go_stale_only_by_wording() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        calibrate(&conn, &[("peaceful", 0.3, 0.05)]);
        set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Band, Some(FlagBand::Stretch)).unwrap();
        calibrate(&conn, &[("peaceful", 0.5, 0.05)]);
        assert_eq!(reason(&conn, &a, "peaceful"), None);
        band_flag(&conn, &a, "peaceful", FlagBand::Stretch, "older lines");
        assert_eq!(reason(&conn, &a, "peaceful"), Some(StaleReason::Wording));
    }

    #[test]
    fn keep_flag_refreshes_wording_and_calibration() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        calibrate(&conn, &[("peaceful", 0.3, 0.05)]);
        set_flag(&conn, &a, "peaceful", FlagKind::Lower, None, FlagBound::AtMost, Some(0.7), Some("0.2|0.05"), "old")
            .unwrap();
        assert_eq!(reason(&conn, &a, "peaceful"), Some(StaleReason::Calibration));
        keep_flag(&conn, &a, "peaceful").unwrap();
        assert_eq!(reason(&conn, &a, "peaceful"), None);
        let f = &flags_for_mood(&conn, "peaceful").unwrap()[0];
        assert_eq!((f.calibrated_at.as_deref(), f.bar_at_flag), (Some("0.3|0.05"), Some(0.7)));
    }

    #[test]
    fn set_mood_flag_higher_records_bar_and_calibration() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        settings::save_mood_calibration(
            &conn,
            &MoodCalibration {
                calibrated_at: "123".into(),
                tracks: 50,
                prompts: Default::default(),
                stats: [("peaceful".to_string(), MoodStat { mean: 0.3, sd: 0.05 })].into_iter().collect(),
                source: Default::default(),
                reference_set: None,
            },
        )
        .unwrap();

        // No suggestion yet: refused.
        let err = set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Higher, None).unwrap_err();
        assert_eq!(err, "This song has no suggested value for that mood yet");

        set_auto(&conn, &a, "peaceful", 0.62);
        set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Higher, None).unwrap();
        let f = &flags_for_mood(&conn, "peaceful").unwrap()[0];
        assert_eq!(
            (f.kind, f.band, f.bound, f.bar_at_flag, f.calibrated_at.as_deref()),
            (FlagKind::Higher, None, FlagBound::AtLeast, Some(0.62), Some("0.3|0.05"))
        );
        set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Lower, None).unwrap();
        assert_eq!(flags_for_mood(&conn, "peaceful").unwrap()[0].bound, FlagBound::AtMost);
    }

    #[test]
    fn set_mood_flag_refuses_energy_unknown_mood_and_bad_kind_band_pairs() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        set_auto(&conn, &a, "peaceful", 0.5);
        assert!(set_flag_for_entry(&conn, &a, "energy", FlagKind::Band, Some(FlagBand::No)).is_err());
        assert!(set_flag_for_entry(&conn, &a, "nosuchmood", FlagKind::Band, Some(FlagBand::No)).is_err());
        assert!(set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Band, None).is_err());
        assert!(set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Higher, Some(FlagBand::No)).is_err());
        assert!(set_flag_for_entry(&conn, &a, "peaceful", FlagKind::Lower, Some(FlagBand::No)).is_err());
        assert_eq!(count(&conn), 0);
    }

    #[test]
    fn keep_flag_refreshes_lines_and_refuses_unknown_mood() {
        let conn = open_memory_db();
        let a = entry(&conn, "a");
        band_flag(&conn, &a, "peaceful", FlagBand::Stretch, "old lines");
        keep_flag(&conn, &a, "peaceful").unwrap();
        let sig = lines_sig(&MoodVocabulary::defaults().prompt_signature("peaceful").unwrap());
        assert_eq!(flags_for_mood(&conn, "peaceful").unwrap()[0].lines_sig, sig);
        assert!(keep_flag(&conn, &a, "nosuchmood").is_err());
    }
}
