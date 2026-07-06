use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::database::DbError;
use crate::models::LibraryEntryUsage;

// ─── Helper ───────────────────────────────────────────────────────────────────

/// Returns the current time as milliseconds since the Unix epoch, as a string.
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis();
    ms.to_string()
}

// ─── Library Entry CRUD ───────────────────────────────────────────────────────

/// Inserts a new library entry. Returns the new UUID as a `String`.
/// A `file_path` UNIQUE collision surfaces as `DbError::Constraint` so callers
/// (bulk import) can treat it as "skip" without matching an error string.
pub fn insert_library_entry(
    conn: &Connection,
    file_path: &str,
    content_hash: Option<&str>,
    name: &str,
    duration: Option<f64>,
    sample_rate: Option<i64>,
    file_size: Option<i64>,
    entry_type: &str,
) -> Result<String, DbError> {
    let id = Uuid::new_v4().to_string();
    let created_at = chrono_now();

    conn.execute(
        "INSERT INTO library_entries
             (id, file_path, content_hash, name, duration, sample_rate, file_size,
              volume, created_at, entry_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1.0, ?8, ?9)",
        params![id, file_path, content_hash, name, duration, sample_rate, file_size, created_at, entry_type],
    )
    .map_err(DbError::from_sqlite)?;

    Ok(id)
}

/// Returns `(id, name)` for the library entry with the given `content_hash`,
/// or `None` if no such entry exists.
pub fn get_library_entry_by_hash(
    conn: &Connection,
    content_hash: &str,
) -> Result<Option<(String, String)>, String> {
    conn.query_row(
        "SELECT id, name FROM library_entries WHERE content_hash = ?1",
        params![content_hash],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(|e| format!("Failed to query library entry by hash: {e}"))
}

/// Updates the `volume` column for the given entry.
pub fn update_library_entry_volume(
    conn: &Connection,
    id: &str,
    volume: f64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET volume = ?1 WHERE id = ?2",
        params![volume, id],
    )
    .map_err(|e| format!("Failed to update volume: {e}"))?;
    Ok(())
}

/// Updates the `name` column for the given entry.
pub fn update_library_entry_name(
    conn: &Connection,
    id: &str,
    name: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| format!("Failed to update name: {e}"))?;
    Ok(())
}

/// Updates the `entry_type` column for the given entry.
pub fn update_library_entry_type(
    conn: &Connection,
    id: &str,
    entry_type: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET entry_type = ?1 WHERE id = ?2",
        params![entry_type, id],
    )
    .map_err(|e| format!("Failed to update entry type: {e}"))?;
    Ok(())
}

