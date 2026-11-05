//! Storage for auto-tagging state: each entry's analysis status and its
//! latest tag scores, kept in the `entry_analysis` table.

use rusqlite::{params, Connection, OptionalExtension};

use super::now_millis;
use crate::models::{AnalysisStatus, EntryAnalysis, EntryTagScores, TagScore, TagSource};

/// Returns the analysis state of an entry and the content hash it was
/// analyzed under, or `None` if the entry has no analysis row.
pub fn get_analysis(
    conn: &Connection,
    entry_id: &str,
) -> Result<Option<(EntryAnalysis, Option<String>)>, String> {
    conn.query_row(
        "SELECT status, error, analyzed_at, content_hash
         FROM entry_analysis WHERE library_entry_id = ?1",
        params![entry_id],
        |row| {
            let status: String = row.get(0)?;
            let analysis = EntryAnalysis {
                status: AnalysisStatus::from_str(&status),
                error: row.get(1)?,
                scored_at: row.get(2)?,
            };
            Ok((analysis, row.get::<_, Option<String>>(3)?))
        },
    )
    .optional()
    .map_err(|e| format!("Failed to read analysis: {e}"))
}

/// Records the analysis status of an entry, creating the row if needed.
/// Stored scores and the last scored time are left as they were.
pub fn set_analysis(
    conn: &Connection,
    entry_id: &str,
    hash: Option<&str>,
    status: AnalysisStatus,
    error: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO entry_analysis (library_entry_id, content_hash, status, error)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(library_entry_id) DO UPDATE SET
             content_hash = excluded.content_hash,
             status = excluded.status,
             error = excluded.error",
        params![entry_id, hash, status.as_str(), error],
    )
    .map_err(|e| format!("Failed to save analysis: {e}"))?;
    Ok(())
}

/// Stores the scores for an entry and stamps the scored time. An entry that
/// has scores is marked `embedded` if it had no analysis row yet.
pub fn save_scores(conn: &Connection, entry_id: &str, scores: &EntryTagScores) -> Result<(), String> {
    let json = serde_json::to_string(scores).map_err(|e| format!("Failed to encode scores: {e}"))?;
    conn.execute(
        "INSERT INTO entry_analysis (library_entry_id, status, analyzed_at, scores_json)
         VALUES (?1, 'embedded', ?2, ?3)
         ON CONFLICT(library_entry_id) DO UPDATE SET
             analyzed_at = excluded.analyzed_at,
             scores_json = excluded.scores_json",
        params![entry_id, now_millis(), json],
    )
    .map_err(|e| format!("Failed to save scores: {e}"))?;
    Ok(())
}

/// Loads the stored scores for an entry, or `None` if it has none. Each
/// score's `applied` field is filled from the entry's current tags, so it
/// reflects edits made since the scores were saved.
pub fn load_scores(conn: &Connection, entry_id: &str) -> Result<Option<EntryTagScores>, String> {
    let json: Option<String> = conn
        .query_row(
            "SELECT scores_json FROM entry_analysis WHERE library_entry_id = ?1",
            params![entry_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to read scores: {e}"))?
        .flatten();
    let Some(json) = json else {
        return Ok(None);
    };
    let mut scores: EntryTagScores =
        serde_json::from_str(&json).map_err(|e| format!("Failed to decode scores: {e}"))?;

    let mut stmt = conn
        .prepare(
            "SELECT t.name, let.source
             FROM library_entry_tags let
             JOIN tags t ON t.id = let.tag_id
             WHERE let.library_entry_id = ?1",
        )
        .map_err(|e| format!("Failed to prepare applied-tag query: {e}"))?;
    let applied: std::collections::HashMap<String, TagSource> = stmt
        .query_map(params![entry_id], |row| {
            Ok((row.get::<_, String>(0)?, TagSource::from_str(&row.get::<_, String>(1)?)))
        })
        .map_err(|e| format!("Failed to query applied tags: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Failed to read applied tags: {e}"))?;

    let fill = |score: &mut TagScore| score.applied = applied.get(&score.tag).copied();
    fill(&mut scores.gate);
    scores.tags.iter_mut().for_each(fill);
    Ok(Some(scores))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{library, open_memory_db};

    fn score(tag: &str, value: f64) -> TagScore {
        TagScore { tag: tag.into(), score: value, threshold: Some(0.5), trusted: true, applied: None }
    }

    fn new_entry(conn: &Connection) -> String {
        library::insert_library_entry(conn, "/a.mp3", None, "A", None, None, None, "music").unwrap()
    }

    #[test]
    fn analysis_round_trips_and_keeps_scores_on_status_change() {
        let conn = open_memory_db();
        let id = new_entry(&conn);
        assert!(get_analysis(&conn, &id).unwrap().is_none());

        set_analysis(&conn, &id, Some("h1"), AnalysisStatus::Pending, None).unwrap();
        let (a, hash) = get_analysis(&conn, &id).unwrap().unwrap();
        assert_eq!(a.status, AnalysisStatus::Pending);
        assert_eq!(a.scored_at, None);
        assert_eq!(hash.as_deref(), Some("h1"));

        let scores = EntryTagScores { gate: score("encounter", 0.9), tags: vec![] };
        save_scores(&conn, &id, &scores).unwrap();
        set_analysis(&conn, &id, Some("h1"), AnalysisStatus::Failed, Some("boom")).unwrap();
        let (a, _) = get_analysis(&conn, &id).unwrap().unwrap();
        assert_eq!(a.status, AnalysisStatus::Failed);
        assert_eq!(a.error.as_deref(), Some("boom"));
        assert!(a.scored_at.is_some(), "scored time survives a status change");
        assert!(load_scores(&conn, &id).unwrap().is_some(), "scores survive a status change");
    }

    #[test]
    fn scores_round_trip_marks_applied_source() {
        let conn = open_memory_db();
        let id = new_entry(&conn);
        assert!(load_scores(&conn, &id).unwrap().is_none());

        let scores = EntryTagScores {
            gate: score("encounter", 0.9),
            tags: vec![score("folk", 0.8), score("brass", 0.7), score("choir", 0.2)],
        };
        save_scores(&conn, &id, &scores).unwrap();
        library::add_tag_to_entry(&conn, &id, "folk").unwrap();
        library::add_tag_to_entry(&conn, &id, "brass").unwrap();
        conn.execute(
            "UPDATE library_entry_tags SET source = 'auto', score = 0.7
             WHERE tag_id = (SELECT id FROM tags WHERE name = 'brass')",
            [],
        )
        .unwrap();

        let loaded = load_scores(&conn, &id).unwrap().unwrap();
        let applied: Vec<_> = loaded.tags.iter().map(|t| t.applied).collect();
        assert_eq!(applied, vec![Some(TagSource::User), Some(TagSource::Auto), None]);
        assert_eq!(loaded.gate.applied, None);
        assert_eq!(loaded.tags[0].score, 0.8);

        let (a, _) = get_analysis(&conn, &id).unwrap().unwrap();
        assert_eq!(a.status, AnalysisStatus::Embedded);
        assert!(a.scored_at.is_some());
    }
}
