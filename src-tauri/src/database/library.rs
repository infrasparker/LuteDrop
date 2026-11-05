use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::database::now_millis;
use crate::database::DbError;
use crate::models::{AutoTag, LibraryEntryUsage, Role, TagSource};

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
    let created_at = now_millis();

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

/// Updates the `entry_type` column for the given entry. Only music entries
/// are auto-tagged and roles only apply to music, so when an entry stops being
/// music its auto tags are deleted in the same transaction (without
/// rejections, since the user never judged them), orphaned tag rows are swept,
/// and its role is cleared entirely regardless of source (changing the type is
/// itself a user action). User tags and rejections are left alone.
///
/// Returns true if the entry was not music before and is music now.
pub fn update_library_entry_type(
    conn: &Connection,
    id: &str,
    entry_type: &str,
) -> Result<bool, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin entry type transaction: {e}"))?;

    let old_type: Option<String> = conn
        .query_row(
            "SELECT entry_type FROM library_entries WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Failed to read entry type: {e}"))?;
    let Some(old_type) = old_type else {
        return Ok(false);
    };
    let was_music = old_type == "music";
    let is_music = entry_type == "music";

    conn.execute(
        "UPDATE library_entries SET entry_type = ?1 WHERE id = ?2",
        params![entry_type, id],
    )
    .map_err(|e| format!("Failed to update entry type: {e}"))?;

    if was_music && !is_music {
        remove_auto_tags(conn, id)?;
        sweep_orphan_tags(conn)?;
        conn.execute(
            "UPDATE library_entries SET role = NULL, role_source = NULL, role_score = NULL
             WHERE id = ?1",
            params![id],
        )
        .map_err(|e| format!("Failed to clear role: {e}"))?;
    }

    tx.commit()
        .map_err(|e| format!("Failed to commit entry type transaction: {e}"))?;
    Ok(!was_music && is_music)
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

/// Finds the entry (if any) that already owns `file_path`. Returns (id, name).
/// `file_path` is UNIQUE, so at most one entry can match.
pub fn find_entry_name_by_file_path(
    conn: &Connection,
    file_path: &str,
) -> Result<Option<(String, String)>, String> {
    conn.query_row(
        "SELECT id, name FROM library_entries WHERE file_path = ?1",
        params![file_path],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(|e| format!("Failed to look up file_path owner: {e}"))
}

/// Refreshes the facts about a file after a relink. Hash and size are always
/// written; the duration is only written when we managed to measure one
/// (`COALESCE` keeps the old value when `duration` is None).
pub fn update_library_entry_file_info(
    conn: &Connection,
    id: &str,
    content_hash: &str,
    file_size: i64,
    duration: Option<f64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET content_hash = ?1, file_size = ?2, duration = COALESCE(?3, duration) WHERE id = ?4",
        params![content_hash, file_size, duration, id],
    )
    .map_err(|e| format!("Failed to update file info: {e}"))?;
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
#[cfg(test)]
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
        "DELETE FROM tags WHERE name = ?1
           AND id NOT IN (SELECT tag_id FROM library_entry_tags)
           AND id NOT IN (SELECT tag_id FROM tag_rejections)",
        params![normalised],
    )
    .map_err(|e| format!("Failed to clean up orphaned tag: {e}"))?;

    Ok(())
}

/// Deletes tag rows that no entry uses and that no entry has rejected.
/// Rejections point at tag rows, so a tag with a rejection must stay.
const SWEEP_ORPHAN_TAGS: &str = "DELETE FROM tags
     WHERE id NOT IN (SELECT tag_id FROM library_entry_tags)
       AND id NOT IN (SELECT tag_id FROM tag_rejections)";

pub fn sweep_orphan_tags(conn: &Connection) -> Result<(), String> {
    conn.execute(SWEEP_ORPHAN_TAGS, [])
        .map_err(|e| format!("Failed to sweep orphaned tags: {e}"))?;
    Ok(())
}

/// Returns the `source` ('user' or 'auto') of the entry's row for this tag,
/// or None if the entry doesn't have the tag.
fn stored_source(conn: &Connection, entry_id: &str, tag_id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT source FROM library_entry_tags WHERE library_entry_id = ?1 AND tag_id = ?2",
        params![entry_id, tag_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| format!("Failed to read tag source: {e}"))
}

/// Records that the user rejected this tag on this entry.
fn insert_rejection(conn: &Connection, entry_id: &str, tag_id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO tag_rejections (library_entry_id, tag_id, created_at)
         VALUES (?1, ?2, ?3)",
        params![entry_id, tag_id, now_millis()],
    )
    .map_err(|e| format!("Failed to record tag rejection: {e}"))?;
    Ok(())
}