/// Updates the loop window for the given entry.
/// Clamps loop points to existing trim bounds before writing.
/// When both values are None, clears loop points (no validation needed).
/// Returns an error if the clamped values result in loop_start >= loop_end.
#[cfg(test)]
pub fn update_library_entry_loop(
    conn: &Connection,
    id: &str,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    // If both are None, just clear loop points — no validation needed
    if loop_start.is_none() && loop_end.is_none() {
        conn.execute(
            "UPDATE library_entries SET loop_start = NULL, loop_end = NULL WHERE id = ?1",
            params![id],
        ).map_err(|e| e.to_string())?;
        return Ok(());
    }

    let (trim_start, trim_end): (Option<f64>, Option<f64>) = conn.query_row(
        "SELECT trim_start, trim_end FROM library_entries WHERE id = ?1",
        params![id], |r| Ok((r.get(0)?, r.get(1)?))
    ).map_err(|e| e.to_string())?;

    let mut ls = loop_start;
    let mut le = loop_end;

    if let (Some(ref mut loop_s), Some(trim_s)) = (&mut ls, trim_start) {
        if *loop_s < trim_s { *loop_s = trim_s; }
    }
    if let (Some(ref mut loop_e), Some(trim_e)) = (&mut le, trim_end) {
        if *loop_e > trim_e { *loop_e = trim_e; }
    }

    if let (Some(loop_s), Some(loop_e)) = (ls, le) {
        if loop_s >= loop_e {
            return Err("loop_start must be less than loop_end".to_string());
        }
    }

    conn.execute(
        "UPDATE library_entries SET loop_start = ?1, loop_end = ?2 WHERE id = ?3",
        params![ls, le, id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Atomically updates all four marker values for a library entry.
/// Validates constraints and clamps: loop points must fit within trim bounds,
/// starts must be less than ends.
pub fn update_library_entry_markers(
    conn: &Connection,
    id: &str,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    // Validate trim
    if let (Some(ts), Some(te)) = (trim_start, trim_end) {
        if ts >= te {
            return Err("trim_start must be less than trim_end".to_string());
        }
    }

    // Clamp loop to trim bounds
    let mut ls = loop_start;
    let mut le = loop_end;

    let effective_ts = trim_start.unwrap_or(0.0);
    let effective_te = trim_end.unwrap_or(f64::MAX);

    if let Some(ref mut loop_s) = ls {
        if *loop_s < effective_ts { *loop_s = effective_ts; }
    }
    if let Some(ref mut loop_e) = le {
        if effective_te < f64::MAX && *loop_e > effective_te { *loop_e = effective_te; }
    }

    // If loop is invalid after clamping, clear it
    if let (Some(loop_s), Some(loop_e)) = (ls, le) {
        if loop_s >= loop_e {
            ls = None;
            le = None;
        }
    }

    conn.execute(
        "UPDATE library_entries SET trim_start = ?1, trim_end = ?2, loop_start = ?3, loop_end = ?4 WHERE id = ?5",
        params![trim_start, trim_end, ls, le, id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Updates the `duration` and `sample_rate` columns for the given entry.
pub fn update_library_entry_duration(
    conn: &Connection,
    id: &str,
    duration: f64,
    sample_rate: Option<u32>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET duration = ?1, sample_rate = ?2 WHERE id = ?3",
        params![duration, sample_rate.map(|v| v as i64), id],
    )
    .map_err(|e| format!("Failed to update duration: {e}"))?;
    Ok(())
}

/// Returns all file paths from the library.
pub fn get_all_file_paths(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare("SELECT file_path FROM library_entries")
        .map_err(|e| e.to_string())?;
    let paths: Vec<String> = stmt.query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    Ok(paths)
}

/// Updates the `file_path` column for the given entry.
pub fn update_library_entry_file_path(
    conn: &Connection,
    id: &str,
    file_path: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET file_path = ?1 WHERE id = ?2",
        params![file_path, id],
    )
    .map_err(|e| format!("Failed to update file_path: {e}"))?;
    Ok(())
}

/// Returns every clip that references the given library entry, with the
/// enclosing comp and track names for display. Empty vec when unreferenced.
pub fn get_entry_usage(
    conn: &Connection,
    entry_id: &str,
) -> Result<Vec<LibraryEntryUsage>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT clips.id, comps.name, tracks.name
             FROM clips
             JOIN tracks ON tracks.id = clips.track_id
             JOIN comps  ON comps.id  = tracks.comp_id
             WHERE clips.library_entry_id = ?1
             ORDER BY comps.name ASC, tracks.name ASC",
        )
        .map_err(|e| format!("Failed to prepare get_entry_usage: {e}"))?;

    let rows = stmt
        .query_map(params![entry_id], |row| {
            Ok(LibraryEntryUsage {
                clip_id: row.get::<_, String>(0)?,
                comp_name: row.get::<_, String>(1)?,
                track_name: row.get::<_, String>(2)?,
            })
        })
        .map_err(|e| format!("Failed to query entry usage: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect entry usage: {e}"))
}

/// Deletes a library entry and cascade-deletes all clips that reference it.
/// Wrapped in a transaction so the operation is atomic — needed because
/// `PRAGMA foreign_keys=ON` would otherwise reject the library_entry delete
/// while clips still reference it.
pub fn delete_library_entry(conn: &Connection, id: &str) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin delete transaction: {e}"))?;

    tx.execute(
        "DELETE FROM clips WHERE library_entry_id = ?1",
        params![id],
    )
    .map_err(|e| format!("Failed to delete referencing clips: {e}"))?;

    tx.execute(
        "DELETE FROM library_entries WHERE id = ?1",
        params![id],
    )
    .map_err(|e| format!("Failed to delete library entry: {e}"))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit delete transaction: {e}"))?;

    Ok(())
}

// ─── Tag CRUD ─────────────────────────────────────────────────────────────────

/// Normalises `name` to lowercase-trimmed, creates the tag if it doesn't exist,
/// and returns its ID.
pub fn ensure_tag(conn: &Connection, name: &str) -> Result<String, String> {
    let normalised = name.trim().to_lowercase();

    if normalised.is_empty() {
        return Err("Tag name cannot be empty".to_string());
    }

    // Try to find an existing tag first.
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM tags WHERE name = ?1",
            params![normalised],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to query tag: {e}"))?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO tags (id, name) VALUES (?1, ?2)",
        params![id, normalised],
    )
    .map_err(|e| format!("Failed to insert tag: {e}"))?;

    Ok(id)
}

