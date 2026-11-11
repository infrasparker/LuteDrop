//! One-off import of the hand-kept case-study file (`cases[]` with `high`,
//! `maybe`, `stretch` and `low` mood lists per track) as flags, so the first
//! auto-tune starts from the notes already made. A mood the case lists as
//! `settled` is one whose target was let go of, so it gets no flag on that
//! track whatever else lists it. Everything else in the file (`top`, `above`,
//! `orderings`, the logs) is ignored.

use std::collections::BTreeSet;

use rusqlite::Connection;
use serde_json::Value;

use crate::database::mood_flags::{lines_sig, set_flag};
use crate::database::settings;
use crate::models::{CaseImportReport, FlagBand, FlagBound, FlagKind};

/// A flag the import will write, before it is matched to the database.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedFlag {
    pub entry_id: String,
    pub mood: String,
    pub band: FlagBand,
    pub bound: FlagBound,
}

/// Lowercase, non-breaking spaces as spaces, runs of whitespace as one.
fn normalize(name: &str) -> String {
    name.replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The entry for a track name: an exact (normalized) match first, otherwise a
/// substring match, but only when exactly one entry matches, so a short name
/// never lands on the wrong song.
fn find_entry<'a>(track: &str, names: &'a [(String, String)]) -> Option<&'a str> {
    let want = normalize(track);
    if want.is_empty() {
        return None;
    }
    let normalized: Vec<String> = names.iter().map(|(_, n)| normalize(n)).collect();
    if let Some(i) = normalized.iter().position(|n| *n == want) {
        return Some(&names[i].0);
    }
    let mut hits = normalized.iter().enumerate().filter(|(_, n)| n.contains(&want));
    match (hits.next(), hits.next()) {
        (Some((i, _)), None) => Some(&names[i].0),
        _ => None,
    }
}

fn mood_list(case: &Value, key: &str) -> Vec<String> {
    case.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// The band and bound for the labels one track has on one mood. When labels
/// disagree the strongest claim wins: high, then maybe+stretch, maybe,
/// stretch, low.
fn band_for(high: bool, maybe: bool, stretch: bool, low: bool) -> Option<(FlagBand, FlagBound)> {
    if high {
        Some((FlagBand::Understandable, FlagBound::AtLeast))
    } else if maybe && stretch {
        Some((FlagBand::Stretch, FlagBound::In))
    } else if maybe {
        Some((FlagBand::Stretch, FlagBound::AtLeast))
    } else if stretch {
        Some((FlagBand::Stretch, FlagBound::AtMost))
    } else if low {
        Some((FlagBand::No, FlagBound::In))
    } else {
        None
    }
}

/// Reads the case-study JSON and plans the flags to write. `names` are the
/// library's (entry id, name) pairs and `mood_ids` the vocabulary's ids. A
/// mood in a case's `settled` list writes no flag for that track; it counts in
/// `settled_skipped`, not as imported or unknown.
pub fn plan_import(
    json: &str,
    names: &[(String, String)],
    mood_ids: &[String],
) -> Result<(Vec<ImportedFlag>, CaseImportReport), String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| format!("That is not a readable case-study file: {e}"))?;
    let cases = doc
        .get("cases")
        .and_then(Value::as_array)
        .ok_or("That case-study file has no \"cases\" list")?;

    let mut flags = Vec::new();
    let mut unmatched = Vec::new();
    let mut skipped = BTreeSet::new();
    let mut settled_skipped = 0;
    for case in cases {
        let Some(track) = case.get("track").and_then(Value::as_str) else { continue };
        let (high, maybe, stretch, low, settled) = (
            mood_list(case, "high"),
            mood_list(case, "maybe"),
            mood_list(case, "stretch"),
            mood_list(case, "low"),
            mood_list(case, "settled"),
        );
        let mut moods: Vec<&String> = high.iter().chain(&maybe).chain(&stretch).chain(&low).collect();
        moods.sort();
        moods.dedup();
        let before = moods.len();
        moods.retain(|m| !settled.contains(m));
        // Counted whether or not the track is found below.
        settled_skipped += before - moods.len();
        if moods.is_empty() {
            continue;
        }
        let Some(entry_id) = find_entry(track, names) else {
            unmatched.push(track.to_string());
            continue;
        };
        for mood in moods {
            if !mood_ids.contains(mood) {
                skipped.insert(mood.clone());
                continue;
            }
            if let Some((band, bound)) =
                band_for(high.contains(mood), maybe.contains(mood), stretch.contains(mood), low.contains(mood))
            {
                flags.push(ImportedFlag { entry_id: entry_id.to_string(), mood: mood.clone(), band, bound });
            }
        }
    }
    let report = CaseImportReport {
        imported: flags.len(),
        unmatched,
        skipped_moods: skipped.into_iter().collect(),
        settled_skipped,
    };
    Ok((flags, report))
}

