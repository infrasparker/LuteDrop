use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

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
pub fn insert_library_entry(
    conn: &Connection,
    file_path: &str,
    content_hash: Option<&str>,
    name: &str,
    duration: Option<f64>,
    sample_rate: Option<i64>,
    file_size: Option<i64>,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let created_at = chrono_now();

    conn.execute(
        "INSERT INTO library_entries
             (id, file_path, content_hash, name, duration, sample_rate, file_size,
              volume, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1.0, ?8)",
        params![id, file_path, content_hash, name, duration, sample_rate, file_size, created_at],
    )
    .map_err(|e| format!("Failed to insert library entry: {e}"))?;

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

/// Updates the trim window for the given entry.
#[allow(dead_code)]
pub fn update_library_entry_trim(
    conn: &Connection,
    id: &str,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET trim_start = ?1, trim_end = ?2 WHERE id = ?3",
        params![trim_start, trim_end, id],
    )
    .map_err(|e| format!("Failed to update trim: {e}"))?;
    Ok(())
}

/// Updates the loop window for the given entry.
pub fn update_library_entry_loop(
    conn: &Connection,
    id: &str,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET loop_start = ?1, loop_end = ?2 WHERE id = ?3",
        params![loop_start, loop_end, id],
    )
    .map_err(|e| format!("Failed to update loop: {e}"))?;
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

/// Probes all library entries with a NULL duration and fills them in.
/// Returns the number of entries updated.
pub fn backfill_durations(conn: &Connection) -> Result<usize, String> {
    let mut stmt = conn
        .prepare("SELECT id, file_path FROM library_entries WHERE duration IS NULL")
        .map_err(|e| format!("Failed to prepare backfill query: {e}"))?;

    let entries: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("Failed to query entries for backfill: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    let mut updated = 0;
    for (id, file_path) in &entries {
        if let Some(duration) = crate::audio::engine::probe_duration(file_path) {
            update_library_entry_duration(conn, id, duration, None)?;
            updated += 1;
        }
    }
    Ok(updated)
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

/// Deletes a library entry.
/// Fails with a descriptive error if any clips still reference the entry.
pub fn delete_library_entry(conn: &Connection, id: &str) -> Result<(), String> {
    let ref_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM clips WHERE library_entry_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to count clip references: {e}"))?;

    if ref_count > 0 {
        return Err(format!(
            "Cannot delete library entry: still referenced by {ref_count} clip(s)"
        ));
    }

    conn.execute(
        "DELETE FROM library_entries WHERE id = ?1",
        params![id],
    )
    .map_err(|e| format!("Failed to delete library entry: {e}"))?;

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

    Ok(())
}

/// Returns all tag names associated with the given library entry, sorted.
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

/// Returns all tag names in the database, sorted.
pub fn list_all_tags(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM tags ORDER BY name ASC")
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
        );

        assert!(result.is_err(), "inserting duplicate file_path should fail");
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

    // ── 4. delete_blocked_by_clips ────────────────────────────────────────────

    #[test]
    fn delete_blocked_by_clips() {
        let conn = open_memory_db();

        // Build the full hierarchy: group → comp → track → clip
        let entry_id = insert_library_entry(
            &conn,
            "/tmp/clip-ref.mp3",
            None,
            "Clip Ref Track",
            None,
            None,
            None,
        )
        .expect("insert entry");

        let group_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO groups (id, name, sort_order) VALUES (?1, 'G', 0)",
            params![group_id],
        )
        .expect("insert group");

        let comp_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, 'C', 0)",
            params![comp_id, group_id],
        )
        .expect("insert comp");

        let track_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tracks (id, comp_id, name, sort_order) VALUES (?1, ?2, 'T', 0)",
            params![track_id, comp_id],
        )
        .expect("insert track");

        let clip_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO clips (id, track_id, library_entry_id, sort_order)
             VALUES (?1, ?2, ?3, 0)",
            params![clip_id, track_id, entry_id],
        )
        .expect("insert clip");

        let result = delete_library_entry(&conn, &entry_id);
        assert!(result.is_err(), "delete should be blocked");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("1 clip(s)"),
            "error should mention clip count, got: {msg}"
        );
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
        )
        .expect("insert");

        add_tag_to_entry(&conn, &entry_id, " Combat ").expect("add padded/cased tag");

        let tags = get_tags_for_entry(&conn, &entry_id).expect("get tags");
        assert_eq!(tags, vec!["combat"], "tag should be lowercased and trimmed");
    }
}