/// Adds a tag the entry doesn't have yet as a user tag and clears any
/// rejection of it, since the user's latest action wins. A tag the entry
/// already has (user or auto) is left exactly as stored.
fn add_new_user_tag(conn: &Connection, entry_id: &str, tag_id: &str) -> Result<(), String> {
    if stored_source(conn, entry_id, tag_id)?.is_some() {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO library_entry_tags (library_entry_id, tag_id, source, score)
         VALUES (?1, ?2, ?3, NULL)",
        params![entry_id, tag_id, TagSource::User.as_str()],
    )
    .map_err(|e| format!("Failed to add tag to entry: {e}"))?;
    conn.execute(
        "DELETE FROM tag_rejections WHERE library_entry_id = ?1 AND tag_id = ?2",
        params![entry_id, tag_id],
    )
    .map_err(|e| format!("Failed to clear tag rejection: {e}"))?;
    Ok(())
}

/// Removes the entry's row for this tag. If it was an auto tag, the removal
/// is also recorded as a rejection; removing a user tag is plain editing.
fn remove_tag_row(conn: &Connection, entry_id: &str, tag_id: &str) -> Result<(), String> {
    let source = stored_source(conn, entry_id, tag_id)?;
    conn.execute(
        "DELETE FROM library_entry_tags WHERE library_entry_id = ?1 AND tag_id = ?2",
        params![entry_id, tag_id],
    )
    .map_err(|e| format!("Failed to remove tag from entry: {e}"))?;
    if source.as_deref().map(TagSource::from_str) == Some(TagSource::Auto) {
        insert_rejection(conn, entry_id, tag_id)?;
    }
    Ok(())
}

/// Deletes every auto tag on the entry, without recording rejections, since
/// the user never judged them.
fn remove_auto_tags(conn: &Connection, entry_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM library_entry_tags WHERE library_entry_id = ?1 AND source = ?2",
        params![entry_id, TagSource::Auto.as_str()],
    )
    .map_err(|e| format!("Failed to remove auto tags: {e}"))?;
    Ok(())
}

/// Atomically saves the full tag list for an entry as a diff against the
/// stored rows, in one transaction:
/// - a listed tag that is stored as auto stays auto, score unchanged;
/// - a listed tag the entry doesn't have is added as a user tag;
/// - a stored auto tag that is missing from the list is removed and rejected;
/// - a stored user tag that is missing from the list is simply removed.
///
/// `seen_auto_tags` names the auto tags the editor was showing when the list
/// was built. When it is `Some`, a missing auto tag is only removed and
/// rejected if it is in that list; an auto tag added after the editor loaded
/// (by a background run) is left alone. With `None`, every missing auto tag
/// is removed and rejected.
pub fn set_entry_tags(
    conn: &Connection,
    entry_id: &str,
    tags: &[String],
    seen_auto_tags: Option<&[String]>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin tag transaction: {e}"))?;

    let mut wanted_ids = Vec::new();
    for tag_name in tags {
        wanted_ids.push(ensure_tag(conn, tag_name)?);
    }

    let seen: Option<Vec<String>> =
        seen_auto_tags.map(|names| names.iter().map(|n| n.trim().to_lowercase()).collect());

    // (tag id, tag name, source) for every tag the entry has now.
    let stored: Vec<(String, String, TagSource)> = {
        let mut stmt = conn
            .prepare(
                "SELECT lt.tag_id, t.name, lt.source FROM library_entry_tags lt
                 JOIN tags t ON t.id = lt.tag_id
                 WHERE lt.library_entry_id = ?1",
            )
            .map_err(|e| format!("Failed to prepare stored tag query: {e}"))?;
        let rows = stmt
            .query_map(params![entry_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    TagSource::from_str(&row.get::<_, String>(2)?),
                ))
            })
            .map_err(|e| format!("Failed to query stored tags: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect stored tags: {e}"))?;
        rows
    };

    for (tag_id, name, source) in &stored {
        if wanted_ids.contains(tag_id) {
            continue;
        }
        let unseen_auto = *source == TagSource::Auto
            && seen.as_ref().is_some_and(|seen| !seen.contains(name));
        if !unseen_auto {
            remove_tag_row(conn, entry_id, tag_id)?;
        }
    }

    for tag_id in &wanted_ids {
        // A listed tag stored as auto is left as stored.
        let stored_as_auto = stored
            .iter()
            .any(|(id, _, source)| id == tag_id && *source == TagSource::Auto);
        if !stored_as_auto {
            add_new_user_tag(conn, entry_id, tag_id)?;
        }
    }

    sweep_orphan_tags(conn)?;

    tx.commit().map_err(|e| format!("Failed to commit tag transaction: {e}"))?;
    Ok(())
}