/// Associates a tag (creating it if necessary) with the given library entry.
/// Silently ignores duplicate associations.
pub fn add_tag_to_entry(
    conn: &Connection,
    entry_id: &str,
    tag_name: &str,
) -> Result<(), String> {
    let tag_id = ensure_tag(conn, tag_name)?;

    conn.execute(
        "INSERT OR IGNORE INTO library_entry_tags (library_entry_id, tag_id) VALUES (?1, ?2)",
        params![entry_id, tag_id],
    )
    .map_err(|e| format!("Failed to add tag to entry: {e}"))?;

    Ok(())
}

/// Removes the association between a tag and a library entry.
/// Does nothing if the association doesn't exist.
/// Cleans up the tag row itself if no entries reference it anymore.
#[allow(dead_code)]
pub fn remove_tag_from_entry(
    conn: &Connection,
    entry_id: &str,
    tag_name: &str,
) -> Result<(), String> {
    let normalised = tag_name.trim().to_lowercase();

    conn.execute(
        "DELETE FROM library_entry_tags
         WHERE library_entry_id = ?1
           AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
        params![entry_id, normalised],
    )
    .map_err(|e| format!("Failed to remove tag from entry: {e}"))?;

    // Clean up orphaned tag
    conn.execute(
        "DELETE FROM tags WHERE name = ?1 AND id NOT IN (SELECT tag_id FROM library_entry_tags)",
        params![normalised],
    )
    .map_err(|e| format!("Failed to clean up orphaned tag: {e}"))?;

    Ok(())
}

/// Atomically replaces the full tag set for an entry: one delete, one insert
/// per tag, one orphan sweep — all in a single transaction (the previous
/// remove-then-re-add loop could leave a partial tag set on mid-way failure).
pub fn set_entry_tags(conn: &Connection, entry_id: &str, tags: &[String]) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin tag transaction: {e}"))?;

    conn.execute(
        "DELETE FROM library_entry_tags WHERE library_entry_id = ?1",
        params![entry_id],
    )
    .map_err(|e| format!("Failed to clear entry tags: {e}"))?;

    for tag_name in tags {
        add_tag_to_entry(conn, entry_id, tag_name)?;
    }

    conn.execute(
        "DELETE FROM tags WHERE id NOT IN (SELECT DISTINCT tag_id FROM library_entry_tags)",
        [],
    )
    .map_err(|e| format!("Failed to sweep orphaned tags: {e}"))?;

    tx.commit().map_err(|e| format!("Failed to commit tag transaction: {e}"))?;
    Ok(())
}