/// Plans the import against the database's music entries and vocabulary and
/// writes every flag in one transaction (a flag the song already has for the
/// mood is replaced).
pub fn import_into(conn: &Connection, json: &str) -> Result<CaseImportReport, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM library_entries WHERE entry_type = 'music' ORDER BY name, id")
        .map_err(|e| format!("Failed to read the library: {e}"))?;
    let names = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| format!("Failed to read the library: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read the library: {e}"))?;
    drop(stmt);

    let vocabulary = settings::load_mood_vocabulary(conn)?;
    let ids: Vec<String> = vocabulary.moods.iter().map(|m| m.id.clone()).collect();
    let (flags, report) = plan_import(json, &names, &ids)?;

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin flag import: {e}"))?;
    for f in &flags {
        let lines = lines_sig(&vocabulary.flag_signature(&f.mood).unwrap_or_default());
        set_flag(&tx, &f.entry_id, &f.mood, FlagKind::Band, Some(f.band), f.bound, None, None, &lines)?;
    }
    tx.commit().map_err(|e| format!("Failed to commit flag import: {e}"))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::mood_flags::flags_for_mood;
    use crate::database::{library, open_memory_db};

    fn names(list: &[&str]) -> Vec<(String, String)> {
        list.iter().enumerate().map(|(i, n)| (format!("id{i}"), n.to_string())).collect()
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn flag_for<'a>(flags: &'a [ImportedFlag], mood: &str) -> &'a ImportedFlag {
        flags.iter().find(|f| f.mood == mood).unwrap_or_else(|| panic!("no flag for {mood}"))
    }

    #[test]
    fn import_maps_labels_to_bands_and_bounds() {
        let json = r#"{"cases":[{"track":"A","high":["peaceful"],"maybe":["wondrous","joyful"],
            "stretch":["grim","joyful"],"low":["battle"],"settled":["quirky"],"top":"peaceful",
            "above":[["peaceful","grim"]]}],"orderings":[["A","B","peaceful"]],"_log":["x"]}"#;
        let moods = ids(&["peaceful", "wondrous", "joyful", "grim", "battle", "quirky"]);
        let (flags, report) = plan_import(json, &names(&["A"]), &moods).unwrap();
        let got = |m: &str| (flag_for(&flags, m).band, flag_for(&flags, m).bound);
        assert_eq!(got("peaceful"), (FlagBand::Understandable, FlagBound::AtLeast));
        assert_eq!(got("wondrous"), (FlagBand::Stretch, FlagBound::AtLeast));
        assert_eq!(got("joyful"), (FlagBand::Stretch, FlagBound::In));
        assert_eq!(got("grim"), (FlagBand::Stretch, FlagBound::AtMost));
        assert_eq!(got("battle"), (FlagBand::No, FlagBound::In));
        assert!(flags.iter().all(|f| f.mood != "quirky"), "settled alone imports no flag");
        assert_eq!(
            report,
            CaseImportReport { imported: 5, unmatched: vec![], skipped_moods: vec![], settled_skipped: 0 }
        );
    }

    #[test]
    fn import_skips_settled_moods() {
        // `settled` lists the moods whose targets were let go: no flag for them,
        // even when the same mood is also listed as high or maybe.
        let json = r#"{"cases":[
            {"track":"Dirtmouth","high":["peaceful","sorrowful"],"settled":["peaceful"]},
            {"track":"17 Roots","high":["mysterious"],"maybe":["wondrous","sorrowful"],
             "settled":["sorrowful","wondrous","gloomy"]}]}"#;
        let moods = ids(&["peaceful", "sorrowful", "mysterious", "wondrous"]);
        let (flags, report) = plan_import(json, &names(&["Dirtmouth", "17 Roots"]), &moods).unwrap();
        let got: Vec<(&str, &str)> = flags.iter().map(|f| (f.entry_id.as_str(), f.mood.as_str())).collect();
        assert_eq!(got, vec![("id0", "sorrowful"), ("id1", "mysterious")]);
        assert_eq!(report.imported, 2);
        // Counted once per (track, mood) skipped; "gloomy" is in no label list, so it is not counted.
        assert_eq!(report.settled_skipped, 3);
        assert!(report.unmatched.is_empty());
        assert!(report.skipped_moods.is_empty(), "a settled mood is not reported as unknown");
    }

    #[test]
    fn import_counts_settled_moods_whether_or_not_the_track_is_found() {
        let json = r#"{"cases":[
            {"track":"Missing A","high":["sorrowful"],"settled":["sorrowful"]},
            {"track":"Missing B","high":["sorrowful","peaceful"],"settled":["peaceful"]},
            {"track":"Here","high":["peaceful"],"settled":["peaceful"]}]}"#;
        let (flags, report) = plan_import(json, &names(&["Here"]), &ids(&["peaceful", "sorrowful"])).unwrap();
        assert!(flags.is_empty());
        assert_eq!(report.settled_skipped, 3);
        // Missing B still had a mood to flag, so it is reported as not found; Missing A had none.
        assert_eq!(report.unmatched, vec!["Missing B".to_string()]);
    }

    #[test]
    fn import_matches_names_with_nbsp_and_substring() {
        let json = r#"{"cases":[
            {"track":"The Hunter's Path","high":["grim"]},
            {"track":"17 Roots","high":["grim"]},
            {"track":"roots","low":["grim"]}]}"#;
        let library = names(&["The\u{a0}Hunter's\u{a0}Path", "17  Roots (remastered)", "Other Roots"]);
        let (flags, report) = plan_import(json, &library, &ids(&["grim"])).unwrap();
        assert_eq!(flags.len(), 2);
        assert_eq!(flags[0].entry_id, "id0");
        assert_eq!(flags[1].entry_id, "id1");
        // "roots" is a substring of two entries, so it is not guessed.
        assert_eq!(report.unmatched, vec!["roots".to_string()]);
    }

    #[test]
    fn import_reports_unmatched_tracks_and_unknown_moods() {
        let json = r#"{"cases":[
            {"track":"Missing Song","high":["grim"]},
            {"track":"Here","high":["peaceful","gloomy"],"low":["spooky"]}]}"#;
        let (flags, report) = plan_import(json, &names(&["Here"]), &ids(&["peaceful"])).unwrap();
        assert_eq!(flags.len(), 1);
        assert_eq!(report.imported, 1);
        assert_eq!(report.unmatched, vec!["Missing Song".to_string()]);
        assert_eq!(report.skipped_moods, vec!["gloomy".to_string(), "spooky".to_string()]);
    }

    #[test]
    fn import_refuses_a_file_that_is_not_a_case_study() {
        assert!(plan_import("not json", &[], &[]).is_err());
        assert!(plan_import("{}", &[], &[]).is_err());
    }

    #[test]
    fn import_into_writes_flags_for_matched_songs() {
        let conn = open_memory_db();
        let a = library::insert_library_entry(&conn, "/a.mp3", None, "Song A", None, None, None, "music").unwrap();
        let json = r#"{"cases":[{"track":"song a","high":["peaceful"]},{"track":"nope","high":["peaceful"]}]}"#;
        let report = import_into(&conn, json).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(report.unmatched, vec!["nope".to_string()]);
        let stored = flags_for_mood(&conn, "peaceful").unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].entry_id, a);
        assert_eq!(stored[0].bound, FlagBound::AtLeast);
    }
}