/// Adds `add` to and removes `remove` from every entry in `entry_ids`, in
/// one transaction. Applies the same rules as `set_entry_tags`: removing an
/// auto tag rejects it, adding a tag the entry already has leaves it as
/// stored, and adding a new tag inserts it as a user tag. Orphaned tag rows
/// are swept at the end.
pub fn bulk_edit_entry_tags(
    conn: &Connection,
    entry_ids: &[String],
    add: &[String],
    remove: &[String],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin bulk tag transaction: {e}"))?;

    for entry_id in entry_ids {
        for tag_name in remove {
            let normalised = tag_name.trim().to_lowercase();
            let tag_id: Option<String> = conn
                .query_row(
                    "SELECT id FROM tags WHERE name = ?1",
                    params![normalised],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("Failed to query tag: {e}"))?;
            if let Some(tag_id) = tag_id {
                remove_tag_row(conn, entry_id, &tag_id)?;
            }
        }
        for tag_name in add {
            let tag_id = ensure_tag(conn, tag_name)?;
            add_new_user_tag(conn, entry_id, &tag_id)?;
        }
    }

    sweep_orphan_tags(conn)?;

    tx.commit()
        .map_err(|e| format!("Failed to commit bulk tag transaction: {e}"))?;
    Ok(())
}

/// Returns the ids of the entry's auto tags. With `tags` set, only those
/// names are considered; with `None`, every auto tag on the entry.
fn auto_tag_ids(
    conn: &Connection,
    entry_id: &str,
    tags: Option<&[String]>,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT lt.tag_id, t.name FROM library_entry_tags lt
             JOIN tags t ON t.id = lt.tag_id
             WHERE lt.library_entry_id = ?1 AND lt.source = 'auto'",
        )
        .map_err(|e| format!("Failed to prepare auto tag query: {e}"))?;
    let rows = stmt
        .query_map(params![entry_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Failed to query auto tags: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect auto tags: {e}"))?;

    let wanted: Option<Vec<String>> =
        tags.map(|names| names.iter().map(|n| n.trim().to_lowercase()).collect());
    Ok(rows
        .into_iter()
        .filter(|(_, name)| wanted.as_ref().map_or(true, |w| w.contains(name)))
        .map(|(id, _)| id)
        .collect())
}

/// Turns the entry's matching auto tags into user tags and clears their
/// scores. `tags: None` keeps every auto tag on the entries.
pub fn keep_auto_tags(
    conn: &Connection,
    entry_ids: &[String],
    tags: Option<&[String]>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin keep transaction: {e}"))?;

    for entry_id in entry_ids {
        for tag_id in auto_tag_ids(conn, entry_id, tags)? {
            conn.execute(
                "UPDATE library_entry_tags SET source = ?3, score = NULL
                 WHERE library_entry_id = ?1 AND tag_id = ?2",
                params![entry_id, tag_id, TagSource::User.as_str()],
            )
            .map_err(|e| format!("Failed to keep auto tag: {e}"))?;
        }
    }

    tx.commit().map_err(|e| format!("Failed to commit keep transaction: {e}"))?;
    Ok(())
}

/// Removes the entry's matching auto tags and records a rejection for each,
/// so they aren't suggested again. `tags: None` rejects every auto tag on
/// the entries. User tags are never touched.
pub fn reject_auto_tags(
    conn: &Connection,
    entry_ids: &[String],
    tags: Option<&[String]>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin reject transaction: {e}"))?;

    for entry_id in entry_ids {
        for tag_id in auto_tag_ids(conn, entry_id, tags)? {
            remove_tag_row(conn, entry_id, &tag_id)?;
        }
    }

    tx.commit().map_err(|e| format!("Failed to commit reject transaction: {e}"))?;
    Ok(())
}

/// True if the user has rejected this tag on this entry.
pub fn is_rejected(conn: &Connection, entry_id: &str, tag_name: &str) -> Result<bool, String> {
    let normalised = tag_name.trim().to_lowercase();
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM tag_rejections r
             JOIN tags t ON t.id = r.tag_id
             WHERE r.library_entry_id = ?1 AND t.name = ?2)",
        params![entry_id, normalised],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to check tag rejection: {e}"))
}

