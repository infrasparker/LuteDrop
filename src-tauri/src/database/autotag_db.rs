//! Storage for auto-tagging state: each entry's analysis status and its
//! latest tag scores, kept in the `entry_analysis` table.

use rusqlite::{params, Connection, OptionalExtension};

use std::collections::HashMap;

use super::now_millis;
use crate::autotag::plan::EntryFacts;
use crate::models::{AnalysisStatus, EntryAnalysis, EntryTagScores, EntryType, Role, TagScore, TagSource};

/// One library entry as an auto-tagging run sees it.
#[derive(Debug, Clone)]
pub struct RunEntry {
    pub facts: EntryFacts,
    pub name: String,
    pub file_path: String,
    /// The stored analysis status and the hash it was recorded under, or
    /// `None` if the entry has never been analyzed.
    pub analysis: Option<(AnalysisStatus, Option<String>)>,
}

/// Loads every library entry with the user's layer only (yes and no tag
/// votes and user role), its file path and analysis status. The
/// analyzer's own layer is left out, so it never trains on its own guesses.
pub fn load_run_entries(conn: &Connection) -> Result<Vec<RunEntry>, String> {
    // Voted tag names per entry: yes votes and no votes.
    let mut user_tags: HashMap<String, Vec<String>> = HashMap::new();
    let mut rejected: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT lt.library_entry_id, t.name, lt.user_vote FROM library_entry_tags lt
                 JOIN tags t ON t.id = lt.tag_id
                 WHERE lt.user_vote IS NOT NULL
                 ORDER BY t.name",
            )
            .map_err(|e| format!("Failed to prepare entry tag query: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
            })
            .map_err(|e| format!("Failed to query entry tags: {e}"))?;
        for row in rows {
            let (entry_id, name, vote) = row.map_err(|e| format!("Failed to read entry tag: {e}"))?;
            let list = if vote == 1 { &mut user_tags } else { &mut rejected };
            list.entry(entry_id).or_default().push(name);
        }
    }

    /// One `library_entries` row: id, name, file path, hash, type, user
    /// role.
    type EntryRow = (String, String, String, Option<String>, String, Option<String>);
    let entries: Vec<EntryRow> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, file_path, content_hash, entry_type, role_user
                 FROM library_entries ORDER BY name",
            )
            .map_err(|e| format!("Failed to prepare entry query: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
            .map_err(|e| format!("Failed to query entries: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read entries: {e}"))?;
        rows
    };

    let mut out = Vec::with_capacity(entries.len());
    for (id, name, file_path, hash, entry_type, role_user) in entries {
        let analysis = get_analysis(conn, &id)?.map(|(a, stored_hash)| (a.status, stored_hash));
        out.push(RunEntry {
            facts: EntryFacts {
                user_tags: user_tags.remove(&id).unwrap_or_default(),
                rejected: rejected.remove(&id).unwrap_or_default(),
                is_music: matches!(EntryType::from_str(&entry_type), EntryType::Music),
                // Only the user's own role is a training label; never the
                // shown role, which may be the analyzer's or the Non-combat
                // default (see `EntryFacts::user_role`).
                user_role: role_user.as_deref().and_then(Role::from_db_str),
                hash,
                id,
            },
            name,
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

/// Loads the stored scores for an entry, or `None` if it has none. Each tag
/// score's `applied` field is filled from the entry's current tags and the
/// gate's from its current role, so they reflect edits made since the scores
/// were saved.
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

    // The layer each shown tag comes from: a yes vote is the user's, an
    // analyzer tag with no vote is the analyzer's. A no vote shows nothing.
    let mut stmt = conn
        .prepare(
            "SELECT t.name, lt.user_vote
             FROM library_entry_tags lt
             JOIN tags t ON t.id = lt.tag_id
             WHERE lt.library_entry_id = ?1
               AND (lt.user_vote = 1 OR (lt.user_vote IS NULL AND lt.auto_score IS NOT NULL))",
        )
        .map_err(|e| format!("Failed to prepare applied-tag query: {e}"))?;
    let applied: HashMap<String, TagSource> = stmt
        .query_map(params![entry_id], |row| {
            let source = match row.get::<_, Option<i64>>(1)? {
                Some(_) => TagSource::User,
                None => TagSource::Auto,
            };
            Ok((row.get::<_, String>(0)?, source))
        })
        .map_err(|e| format!("Failed to query applied tags: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Failed to read applied tags: {e}"))?;

    let fill = |score: &mut TagScore| score.applied = applied.get(&score.tag).copied();
    scores.tags.iter_mut().for_each(fill);

    // The gate scores the role, so its `applied` is the layer the shown role
    // comes from.
    let role: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT role_user, role_auto FROM library_entries WHERE id = ?1",
            params![entry_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to read role: {e}"))?;
    scores.gate.applied = match role {
        Some((Some(_), _)) => Some(TagSource::User),
        Some((None, Some(_))) => Some(TagSource::Auto),
        _ => None,
    };
    Ok(Some(scores))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotag::plan::build_score_request;
    use crate::database::{library, open_memory_db};
    use crate::models::{AutoTag, SceneConfig};

    fn score(tag: &str, value: f64) -> TagScore {
        TagScore { tag: tag.into(), score: value, threshold: Some(0.5), trusted: true, applied: None }
    }

    fn new_entry(conn: &Connection) -> String {
        library::insert_library_entry(conn, "/a.mp3", None, "A", None, None, None, "music").unwrap()
    }

    fn auto(tag: &str, score: f64) -> AutoTag {
        AutoTag { tag: tag.into(), score }
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
    fn scores_round_trip_marks_applied_layer() {
        let conn = open_memory_db();
        let id = new_entry(&conn);
        assert!(load_scores(&conn, &id).unwrap().is_none());

        let scores = EntryTagScores {
            gate: score("encounter", 0.9),
            tags: vec![
                score("folk", 0.8),
                score("brass", 0.7),
                score("choir", 0.2),
                score("dark", 0.6),
                score("drums", 0.6),
            ],
        };
        save_scores(&conn, &id, &scores).unwrap();
        library::add_tag_to_entry(&conn, &id, "folk").unwrap();
        library::apply_auto_tags(&conn, &id, &[auto("brass", 0.7), auto("dark", 0.6), auto("drums", 0.6)])
            .unwrap();
        library::reject_auto_tags(&conn, std::slice::from_ref(&id), Some(&["dark".to_string()])).unwrap();
        library::keep_auto_tags(&conn, std::slice::from_ref(&id), Some(&["drums".to_string()])).unwrap();

        let loaded = load_scores(&conn, &id).unwrap().unwrap();
        let applied: Vec<_> = loaded.tags.iter().map(|t| (t.tag.as_str(), t.applied)).collect();
        assert_eq!(
            applied,
            vec![
                ("folk", Some(TagSource::User)),
                ("brass", Some(TagSource::Auto)),
                ("choir", None),
                ("dark", None),
                ("drums", Some(TagSource::User)),
            ]
        );
        assert_eq!(loaded.gate.applied, None);
        assert_eq!(loaded.tags[0].score, 0.8);

        let (a, _) = get_analysis(&conn, &id).unwrap().unwrap();
        assert_eq!(a.status, AnalysisStatus::Embedded);
        assert!(a.scored_at.is_some());
    }

    #[test]
    fn facts_loader_reads_votes_not_analyzer_tags() {
        let conn = open_memory_db();
        let id = library::insert_library_entry(&conn, "/a.mp3", Some("h-a"), "A", None, None, None, "music")
            .unwrap();
        let amb = library::insert_library_entry(&conn, "/b.mp3", None, "B", None, None, None, "ambiance")
            .unwrap();
        library::add_tag_to_entry(&conn, &id, "brass").unwrap();
        library::apply_auto_tags(&conn, &id, &[auto("encounter", 0.9), auto("dark", 0.6), auto("brass", 0.8)])
            .unwrap();
        library::reject_auto_tags(&conn, std::slice::from_ref(&id), Some(&["dark".to_string()])).unwrap();
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

        // The analyzer's `encounter` has no vote, so it trains nothing, while
        // the user's yes on `brass` is a positive.
        let scene = SceneConfig { vocabulary: vec!["encounter".into(), "brass".into()] };
        let req = build_score_request(std::slice::from_ref(&a.facts), &scene, &Default::default());
        assert!(req.tags["encounter"].positives.is_empty());
        assert_eq!(req.tags["brass"].positives, vec!["h-a"]);
    }

    #[test]
    fn facts_loader_reads_user_roles_only() {
        let conn = open_memory_db();
        let new = |path: &str| {
            library::insert_library_entry(&conn, path, None, path, None, None, None, "music").unwrap()
        };
        let (user, both, auto, unset) = (new("/u.mp3"), new("/b.mp3"), new("/a.mp3"), new("/n.mp3"));
        library::set_user_role(&conn, &[user.clone(), both.clone()], Some(Role::NonCombat)).unwrap();
        library::apply_auto_role(&conn, &both, Some(Role::Combat), 0.9).unwrap();
        library::apply_auto_role(&conn, &auto, Some(Role::Combat), 0.9).unwrap();

        let entries = load_run_entries(&conn).unwrap();
        let role = |id: &str| entries.iter().find(|e| e.facts.id == id).unwrap().facts.user_role;
        assert_eq!(role(&user), Some(Role::NonCombat));
        assert_eq!(role(&both), Some(Role::NonCombat), "the user layer, not the analyzer's");
        assert_eq!(role(&auto), None, "an auto role is never a user role");
        assert_eq!(role(&unset), None);
    }

    #[test]
    fn gate_score_applied_follows_role_layer() {
        let conn = open_memory_db();
        let id = new_entry(&conn);
        // A user tag named "combat" must not be mistaken for the role.
        library::add_tag_to_entry(&conn, &id, "combat").unwrap();
        let scores = EntryTagScores { gate: score("combat", 0.9), tags: vec![] };
        save_scores(&conn, &id, &scores).unwrap();
        assert_eq!(load_scores(&conn, &id).unwrap().unwrap().gate.applied, None);

        library::apply_auto_role(&conn, &id, Some(Role::Combat), 0.9).unwrap();
        assert_eq!(load_scores(&conn, &id).unwrap().unwrap().gate.applied, Some(TagSource::Auto));

        library::set_user_role(&conn, std::slice::from_ref(&id), Some(Role::NonCombat)).unwrap();
        assert_eq!(load_scores(&conn, &id).unwrap().unwrap().gate.applied, Some(TagSource::User));

        // "Use auto" shows the analyzer's role again.
        library::set_user_role(&conn, std::slice::from_ref(&id), None).unwrap();
        assert_eq!(load_scores(&conn, &id).unwrap().unwrap().gate.applied, Some(TagSource::Auto));
    }
}
