//! Storage for auto-tagging state: each entry's analysis status and its
//! latest tag scores, kept in the `entry_analysis` table.

use rusqlite::{params, Connection, OptionalExtension};

use std::collections::HashMap;

use super::now_millis;
use crate::autotag::plan::EntryFacts;
use crate::models::{AnalysisStatus, EntryAnalysis, EntryTagScores, EntryType, TagScore, TagSource};

/// One library entry as an auto-tagging run sees it.
#[derive(Debug, Clone)]
pub struct RunEntry {
    pub facts: EntryFacts,
    pub file_path: String,
    /// The stored analysis status and the hash it was recorded under, or
    /// `None` if the entry has never been analyzed.
    pub analysis: Option<(AnalysisStatus, Option<String>)>,
}

/// Loads every library entry with its user tags, rejected tags, file path
/// and analysis status. Auto tags are left out of `user_tags`, so the
/// analyzer never trains on its own guesses.
pub fn load_run_entries(conn: &Connection) -> Result<Vec<RunEntry>, String> {
    // Tag names per entry, split into user tags and rejections.
    let mut user_tags: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT lt.library_entry_id, t.name, lt.source FROM library_entry_tags lt
                 JOIN tags t ON t.id = lt.tag_id
                 ORDER BY t.name",
            )
            .map_err(|e| format!("Failed to prepare entry tag query: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(|e| format!("Failed to query entry tags: {e}"))?;
        for row in rows {
            let (entry_id, name, source) = row.map_err(|e| format!("Failed to read entry tag: {e}"))?;
            if TagSource::from_str(&source) == TagSource::User {
                user_tags.entry(entry_id).or_default().push(name);
            }
        }
    }

    let mut rejected: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT r.library_entry_id, t.name FROM tag_rejections r
                 JOIN tags t ON t.id = r.tag_id
                 ORDER BY t.name",
            )
            .map_err(|e| format!("Failed to prepare rejection query: {e}"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| format!("Failed to query rejections: {e}"))?;
        for row in rows {
            let (entry_id, name) = row.map_err(|e| format!("Failed to read rejection: {e}"))?;
            rejected.entry(entry_id).or_default().push(name);
        }
    }

    let entries: Vec<(String, String, Option<String>, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, file_path, content_hash, entry_type FROM library_entries ORDER BY name")
            .map_err(|e| format!("Failed to prepare entry query: {e}"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .map_err(|e| format!("Failed to query entries: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read entries: {e}"))?;
        rows
    };

    let mut out = Vec::with_capacity(entries.len());
    for (id, file_path, hash, entry_type) in entries {
        let analysis = get_analysis(conn, &id)?.map(|(a, stored_hash)| (a.status, stored_hash));
        out.push(RunEntry {
            facts: EntryFacts {
                user_tags: user_tags.remove(&id).unwrap_or_default(),
                rejected: rejected.remove(&id).unwrap_or_default(),
                is_music: matches!(EntryType::from_str(&entry_type), EntryType::Music),
                hash,
                id,
            },
            file_path,
            analysis,
        });
    }
    Ok(out)
}

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
    use crate::autotag::plan::build_score_request;
    use crate::database::{library, open_memory_db};
    use crate::models::SceneConfig;

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

    #[test]
    fn facts_loader_excludes_auto_tags() {
        let conn = open_memory_db();
        let id = library::insert_library_entry(&conn, "/a.mp3", Some("h-a"), "A", None, None, None, "music")
            .unwrap();
        let amb = library::insert_library_entry(&conn, "/b.mp3", None, "B", None, None, None, "ambiance")
            .unwrap();
        library::add_tag_to_entry(&conn, &id, "brass").unwrap();
        library::add_tag_to_entry(&conn, &id, "encounter").unwrap();
        library::add_tag_to_entry(&conn, &id, "dark").unwrap();
        conn.execute(
            "UPDATE library_entry_tags SET source = 'auto', score = 0.9
             WHERE tag_id = (SELECT id FROM tags WHERE name = 'encounter')",
            [],
        )
        .unwrap();
        // Rejecting through the full-list save records a rejection for `dark`.
        conn.execute(
            "UPDATE library_entry_tags SET source = 'auto', score = 0.6
             WHERE tag_id = (SELECT id FROM tags WHERE name = 'dark')",
            [],
        )
        .unwrap();
        library::reject_auto_tags(&conn, &[id.clone()], Some(&["dark".to_string()])).unwrap();
        set_analysis(&conn, &id, Some("h-old"), AnalysisStatus::Embedded, None).unwrap();

        let entries = load_run_entries(&conn).unwrap();
        let a = entries.iter().find(|e| e.facts.id == id).unwrap();
        assert_eq!(a.facts.user_tags, vec!["brass"]);
        assert_eq!(a.facts.rejected, vec!["dark"]);
        assert_eq!(a.facts.hash.as_deref(), Some("h-a"));
        assert!(a.facts.is_music);
        assert_eq!(a.file_path, "/a.mp3");
        assert_eq!(a.analysis, Some((AnalysisStatus::Embedded, Some("h-old".to_string()))));
        let b = entries.iter().find(|e| e.facts.id == amb).unwrap();
        assert!(!b.facts.is_music);
        assert_eq!(b.analysis, None);

        // The auto `encounter` row does not make the entry a gate positive.
        let req = build_score_request(&[a.facts.clone()], &SceneConfig::default(), &Default::default());
        assert!(req.gate.positives.is_empty());
    }
}
