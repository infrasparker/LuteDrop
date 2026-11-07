//! Storage for analysis state: each entry's analysis status and when it was
//! last scored, kept in the `entry_analysis` table.

use rusqlite::{params, Connection, OptionalExtension};

use super::now_millis;
use crate::autotag::plan::EntryFacts;
use crate::models::{AnalysisStatus, EntryAnalysis, EntryType};

/// One library entry as an analysis run sees it.
#[derive(Debug, Clone)]
pub struct RunEntry {
    pub facts: EntryFacts,
    pub name: String,
    pub file_path: String,
    /// The stored analysis status and the hash it was recorded under, or
    /// `None` if the entry has never been analyzed.
    pub analysis: Option<(AnalysisStatus, Option<String>)>,
}

/// Loads every library entry with its hash, type, file path and analysis
/// status.
pub fn load_run_entries(conn: &Connection) -> Result<Vec<RunEntry>, String> {
    /// One `library_entries` row: id, name, file path, hash, type.
    type EntryRow = (String, String, String, Option<String>, String);
    let entries: Vec<EntryRow> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, file_path, content_hash, entry_type
                 FROM library_entries ORDER BY name",
            )
            .map_err(|e| format!("Failed to prepare entry query: {e}"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
            .map_err(|e| format!("Failed to query entries: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read entries: {e}"))?;
        rows
    };

    let mut out = Vec::with_capacity(entries.len());
    for (id, name, file_path, hash, entry_type) in entries {
        let analysis = get_analysis(conn, &id)?.map(|(a, stored_hash)| (a.status, stored_hash));
        out.push(RunEntry {
            facts: EntryFacts {
                is_music: matches!(EntryType::from_str(&entry_type), EntryType::Music),
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
/// The last scored time is left as it was.
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

/// Stamps the time an entry was last scored. An entry with no analysis row
/// yet gets one, marked `embedded` (only embedded entries are scored).
pub fn mark_scored(conn: &Connection, entry_id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO entry_analysis (library_entry_id, status, analyzed_at)
         VALUES (?1, 'embedded', ?2)
         ON CONFLICT(library_entry_id) DO UPDATE SET analyzed_at = excluded.analyzed_at",
        params![entry_id, now_millis()],
    )
    .map_err(|e| format!("Failed to save scored time: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{library, open_memory_db};

    fn new_entry(conn: &Connection) -> String {
        library::insert_library_entry(conn, "/a.mp3", None, "A", None, None, None, "music").unwrap()
    }

    #[test]
    fn analysis_round_trips_and_keeps_scored_time_on_status_change() {
        let conn = open_memory_db();
        let id = new_entry(&conn);
        assert!(get_analysis(&conn, &id).unwrap().is_none());

        set_analysis(&conn, &id, Some("h1"), AnalysisStatus::Pending, None).unwrap();
        let (a, hash) = get_analysis(&conn, &id).unwrap().unwrap();
        assert_eq!(a.status, AnalysisStatus::Pending);
        assert_eq!(a.scored_at, None);
        assert_eq!(hash.as_deref(), Some("h1"));

        mark_scored(&conn, &id).unwrap();
        set_analysis(&conn, &id, Some("h1"), AnalysisStatus::Failed, Some("boom")).unwrap();
        let (a, _) = get_analysis(&conn, &id).unwrap().unwrap();
        assert_eq!(a.status, AnalysisStatus::Failed);
        assert_eq!(a.error.as_deref(), Some("boom"));
        assert!(a.scored_at.is_some(), "scored time survives a status change");
    }

    #[test]
    fn mark_scored_creates_an_embedded_row() {
        let conn = open_memory_db();
        let id = new_entry(&conn);
        mark_scored(&conn, &id).unwrap();
        let (a, _) = get_analysis(&conn, &id).unwrap().unwrap();
        assert_eq!(a.status, AnalysisStatus::Embedded);
        assert!(a.scored_at.is_some());
    }

    #[test]
    fn run_entries_carry_hash_type_and_analysis() {
        let conn = open_memory_db();
        let id = library::insert_library_entry(&conn, "/a.mp3", Some("h-a"), "A", None, None, None, "music")
            .unwrap();
        let amb = library::insert_library_entry(&conn, "/b.mp3", None, "B", None, None, None, "ambiance")
            .unwrap();
        set_analysis(&conn, &id, Some("h-old"), AnalysisStatus::Embedded, None).unwrap();

        let entries = load_run_entries(&conn).unwrap();
        let a = entries.iter().find(|e| e.facts.id == id).unwrap();
        assert_eq!(a.facts.hash.as_deref(), Some("h-a"));
        assert!(a.facts.is_music);
        assert_eq!(a.file_path, "/a.mp3");
        assert_eq!(a.analysis, Some((AnalysisStatus::Embedded, Some("h-old".to_string()))));
        let b = entries.iter().find(|e| e.facts.id == amb).unwrap();
        assert!(!b.facts.is_music);
        assert_eq!(b.analysis, None);
    }
}