/// Returns all tag names associated with the given library entry, sorted.
#[allow(dead_code)]
pub fn get_tags_for_entry(
    conn: &Connection,
    entry_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.name
             FROM tags t
             JOIN library_entry_tags let ON let.tag_id = t.id
             WHERE let.library_entry_id = ?1
             ORDER BY t.name ASC",
        )
        .map_err(|e| format!("Failed to prepare get_tags_for_entry: {e}"))?;

    let names = stmt
        .query_map(params![entry_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to query tags for entry: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect tags for entry: {e}"))?;

    Ok(names)
}

/// Returns all tag names that are associated with at least one library entry, sorted.
pub fn list_all_tags(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT t.name FROM tags t
             JOIN library_entry_tags lt ON lt.tag_id = t.id
             ORDER BY t.name ASC"
        )
        .map_err(|e| format!("Failed to prepare list_all_tags: {e}"))?;

    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to query all tags: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect all tags: {e}"))?;

    Ok(names)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_memory_db;

    // ── 1. insert_and_find_by_hash ────────────────────────────────────────────

    #[test]
    fn insert_and_find_by_hash() {
        let conn = open_memory_db();
        let id = insert_library_entry(
            &conn,
            "/tmp/foo.mp3",
            Some("abc123"),
            "Foo Track",
            Some(120.0),
            Some(44100),
            Some(1024),
            "music",
        )
        .expect("insert should succeed");

        let found = get_library_entry_by_hash(&conn, "abc123")
            .expect("query should succeed");

        let (found_id, found_name) = found.expect("entry should be found");
        assert_eq!(found_id, id);
        assert_eq!(found_name, "Foo Track");
    }

    // ── 2. duplicate_file_path_rejected ──────────────────────────────────────

    #[test]
    fn duplicate_file_path_rejected() {
        let conn = open_memory_db();
        insert_library_entry(
            &conn,
            "/tmp/dup.mp3",
            Some("hash1"),
            "Entry One",
            None,
            None,
            None,
            "music",
        )
        .expect("first insert should succeed");

        let result = insert_library_entry(
            &conn,
            "/tmp/dup.mp3",
            Some("hash2"),
            "Entry Two",
            None,
            None,
            None,
            "music",
        );

        assert!(result.is_err(), "inserting duplicate file_path should fail");
    }

    #[test]
    fn insert_duplicate_file_path_is_typed_constraint_error() {
        use crate::database::DbError;
        let conn = open_memory_db();
        insert_library_entry(&conn, "/dup.mp3", Some("h1"), "A", None, None, None, "music").unwrap();
        // Same file_path (UNIQUE) — must surface as DbError::Constraint, not a
        // formatted string we have to grep.
        let err = insert_library_entry(&conn, "/dup.mp3", Some("h2"), "B", None, None, None, "music")
            .unwrap_err();
        assert!(matches!(err, DbError::Constraint));
    }

    // ── 3. update_volume ─────────────────────────────────────────────────────

    #[test]
    fn update_volume() {
        let conn = open_memory_db();
        let id = insert_library_entry(
            &conn,
            "/tmp/vol.mp3",
            None,
            "Volume Test",
            None,
            None,
            None,
            "music",
        )
        .expect("insert should succeed");

        update_library_entry_volume(&conn, &id, 0.5).expect("update should succeed");

        let stored: f64 = conn
            .query_row(
                "SELECT volume FROM library_entries WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("SELECT should succeed");

        assert!((stored - 0.5).abs() < 1e-9, "volume should be 0.5, got {stored}");
    }

    // ── 4. delete_cascades_to_clips ───────────────────────────────────────────

    /// Build the `config → group → comp → track` hierarchy and return
    /// `(config_id, group_id, comp_id, track_id, comp_name, track_name)`.
    fn make_hierarchy(conn: &Connection, comp_name: &str, track_name: &str) -> (String, String) {
        let config_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Test', '0', 1000)",
            params![config_id],
        )
        .expect("insert config");

        let group_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES (?1, 'G', 0, ?2)",
            params![group_id, config_id],
        )
        .expect("insert group");

        let comp_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, ?3, 0)",
            params![comp_id, group_id, comp_name],
        )
        .expect("insert comp");

        let track_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tracks (id, comp_id, name, sort_order) VALUES (?1, ?2, ?3, 0)",
            params![track_id, comp_id, track_name],
        )
        .expect("insert track");

        (comp_id, track_id)
    }

    #[test]
    fn delete_cascades_to_clips() {
        let conn = open_memory_db();

        let entry_id = insert_library_entry(
            &conn,
            "/tmp/clip-ref.mp3",
            None,
            "Clip Ref Track",
            None,
            None,
            None,
            "music",
        )
        .expect("insert entry");

        let (_comp_id, track_id) = make_hierarchy(&conn, "Thunder Storm", "Rain");

        let clip_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO clips (id, track_id, library_entry_id, sort_order)
             VALUES (?1, ?2, ?3, 0)",
            params![clip_id, track_id, entry_id],
        )
        .expect("insert clip");

        delete_library_entry(&conn, &entry_id).expect("cascade delete should succeed");

        let clip_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM clips WHERE library_entry_id = ?1",
                params![entry_id],
                |row| row.get(0),
            )
            .expect("count clips");
        assert_eq!(clip_count, 0, "referencing clips should be deleted");

        let entry_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM library_entries WHERE id = ?1",
                params![entry_id],
                |row| row.get(0),
            )
            .expect("count entries");
        assert_eq!(entry_count, 0, "entry itself should be deleted");
    }

    // ── 4b. get_entry_usage_returns_locations ─────────────────────────────────

    #[test]
    fn get_entry_usage_returns_locations() {
        let conn = open_memory_db();

        let entry_id = insert_library_entry(
            &conn,
            "/tmp/usage.mp3",
            None,
            "Usage Test",
            None,
            None,
            None,
            "music",
        )
        .expect("insert entry");

        let (_comp_id, track_id) = make_hierarchy(&conn, "Rain Scene", "Drops");

        let clip_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO clips (id, track_id, library_entry_id, sort_order)
             VALUES (?1, ?2, ?3, 0)",
            params![clip_id, track_id, entry_id],
        )
        .expect("insert clip");

        let usage = get_entry_usage(&conn, &entry_id).expect("query usage");
        assert_eq!(usage.len(), 1, "expected one usage row");
        assert_eq!(usage[0].clip_id, clip_id);
        assert_eq!(usage[0].comp_name, "Rain Scene");
        assert_eq!(usage[0].track_name, "Drops");

        let empty = get_entry_usage(&conn, "no-such-entry").expect("query no-such-entry");
        assert!(empty.is_empty(), "unreferenced entry should return empty vec");
    }

    // ── 5. delete_succeeds_when_unreferenced ──────────────────────────────────

    #[test]
    fn delete_succeeds_when_unreferenced() {
        let conn = open_memory_db();
        let id = insert_library_entry(
            &conn,
            "/tmp/orphan.mp3",
            None,
            "Orphan",
            None,
            None,
            None,
            "music",
        )
        .expect("insert should succeed");

        delete_library_entry(&conn, &id).expect("delete should succeed");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM library_entries WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("SELECT should succeed");

        assert_eq!(count, 0, "entry should be gone");
    }

    // ── 6. tag_lifecycle ──────────────────────────────────────────────────────

    #[test]
    fn tag_lifecycle() {
        let conn = open_memory_db();
        let entry_id = insert_library_entry(
            &conn,
            "/tmp/tagged.mp3",
            None,
            "Tagged",
            None,
            None,
            None,
            "music",
        )
        .expect("insert");

        add_tag_to_entry(&conn, &entry_id, "battle").expect("add battle");
        add_tag_to_entry(&conn, &entry_id, "ambient").expect("add ambient");

        let tags = get_tags_for_entry(&conn, &entry_id).expect("get tags");
        assert_eq!(tags, vec!["ambient", "battle"], "tags should be sorted");

        remove_tag_from_entry(&conn, &entry_id, "battle").expect("remove battle");

        let tags_after = get_tags_for_entry(&conn, &entry_id).expect("get tags after remove");
        assert_eq!(tags_after, vec!["ambient"]);
    }

    // ── 7. duplicate_tag_is_idempotent ────────────────────────────────────────

    #[test]
    fn duplicate_tag_is_idempotent() {
        let conn = open_memory_db();
        let entry_id = insert_library_entry(
            &conn,
            "/tmp/idempotent.mp3",
            None,
            "Idempotent",
            None,
            None,
            None,
            "music",
        )
        .expect("insert");

        add_tag_to_entry(&conn, &entry_id, "boss").expect("first add");
        add_tag_to_entry(&conn, &entry_id, "boss").expect("second add should not error");

        let tags = get_tags_for_entry(&conn, &entry_id).expect("get tags");
        assert_eq!(tags.len(), 1, "should only have one 'boss' tag");
        assert_eq!(tags[0], "boss");
    }

    // ── 8. tag_normalization ──────────────────────────────────────────────────

    #[test]
    fn tag_normalization() {
        let conn = open_memory_db();
        let entry_id = insert_library_entry(
            &conn,
            "/tmp/normalize.mp3",
            None,
            "Normalize Test",
            None,
            None,
            None,
            "music",
        )
        .expect("insert");

        add_tag_to_entry(&conn, &entry_id, " Combat ").expect("add padded/cased tag");

        let tags = get_tags_for_entry(&conn, &entry_id).expect("get tags");
        assert_eq!(tags, vec!["combat"], "tag should be lowercased and trimmed");
    }

    // ── 9. loop_clamped_to_trim_bounds ───────────────────────────────────────

    #[test]
    fn loop_clamped_to_trim_bounds() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/c.mp3", Some("c"), "C", None, None, None, "music").unwrap();
        // Set trim bounds directly, then verify loop clamping via update_library_entry_loop
        conn.execute(
            "UPDATE library_entries SET trim_start = 10.0, trim_end = 50.0 WHERE id = ?1",
            params![id],
        ).unwrap();
        update_library_entry_loop(&conn, &id, Some(5.0), Some(60.0)).unwrap();
        let (ls, le): (Option<f64>, Option<f64>) = conn.query_row(
            "SELECT loop_start, loop_end FROM library_entries WHERE id = ?1",
            params![id], |r| Ok((r.get(0)?, r.get(1)?))
        ).unwrap();
        assert!((ls.unwrap() - 10.0).abs() < 0.001);
        assert!((le.unwrap() - 50.0).abs() < 0.001);
    }

    // ── 11. loop_start_must_be_less_than_end ─────────────────────────────────

    #[test]
    fn loop_start_must_be_less_than_end() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/e.mp3", Some("e"), "E", None, None, None, "music").unwrap();
        let result = update_library_entry_loop(&conn, &id, Some(50.0), Some(10.0));
        assert!(result.is_err());
    }

    // ── 12. set_entry_tags_replaces_atomically_and_cleans_orphans ────────────

    #[test]
    fn set_entry_tags_replaces_atomically_and_cleans_orphans() {
        let conn = open_memory_db();
        let e = insert_library_entry(&conn, "/t.mp3", None, "T", None, None, None, "music").unwrap();
        add_tag_to_entry(&conn, &e, "old-only").unwrap();
        add_tag_to_entry(&conn, &e, "kept").unwrap();

        set_entry_tags(&conn, &e, &["kept".to_string(), "new".to_string()]).unwrap();

        assert_eq!(get_tags_for_entry(&conn, &e).unwrap(), vec!["kept", "new"]);
        // "old-only" is orphaned and must be gone from tags entirely:
        let orphan: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE name = 'old-only'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphan, 0);
    }
}