/// Replaces the entry's auto tags with `desired` and returns how many auto
/// rows were (added, removed). Names the entry holds as user tags or has
/// rejected are skipped, and user rows are never changed. A name that is
/// already an auto tag keeps its row and gets the new score.
///
/// This does not open its own transaction. It reads the entry's current rows
/// on the connection it is given, so it sees whatever the caller's open
/// transaction has already written and must be called inside one. It does
/// not sweep orphaned tag rows; the caller runs `sweep_orphan_tags` once
/// after its last call.
pub fn apply_auto_tags(
    conn: &Connection,
    entry_id: &str,
    desired: &[AutoTag],
) -> Result<(u32, u32), String> {
    // Stored tag ids on this entry, split by source.
    let mut auto_ids: Vec<String> = Vec::new();
    let mut user_ids: Vec<String> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT tag_id, source FROM library_entry_tags WHERE library_entry_id = ?1")
            .map_err(|e| format!("Failed to prepare stored tag query: {e}"))?;
        let rows = stmt
            .query_map(params![entry_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("Failed to query stored tags: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect stored tags: {e}"))?;
        for (tag_id, source) in rows {
            if TagSource::from_str(&source) == TagSource::Auto {
                auto_ids.push(tag_id);
            } else {
                user_ids.push(tag_id);
            }
        }
    }

    // Keep only the desired tags the user hasn't claimed or rejected.
    let mut keep: Vec<(String, f64)> = Vec::new();
    for wanted in desired {
        let tag_id = ensure_tag(conn, &wanted.tag)?;
        if user_ids.contains(&tag_id)
            || keep.iter().any(|(id, _)| *id == tag_id)
            || is_rejected(conn, entry_id, &wanted.tag)?
        {
            continue;
        }
        keep.push((tag_id, wanted.score));
    }

    let mut removed = 0;
    for tag_id in &auto_ids {
        if !keep.iter().any(|(id, _)| id == tag_id) {
            conn.execute(
                "DELETE FROM library_entry_tags WHERE library_entry_id = ?1 AND tag_id = ?2",
                params![entry_id, tag_id],
            )
            .map_err(|e| format!("Failed to remove auto tag: {e}"))?;
            removed += 1;
        }
    }

    let mut added = 0;
    for (tag_id, score) in &keep {
        if auto_ids.contains(tag_id) {
            conn.execute(
                "UPDATE library_entry_tags SET score = ?3
                 WHERE library_entry_id = ?1 AND tag_id = ?2",
                params![entry_id, tag_id, score],
            )
            .map_err(|e| format!("Failed to update auto tag score: {e}"))?;
        } else {
            conn.execute(
                "INSERT INTO library_entry_tags (library_entry_id, tag_id, source, score)
                 VALUES (?1, ?2, ?3, ?4)",
                params![entry_id, tag_id, TagSource::Auto.as_str(), score],
            )
            .map_err(|e| format!("Failed to add auto tag: {e}"))?;
            added += 1;
        }
    }

    Ok((added, removed))
}

/// Sets the role on each entry as a user decision: `role_source` becomes
/// user and any score is dropped. `None` clears the role but keeps the user
/// source, so the analyzer does not fill it in again.
pub fn set_user_role(
    conn: &Connection,
    entry_ids: &[String],
    role: Option<Role>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin role transaction: {e}"))?;

    let role_str = role.map(|r| r.as_db_str());
    for entry_id in entry_ids {
        conn.execute(
            "UPDATE library_entries
             SET role = ?2, role_source = ?3, role_score = NULL
             WHERE id = ?1",
            params![entry_id, role_str, TagSource::User.as_str()],
        )
        .map_err(|e| format!("Failed to set role: {e}"))?;
    }

    tx.commit().map_err(|e| format!("Failed to commit role transaction: {e}"))?;
    Ok(())
}

/// Turns each entry's auto role into a user role and clears its score.
/// Entries without an auto role are left as they are.
pub fn keep_auto_role(conn: &Connection, entry_ids: &[String]) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin keep role transaction: {e}"))?;

    for entry_id in entry_ids {
        conn.execute(
            "UPDATE library_entries SET role_source = ?2, role_score = NULL
             WHERE id = ?1 AND role_source = ?3",
            params![entry_id, TagSource::User.as_str(), TagSource::Auto.as_str()],
        )
        .map_err(|e| format!("Failed to keep auto role: {e}"))?;
    }

    tx.commit().map_err(|e| format!("Failed to commit keep role transaction: {e}"))?;
    Ok(())
}

/// Writes the analyzer's role for an entry and returns whether the row
/// changed. `Some(role)` stores it as an auto role with `score` (the Combat
/// probability); `None` clears an existing auto role. Nothing is written when
/// the user has set or cleared the role (`role_source` is user).
///
/// Like `apply_auto_tags`, this reads the entry's current row on the
/// connection it is given and does not open a transaction, so a role the user
/// set earlier in the caller's transaction is seen and kept.
pub fn apply_auto_role(
    conn: &Connection,
    entry_id: &str,
    role: Option<Role>,
    score: f64,
) -> Result<bool, String> {
    let current: Option<(Option<String>, Option<String>, Option<f64>)> = conn
        .query_row(
            "SELECT role, role_source, role_score FROM library_entries WHERE id = ?1",
            params![entry_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to read role: {e}"))?;

    let Some((current_role, current_source, current_score)) = current else {
        return Ok(false);
    };
    if current_source.as_deref().map(TagSource::from_str) == Some(TagSource::User) {
        return Ok(false);
    }

    // The values the row should hold afterwards.
    let (new_role, new_source, new_score) = match role {
        Some(r) => (Some(r.as_db_str()), Some(TagSource::Auto.as_str()), Some(score)),
        None => (None, None, None),
    };
    if current_role.as_deref() == new_role
        && current_source.as_deref() == new_source
        && current_score == new_score
    {
        return Ok(false);
    }

    conn.execute(
        "UPDATE library_entries SET role = ?2, role_source = ?3, role_score = ?4
         WHERE id = ?1",
        params![entry_id, new_role, new_source, new_score],
    )
    .map_err(|e| format!("Failed to write auto role: {e}"))?;
    Ok(true)
}

/// Returns all tag names associated with the given library entry, sorted.
#[cfg(test)]
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

        set_entry_tags(&conn, &e, &["kept".to_string(), "new".to_string()], None).unwrap();

        assert_eq!(get_tags_for_entry(&conn, &e).unwrap(), vec!["kept", "new"]);
        // "old-only" is orphaned and must be gone from tags entirely:
        let orphan: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE name = 'old-only'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphan, 0);
    }

    #[test]
    fn bulk_edit_entry_tags_adds_and_removes_across_entries() {
        let conn = open_memory_db();
        let a = insert_library_entry(&conn, "/tmp/a.mp3", None, "A", None, None, None, "music")
            .expect("insert a");
        let b = insert_library_entry(&conn, "/tmp/b.mp3", None, "B", None, None, None, "music")
            .expect("insert b");
        let c = insert_library_entry(&conn, "/tmp/c.mp3", None, "C", None, None, None, "music")
            .expect("insert c");

        set_entry_tags(&conn, &a, &["combat".into(), "night".into()], None).expect("tag a");
        set_entry_tags(&conn, &b, &["night".into()], None).expect("tag b");
        set_entry_tags(&conn, &c, &["night".into()], None).expect("tag c");

        // Add "boss" and remove "night" from a and b only; c is untouched.
        bulk_edit_entry_tags(
            &conn,
            &[a.clone(), b.clone()],
            &["Boss ".into()],
            &["night".into()],
        )
        .expect("bulk edit");

        assert_eq!(get_tags_for_entry(&conn, &a).unwrap(), vec!["boss", "combat"]);
        assert_eq!(get_tags_for_entry(&conn, &b).unwrap(), vec!["boss"]);
        assert_eq!(get_tags_for_entry(&conn, &c).unwrap(), vec!["night"]);

        // Removing the last use of a tag sweeps the orphan row.
        bulk_edit_entry_tags(&conn, &[c.clone()], &[], &["night".into()]).expect("bulk remove");
        assert_eq!(list_all_tags(&conn).unwrap(), vec!["boss", "combat"]);
    }

    // ── Auto-tag write semantics ──────────────────────────────────────────────

    fn new_entry(conn: &Connection, path: &str) -> String {
        insert_library_entry(conn, path, None, "T", None, None, None, "music").unwrap()
    }

    /// Stores `tag` on `entry` as an auto tag with the given score.
    fn add_auto(conn: &Connection, entry: &str, tag: &str, score: f64) {
        let tag_id = ensure_tag(conn, tag).unwrap();
        conn.execute(
            "INSERT INTO library_entry_tags (library_entry_id, tag_id, source, score)
             VALUES (?1, ?2, 'auto', ?3)",
            params![entry, tag_id, score],
        )
        .unwrap();
    }

    /// Returns (source, score) for a stored tag row, or None if absent.
    fn row_of(conn: &Connection, entry: &str, tag: &str) -> Option<(String, Option<f64>)> {
        conn.query_row(
            "SELECT lt.source, lt.score FROM library_entry_tags lt
             JOIN tags t ON t.id = lt.tag_id
             WHERE lt.library_entry_id = ?1 AND t.name = ?2",
            params![entry, tag],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn type_change_from_music_removes_auto_tags() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        add_auto(&conn, &e, "dark", 0.7);
        add_tag_to_entry(&conn, &e, "night").unwrap();
        let rejected = vec!["dark".to_string()];
        reject_auto_tags(&conn, &[e.clone()], Some(&rejected[..])).unwrap();
        add_auto(&conn, &e, "dark", 0.7);

        assert!(!update_library_entry_type(&conn, &e, "ambiance").unwrap());

        assert_eq!(row_of(&conn, &e, "brass"), None);
        assert_eq!(row_of(&conn, &e, "dark"), None);
        assert_eq!(row_of(&conn, &e, "night"), Some(("user".into(), None)));
        // The rejection stays, so its tag row survives the sweep. "brass" is
        // used by nothing and is swept.
        assert!(is_rejected(&conn, &e, "dark").unwrap());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tag_rejections"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 2);
    }

    #[test]
    fn type_change_to_music_reports_it_and_keeps_tags() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        update_library_entry_type(&conn, &e, "ambiance").unwrap();
        assert!(update_library_entry_type(&conn, &e, "music").unwrap());
    }

    #[test]
    fn music_to_music_type_change_keeps_auto_tags() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);

        assert!(!update_library_entry_type(&conn, &e, "music").unwrap());

        assert_eq!(row_of(&conn, &e, "brass"), Some(("auto".into(), Some(0.8))));
    }

    #[test]
    fn set_tags_keeps_auto_rows_auto_when_resent() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        set_entry_tags(&conn, &e, &["brass".into(), "night".into()], None).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), Some(("auto".into(), Some(0.8))));
        assert_eq!(row_of(&conn, &e, "night"), Some(("user".into(), None)));
    }

    #[test]
    fn set_tags_missing_auto_becomes_rejection() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), None);
        assert!(is_rejected(&conn, &e, "brass").unwrap());
    }

    #[test]
    fn set_tags_only_rejects_seen_auto_tags() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        // Added by a run after the editor loaded the entry, so not in `seen`.
        add_auto(&conn, &e, "dark", 0.7);
        add_tag_to_entry(&conn, &e, "night").unwrap();

        let seen = vec![" Brass".to_string()];
        set_entry_tags(&conn, &e, &[], Some(&seen)).unwrap();

        assert_eq!(row_of(&conn, &e, "brass"), None);
        assert!(is_rejected(&conn, &e, "brass").unwrap());
        assert_eq!(row_of(&conn, &e, "dark"), Some(("auto".into(), Some(0.7))));
        assert!(!is_rejected(&conn, &e, "dark").unwrap());
        assert_eq!(row_of(&conn, &e, "night"), None, "user tags are still removed");
    }

    #[test]
    fn set_tags_missing_user_is_plain_delete_no_rejection() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "night").unwrap();
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert_eq!(row_of(&conn, &e, "night"), None);
        assert!(!is_rejected(&conn, &e, "night").unwrap());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tag_rejections"), 0);
    }

    #[test]
    fn user_add_clears_rejection() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert!(is_rejected(&conn, &e, "brass").unwrap());

        set_entry_tags(&conn, &e, &["brass".into()], None).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), Some(("user".into(), None)));
        assert!(!is_rejected(&conn, &e, "brass").unwrap());

        // Same rule for bulk add.
        let f = new_entry(&conn, "/b.mp3");
        add_auto(&conn, &f, "drums", 0.7);
        bulk_edit_entry_tags(&conn, &[f.clone()], &[], &["drums".into()]).unwrap();
        assert!(is_rejected(&conn, &f, "drums").unwrap());
        bulk_edit_entry_tags(&conn, &[f.clone()], &["drums".into()], &[]).unwrap();
        assert_eq!(row_of(&conn, &f, "drums"), Some(("user".into(), None)));
        assert!(!is_rejected(&conn, &f, "drums").unwrap());
    }

    #[test]
    fn bulk_remove_of_auto_tag_records_rejection() {
        let conn = open_memory_db();
        let a = new_entry(&conn, "/a.mp3");
        let b = new_entry(&conn, "/b.mp3");
        add_auto(&conn, &a, "brass", 0.8);
        add_tag_to_entry(&conn, &b, "brass").unwrap();
        bulk_edit_entry_tags(&conn, &[a.clone(), b.clone()], &[], &["brass".into()]).unwrap();
        assert!(is_rejected(&conn, &a, "brass").unwrap());
        assert!(!is_rejected(&conn, &b, "brass").unwrap());
        assert_eq!(row_of(&conn, &a, "brass"), None);
        assert_eq!(row_of(&conn, &b, "brass"), None);
    }

    #[test]
    fn bulk_add_existing_auto_stays_auto() {
        let conn = open_memory_db();
        let a = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &a, "brass", 0.8);
        bulk_edit_entry_tags(&conn, &[a.clone()], &["brass".into()], &[]).unwrap();
        assert_eq!(row_of(&conn, &a, "brass"), Some(("auto".into(), Some(0.8))));
    }

    #[test]
    fn keep_flips_to_user_and_clears_score() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        add_auto(&conn, &e, "drums", 0.7);

        keep_auto_tags(&conn, &[e.clone()], Some(&["brass".to_string()])).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), Some(("user".into(), None)));
        assert_eq!(row_of(&conn, &e, "drums"), Some(("auto".into(), Some(0.7))));

        keep_auto_tags(&conn, &[e.clone()], None).unwrap();
        assert_eq!(row_of(&conn, &e, "drums"), Some(("user".into(), None)));
    }

    #[test]
    fn reject_auto_tags_deletes_rows_and_records_rejections() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        add_auto(&conn, &e, "drums", 0.7);
        add_tag_to_entry(&conn, &e, "night").unwrap();

        reject_auto_tags(&conn, &[e.clone()], Some(&["brass".to_string()])).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), None);
        assert!(is_rejected(&conn, &e, "brass").unwrap());
        assert!(row_of(&conn, &e, "drums").is_some());

        reject_auto_tags(&conn, &[e.clone()], None).unwrap();
        assert_eq!(row_of(&conn, &e, "drums"), None);
        assert!(is_rejected(&conn, &e, "drums").unwrap());
        // User rows are never rejected.
        assert_eq!(row_of(&conn, &e, "night"), Some(("user".into(), None)));
        assert!(!is_rejected(&conn, &e, "night").unwrap());
    }

    fn auto(tag: &str, score: f64) -> crate::models::AutoTag {
        crate::models::AutoTag { tag: tag.into(), score }
    }

    #[test]
    fn apply_auto_never_touches_user_rows() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "night").unwrap();
        apply_auto_tags(&conn, &e, &[auto("night", 0.9), auto("brass", 0.8)]).unwrap();
        assert_eq!(row_of(&conn, &e, "night"), Some(("user".into(), None)));
        assert_eq!(row_of(&conn, &e, "brass"), Some(("auto".into(), Some(0.8))));

        // An empty desired set removes auto rows but leaves user rows.
        apply_auto_tags(&conn, &e, &[]).unwrap();
        assert_eq!(row_of(&conn, &e, "night"), Some(("user".into(), None)));
        assert_eq!(row_of(&conn, &e, "brass"), None);
    }

    #[test]
    fn apply_auto_skips_rejected_and_user_named() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "night").unwrap();
        add_auto(&conn, &e, "drums", 0.7);
        reject_auto_tags(&conn, &[e.clone()], None).unwrap();

        let (added, removed) = apply_auto_tags(
            &conn,
            &e,
            &[auto("night", 0.9), auto("drums", 0.9), auto("brass", 0.8)],
        )
        .unwrap();
        assert_eq!((added, removed), (1, 0));
        assert_eq!(row_of(&conn, &e, "drums"), None);
        assert_eq!(row_of(&conn, &e, "night"), Some(("user".into(), None)));
        assert!(row_of(&conn, &e, "brass").is_some());
    }

    #[test]
    fn apply_auto_replaces_previous_auto_set() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "old", 0.6);
        add_auto(&conn, &e, "same", 0.6);

        let (added, removed) =
            apply_auto_tags(&conn, &e, &[auto("same", 0.9), auto("fresh", 0.8)]).unwrap();
        assert_eq!((added, removed), (1, 1));
        assert_eq!(row_of(&conn, &e, "old"), None);
        // A kept name gets its new score.
        assert_eq!(row_of(&conn, &e, "same"), Some(("auto".into(), Some(0.9))));
        assert_eq!(row_of(&conn, &e, "fresh"), Some(("auto".into(), Some(0.8))));
        // The dropped tag was not a user rejection and, once the caller
        // sweeps, no longer has any row.
        assert!(!is_rejected(&conn, &e, "old").unwrap());
        sweep_orphan_tags(&conn).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'old'"), 0);
    }

    #[test]
    fn removing_a_tag_leaves_the_other_auto_tags() {
        let conn = open_memory_db();

        // Rejecting one auto tag rejects only that tag.
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "encounter", 0.9);
        add_auto(&conn, &e, "brass", 0.8);
        reject_auto_tags(&conn, &[e.clone()], Some(&["Encounter".to_string()])).unwrap();
        assert_eq!(row_of(&conn, &e, "encounter"), None);
        assert!(is_rejected(&conn, &e, "encounter").unwrap());
        assert_eq!(row_of(&conn, &e, "brass"), Some(("auto".into(), Some(0.8))));

        // A full-list save that drops a user tag keeps the listed auto tags.
        let a = new_entry(&conn, "/b.mp3");
        add_tag_to_entry(&conn, &a, "encounter").unwrap();
        add_auto(&conn, &a, "brass", 0.8);
        set_entry_tags(&conn, &a, &["brass".into()], None).unwrap();
        assert_eq!(row_of(&conn, &a, "encounter"), None);
        assert_eq!(row_of(&conn, &a, "brass"), Some(("auto".into(), Some(0.8))));

        // A bulk remove of a user tag keeps the auto tags.
        let b = new_entry(&conn, "/c.mp3");
        add_tag_to_entry(&conn, &b, "encounter").unwrap();
        add_auto(&conn, &b, "brass", 0.8);
        bulk_edit_entry_tags(&conn, &[b.clone()], &[], &["encounter".into()]).unwrap();
        assert_eq!(row_of(&conn, &b, "encounter"), None);
        assert_eq!(row_of(&conn, &b, "brass"), Some(("auto".into(), Some(0.8))));

        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tag_rejections"), 1);
    }

    #[test]
    fn rejection_survives_orphan_sweep() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "x", 0.8);
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'x'"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tag_rejections"), 1);
        assert!(list_all_tags(&conn).unwrap().is_empty());

        // Other sweeps leave it alone too.
        let f = new_entry(&conn, "/b.mp3");
        add_tag_to_entry(&conn, &f, "y").unwrap();
        bulk_edit_entry_tags(&conn, &[f.clone()], &[], &["y".into()]).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tag_rejections"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'x'"), 1);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'y'"), 0);
    }

    #[test]
    fn delete_entry_cascades_rejections() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "x", 0.8);
        reject_auto_tags(&conn, &[e.clone()], None).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tag_rejections"), 1);
        conn.execute("DELETE FROM library_entries WHERE id = ?1", params![e]).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tag_rejections"), 0);
    }
    // ── Role facet ────────────────────────────────────────────────────────────

    fn role_row(conn: &Connection, id: &str) -> (Option<String>, Option<String>, Option<f64>) {
        conn.query_row(
            "SELECT role, role_source, role_score FROM library_entries WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn music_entry(conn: &Connection, path: &str) -> String {
        insert_library_entry(conn, path, None, "T", None, None, None, "music").unwrap()
    }

    #[test]
    fn user_role_set_and_clear() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let b = music_entry(&conn, "/b.mp3");

        set_user_role(&conn, &[a.clone(), b.clone()], Some(Role::NonCombat)).unwrap();
        assert_eq!(role_row(&conn, &a), (Some("non_combat".into()), Some("user".into()), None));
        assert_eq!(role_row(&conn, &b), (Some("non_combat".into()), Some("user".into()), None));

        // Clearing keeps the user source so the analyzer stays out.
        set_user_role(&conn, &[a.clone()], None).unwrap();
        assert_eq!(role_row(&conn, &a), (None, Some("user".into()), None));
        assert_eq!(role_row(&conn, &b).0, Some("non_combat".into()));
    }

    #[test]
    fn user_clear_blocks_auto_role() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        set_user_role(&conn, &[a.clone()], None).unwrap();

        let changed = apply_auto_role(&conn, &a, Some(Role::Combat), 0.95).unwrap();

        assert!(!changed);
        assert_eq!(role_row(&conn, &a), (None, Some("user".into()), None));
    }

    #[test]
    fn apply_auto_role_never_overwrites_user_role() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        set_user_role(&conn, &[a.clone()], Some(Role::NonCombat)).unwrap();

        assert!(!apply_auto_role(&conn, &a, Some(Role::Combat), 0.99).unwrap());
        assert!(!apply_auto_role(&conn, &a, None, 0.1).unwrap());
        assert_eq!(role_row(&conn, &a), (Some("non_combat".into()), Some("user".into()), None));
    }

    #[test]
    fn apply_auto_role_none_clears_only_auto() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");

        assert!(apply_auto_role(&conn, &a, Some(Role::Combat), 0.8).unwrap());
        assert_eq!(role_row(&conn, &a), (Some("combat".into()), Some("auto".into()), Some(0.8)));

        // Same value again changes nothing; a new score does.
        assert!(!apply_auto_role(&conn, &a, Some(Role::Combat), 0.8).unwrap());
        assert!(apply_auto_role(&conn, &a, Some(Role::Combat), 0.7).unwrap());

        assert!(apply_auto_role(&conn, &a, None, 0.5).unwrap());
        assert_eq!(role_row(&conn, &a), (None, None, None));
        // Nothing left to clear.
        assert!(!apply_auto_role(&conn, &a, None, 0.5).unwrap());
    }

    #[test]
    fn keep_auto_role_flips_source() {
        let conn = open_memory_db();
        let auto = music_entry(&conn, "/a.mp3");
        let user = music_entry(&conn, "/b.mp3");
        apply_auto_role(&conn, &auto, Some(Role::Combat), 0.8).unwrap();
        set_user_role(&conn, &[user.clone()], Some(Role::NonCombat)).unwrap();

        keep_auto_role(&conn, &[auto.clone(), user.clone()]).unwrap();

        assert_eq!(role_row(&conn, &auto), (Some("combat".into()), Some("user".into()), None));
        assert_eq!(role_row(&conn, &user), (Some("non_combat".into()), Some("user".into()), None));
        // Once kept, the analyzer can no longer change it.
        assert!(!apply_auto_role(&conn, &auto, Some(Role::NonCombat), 0.9).unwrap());
    }

    #[test]
    fn leaving_music_clears_role() {
        let conn = open_memory_db();
        let auto = music_entry(&conn, "/a.mp3");
        let user = music_entry(&conn, "/b.mp3");
        let cleared = music_entry(&conn, "/c.mp3");
        apply_auto_role(&conn, &auto, Some(Role::Combat), 0.8).unwrap();
        set_user_role(&conn, &[user.clone()], Some(Role::NonCombat)).unwrap();
        set_user_role(&conn, &[cleared.clone()], None).unwrap();

        for id in [&auto, &user, &cleared] {
            update_library_entry_type(&conn, id, "ambiance").unwrap();
        }

        assert_eq!(role_row(&conn, &auto), (None, None, None));
        assert_eq!(role_row(&conn, &user), (None, None, None));
        assert_eq!(role_row(&conn, &cleared), (None, None, None));
    }
}
