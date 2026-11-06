use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::database::now_millis;
use crate::database::DbError;
use crate::models::{AutoTag, DarkKind, LibraryEntryUsage, MoodScale, MoodStep, MoodZ, Role};

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
/// music the analyzer layer of its tags is dropped in the same transaction
/// (rows the user has not voted on go), orphaned tag rows are swept, and its
/// role and mood are cleared entirely, both layers (changing the type
/// is itself a user action). The user's tag votes are left alone.
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
            "UPDATE library_entries
             SET role_auto = NULL, role_auto_score = NULL, role_user = NULL
             WHERE id = ?1",
            params![id],
        )
        .map_err(|e| format!("Failed to clear role: {e}"))?;
        conn.execute(
            "UPDATE library_entries
             SET mood_bright = NULL, mood_bright_user = NULL,
                 mood_grand = NULL, mood_grand_user = NULL,
                 mood_energy = NULL, mood_energy_user = NULL,
                 mood_sad = NULL, mood_scary = NULL, dark_kind_user = NULL
             WHERE id = ?1",
            params![id],
        )
        .map_err(|e| format!("Failed to clear mood: {e}"))?;
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

// ─── Tag layers ───────────────────────────────────────────────────────────────
//
// Each `library_entry_tags` row holds two independent layers for one tag on
// one entry: the analyzer's `auto_score` (written only by analysis runs) and
// the user's `user_vote` (1 yes, 0 no, NULL not voted; written only by the
// user). A row with neither is deleted. The entry shows the tag when the user
// voted yes, or did not vote and the analyzer tags it.

/// SQL condition (on a `library_entry_tags` row aliased `lt`) for a tag the
/// entry shows.
const SHOWN: &str = "(lt.user_vote = 1 OR (lt.user_vote IS NULL AND lt.auto_score IS NOT NULL))";

/// Sets the user's vote on one tag of one entry, creating the row if needed.
fn set_vote(conn: &Connection, entry_id: &str, tag_id: &str, vote: bool) -> Result<(), String> {
    conn.execute(
        "INSERT INTO library_entry_tags (library_entry_id, tag_id, user_vote) VALUES (?1, ?2, ?3)
         ON CONFLICT (library_entry_id, tag_id) DO UPDATE SET user_vote = excluded.user_vote",
        params![entry_id, tag_id, vote as i64],
    )
    .map_err(|e| format!("Failed to record tag vote: {e}"))?;
    Ok(())
}

/// Clears the user's vote on one tag of one entry. A row the analyzer does
/// not tag either is deleted (first, since the table never holds a row with
/// neither layer).
fn clear_vote(conn: &Connection, entry_id: &str, tag_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM library_entry_tags
         WHERE library_entry_id = ?1 AND tag_id = ?2 AND auto_score IS NULL",
        params![entry_id, tag_id],
    )
    .map_err(|e| format!("Failed to delete tag vote: {e}"))?;
    conn.execute(
        "UPDATE library_entry_tags SET user_vote = NULL WHERE library_entry_id = ?1 AND tag_id = ?2",
        params![entry_id, tag_id],
    )
    .map_err(|e| format!("Failed to clear tag vote: {e}"))?;
    Ok(())
}

/// The id of the tag with this (normalised) name, if it exists.
fn tag_id_of(conn: &Connection, tag_name: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM tags WHERE name = ?1",
        params![tag_name.trim().to_lowercase()],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| format!("Failed to query tag: {e}"))
}

/// One stored tag row of an entry.
struct StoredTag {
    id: String,
    name: String,
    /// The entry shows the tag.
    shown: bool,
    /// The entry shows the tag only because of the analyzer (no vote).
    auto_only: bool,
}

fn stored_tags(conn: &Connection, entry_id: &str) -> Result<Vec<StoredTag>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT lt.tag_id, t.name, {SHOWN}, lt.user_vote IS NULL AND lt.auto_score IS NOT NULL
             FROM library_entry_tags lt
             JOIN tags t ON t.id = lt.tag_id
             WHERE lt.library_entry_id = ?1"
        ))
        .map_err(|e| format!("Failed to prepare stored tag query: {e}"))?;
    let rows = stmt
        .query_map(params![entry_id], |row| {
            Ok(StoredTag {
                id: row.get(0)?,
                name: row.get(1)?,
                shown: row.get(2)?,
                auto_only: row.get(3)?,
            })
        })
        .map_err(|e| format!("Failed to query stored tags: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect stored tags: {e}"))?;
    Ok(rows)
}

/// Votes yes on a user-added tag. A tag the entry already shows is left as
/// it is (an analyzer tag stays the analyzer's until the user confirms it).
fn add_user_tag(conn: &Connection, entry_id: &str, tag_id: &str, stored: &[StoredTag]) -> Result<(), String> {
    if stored.iter().any(|t| t.id == tag_id && t.shown) {
        return Ok(());
    }
    set_vote(conn, entry_id, tag_id, true)
}

/// Associates a tag (creating it if necessary) with the given library entry
/// as a yes vote. Used for tags chosen at import time.
pub fn add_tag_to_entry(
    conn: &Connection,
    entry_id: &str,
    tag_name: &str,
) -> Result<(), String> {
    let tag_id = ensure_tag(conn, tag_name)?;
    set_vote(conn, entry_id, &tag_id, true)
}

/// Deletes tag rows that no entry row references. A no vote is a row, so a
/// tag the user voted no on stays.
pub(crate) const SWEEP_ORPHAN_TAGS: &str =
    "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM library_entry_tags)";

pub fn sweep_orphan_tags(conn: &Connection) -> Result<(), String> {
    conn.execute(SWEEP_ORPHAN_TAGS, [])
        .map_err(|e| format!("Failed to sweep orphaned tags: {e}"))?;
    Ok(())
}

/// Drops the analyzer layer from every tag on the entry: rows the user has
/// not voted on are deleted, the rest lose their score. The votes stay.
fn remove_auto_tags(conn: &Connection, entry_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM library_entry_tags WHERE library_entry_id = ?1 AND user_vote IS NULL",
        params![entry_id],
    )
    .map_err(|e| format!("Failed to remove auto tags: {e}"))?;
    conn.execute(
        "UPDATE library_entry_tags SET auto_score = NULL WHERE library_entry_id = ?1",
        params![entry_id],
    )
    .map_err(|e| format!("Failed to clear auto tag scores: {e}"))?;
    Ok(())
}

/// Atomically saves the full tag list for an entry as a diff against the
/// tags it shows, in one transaction:
/// - a listed tag the entry already shows is left as it is;
/// - a listed tag the entry does not show gets a yes vote;
/// - a shown tag missing from the list gets a no vote, whether it was the
///   user's or the analyzer's.
///
/// `seen_auto_tags` names the analyzer tags the editor was showing when the
/// list was built. When it is `Some`, a missing analyzer tag only gets a no
/// vote if it is in that list; one a background run added after the editor
/// loaded is left alone. With `None`, every missing shown tag gets a no vote.
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

    let stored = stored_tags(conn, entry_id)?;
    for tag in stored.iter().filter(|t| t.shown && !wanted_ids.contains(&t.id)) {
        let unseen_auto = tag.auto_only && seen.as_ref().is_some_and(|seen| !seen.contains(&tag.name));
        if !unseen_auto {
            set_vote(conn, entry_id, &tag.id, false)?;
        }
    }
    for tag_id in &wanted_ids {
        add_user_tag(conn, entry_id, tag_id, &stored)?;
    }

    sweep_orphan_tags(conn)?;

    tx.commit().map_err(|e| format!("Failed to commit tag transaction: {e}"))?;
    Ok(())
}

/// Adds `add` to and removes `remove` from every entry in `entry_ids`, in
/// one transaction, with the same rules as `set_entry_tags`: removing a tag
/// the entry shows votes no (removing one it doesn't show does nothing), and
/// adding a tag it doesn't show votes yes. Orphaned tag rows are swept at the
/// end.
pub fn bulk_edit_entry_tags(
    conn: &Connection,
    entry_ids: &[String],
    add: &[String],
    remove: &[String],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin bulk tag transaction: {e}"))?;

    let mut add_ids = Vec::new();
    for tag_name in add {
        add_ids.push(ensure_tag(conn, tag_name)?);
    }
    let mut remove_ids = Vec::new();
    for tag_name in remove {
        if let Some(id) = tag_id_of(conn, tag_name)? {
            remove_ids.push(id);
        }
    }

    for entry_id in entry_ids {
        for tag in stored_tags(conn, entry_id)?.iter().filter(|t| t.shown && remove_ids.contains(&t.id)) {
            set_vote(conn, entry_id, &tag.id, false)?;
        }
        let stored = stored_tags(conn, entry_id)?;
        for tag_id in &add_ids {
            add_user_tag(conn, entry_id, tag_id, &stored)?;
        }
    }

    sweep_orphan_tags(conn)?;

    tx.commit()
        .map_err(|e| format!("Failed to commit bulk tag transaction: {e}"))?;
    Ok(())
}

/// Returns the ids of the tags the entry shows only because of the analyzer.
/// With `tags` set, only those names are considered; with `None`, all of them.
fn auto_tag_ids(
    conn: &Connection,
    entry_id: &str,
    tags: Option<&[String]>,
) -> Result<Vec<String>, String> {
    let wanted: Option<Vec<String>> =
        tags.map(|names| names.iter().map(|n| n.trim().to_lowercase()).collect());
    Ok(stored_tags(conn, entry_id)?
        .into_iter()
        .filter(|t| t.auto_only && wanted.as_ref().map_or(true, |w| w.contains(&t.name)))
        .map(|t| t.id)
        .collect())
}

/// Votes yes on the entries' matching analyzer tags, which makes them the
/// user's (and training examples). `tags: None` confirms every analyzer tag
/// on the entries. The analyzer's scores stay.
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
            set_vote(conn, entry_id, &tag_id, true)?;
        }
    }

    tx.commit().map_err(|e| format!("Failed to commit keep transaction: {e}"))?;
    Ok(())
}

/// Votes no on the entries' matching analyzer tags, so they are no longer
/// shown. `tags: None` rejects every analyzer tag on the entries. Tags the
/// user voted on are never touched.
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
            set_vote(conn, entry_id, &tag_id, false)?;
        }
    }

    tx.commit().map_err(|e| format!("Failed to commit reject transaction: {e}"))?;
    Ok(())
}

/// Clears the user's vote (yes or no) on the named tags of every entry, so
/// the analyzer's layer decides again. Rows the analyzer does not tag are
/// deleted, and orphaned tag rows are swept.
pub fn reset_entry_tag_votes(
    conn: &Connection,
    entry_ids: &[String],
    tags: &[String],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin vote reset transaction: {e}"))?;

    let mut tag_ids = Vec::new();
    for tag_name in tags {
        if let Some(id) = tag_id_of(conn, tag_name)? {
            tag_ids.push(id);
        }
    }
    for entry_id in entry_ids {
        for tag_id in &tag_ids {
            clear_vote(conn, entry_id, tag_id)?;
        }
    }
    sweep_orphan_tags(conn)?;

    tx.commit().map_err(|e| format!("Failed to commit vote reset transaction: {e}"))?;
    Ok(())
}

/// True if the user voted no on this tag on this entry.
#[cfg(test)]
pub fn is_rejected(conn: &Connection, entry_id: &str, tag_name: &str) -> Result<bool, String> {
    let normalised = tag_name.trim().to_lowercase();
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM library_entry_tags lt
             JOIN tags t ON t.id = lt.tag_id
             WHERE lt.library_entry_id = ?1 AND t.name = ?2 AND lt.user_vote = 0)",
        params![entry_id, normalised],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to check tag rejection: {e}"))
}

/// Replaces the analyzer layer of the entry's tags with `desired` and
/// returns how many tags the entry now shows (added, removed) compared to
/// before.
///
/// Only `auto_score` is written. A desired tag gets its score (creating the
/// row if needed); a tag no longer desired loses it, and its row goes if the
/// user has not voted on it. The user's votes are never read or changed, so
/// a tag the user voted on is scored like any other.
///
/// The counts are changes to what the entry shows, which here is cheap: a
/// new row (nothing was there) is an addition and a deleted row (the
/// analyzer's tag with no vote) is a removal. Scoring a tag the user voted on
/// changes nothing shown, so it is not counted.
///
/// This does not open its own transaction and must be called inside one. It
/// does not sweep orphaned tag rows; the caller runs `sweep_orphan_tags`
/// once after its last call.
pub fn apply_auto_tags(
    conn: &Connection,
    entry_id: &str,
    desired: &[AutoTag],
) -> Result<(u32, u32), String> {
    let mut keep: Vec<(String, f64)> = Vec::new();
    for wanted in desired {
        let tag_id = ensure_tag(conn, &wanted.tag)?;
        if !keep.iter().any(|(id, _)| *id == tag_id) {
            keep.push((tag_id, wanted.score));
        }
    }

    // (tag id, has a vote) for every row the analyzer currently tags.
    let scored: Vec<(String, bool)> = {
        let mut stmt = conn
            .prepare(
                "SELECT tag_id, user_vote IS NOT NULL FROM library_entry_tags
                 WHERE library_entry_id = ?1 AND auto_score IS NOT NULL",
            )
            .map_err(|e| format!("Failed to prepare auto tag query: {e}"))?;
        let rows = stmt
            .query_map(params![entry_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("Failed to query auto tags: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect auto tags: {e}"))?;
        rows
    };

    let mut removed = 0;
    for (tag_id, voted) in &scored {
        if keep.iter().any(|(id, _)| id == tag_id) {
            continue;
        }
        if *voted {
            conn.execute(
                "UPDATE library_entry_tags SET auto_score = NULL
                 WHERE library_entry_id = ?1 AND tag_id = ?2",
                params![entry_id, tag_id],
            )
            .map_err(|e| format!("Failed to clear auto tag score: {e}"))?;
        } else {
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
        let updated = conn
            .execute(
                "UPDATE library_entry_tags SET auto_score = ?3
                 WHERE library_entry_id = ?1 AND tag_id = ?2",
                params![entry_id, tag_id, score],
            )
            .map_err(|e| format!("Failed to update auto tag score: {e}"))?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO library_entry_tags (library_entry_id, tag_id, auto_score)
                 VALUES (?1, ?2, ?3)",
                params![entry_id, tag_id, score],
            )
            .map_err(|e| format!("Failed to add auto tag: {e}"))?;
            added += 1;
        }
    }

    Ok((added, removed))
}

// ─── Role layers ──────────────────────────────────────────────────────────────

/// A facet stored on `library_entries` as an analyzer layer (`<name>_auto`,
/// `<name>_auto_score`) and a user layer (`<name>_user`). Column names come
/// from this closed enum, never from caller-supplied text.
#[derive(Clone, Copy)]
enum Facet {
    Role,
}

impl Facet {
    fn name(self) -> &'static str {
        match self {
            Facet::Role => "role",
        }
    }
}

/// Sets the user layer of a facet on each entry. `None` clears it, so the
/// analyzer's value shows.
fn set_user_facet(
    conn: &Connection,
    facet: Facet,
    entry_ids: &[String],
    value: Option<&str>,
) -> Result<(), String> {
    let name = facet.name();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin {name} transaction: {e}"))?;
    let sql = format!("UPDATE library_entries SET {name}_user = ?2 WHERE id = ?1");
    for entry_id in entry_ids {
        conn.execute(&sql, params![entry_id, value])
            .map_err(|e| format!("Failed to set {name}: {e}"))?;
    }
    tx.commit().map_err(|e| format!("Failed to commit {name} transaction: {e}"))?;
    Ok(())
}

/// Copies the analyzer's value of a facet into the user layer on each entry
/// that has one. Entries without an analyzer value are left alone.
fn keep_auto_facet(conn: &Connection, facet: Facet, entry_ids: &[String]) -> Result<(), String> {
    let name = facet.name();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin keep {name} transaction: {e}"))?;
    let sql = format!(
        "UPDATE library_entries SET {name}_user = {name}_auto
         WHERE id = ?1 AND {name}_auto IS NOT NULL"
    );
    for entry_id in entry_ids {
        conn.execute(&sql, params![entry_id])
            .map_err(|e| format!("Failed to keep auto {name}: {e}"))?;
    }
    tx.commit().map_err(|e| format!("Failed to commit keep {name} transaction: {e}"))?;
    Ok(())
}

/// Writes the analyzer layer of a facet and returns whether it changed.
/// `Some` stores the value with `score`; `None` clears value and score. The
/// user layer is never read or written. Opens no transaction of its own.
fn apply_auto_facet(
    conn: &Connection,
    facet: Facet,
    entry_id: &str,
    value: Option<&str>,
    score: f64,
) -> Result<bool, String> {
    let name = facet.name();
    let current: Option<(Option<String>, Option<f64>)> = conn
        .query_row(
            &format!("SELECT {name}_auto, {name}_auto_score FROM library_entries WHERE id = ?1"),
            params![entry_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("Failed to read {name}: {e}"))?;
    let Some((current_value, current_score)) = current else {
        return Ok(false);
    };

    let new_score = value.map(|_| score);
    if current_value.as_deref() == value && current_score == new_score {
        return Ok(false);
    }
    conn.execute(
        &format!("UPDATE library_entries SET {name}_auto = ?2, {name}_auto_score = ?3 WHERE id = ?1"),
        params![entry_id, value, new_score],
    )
    .map_err(|e| format!("Failed to write auto {name}: {e}"))?;
    Ok(true)
}

/// Sets the user's role on each entry. `None` clears it, so the analyzer's
/// role (if any) shows.
pub fn set_user_role(
    conn: &Connection,
    entry_ids: &[String],
    role: Option<Role>,
) -> Result<(), String> {
    set_user_facet(conn, Facet::Role, entry_ids, role.map(|r| r.as_db_str()))
}

/// Makes the analyzer's role the user's, on each entry that has one.
pub fn keep_auto_role(conn: &Connection, entry_ids: &[String]) -> Result<(), String> {
    keep_auto_facet(conn, Facet::Role, entry_ids)
}

/// Writes the analyzer's role for an entry and returns whether it changed.
/// `Some(role)` stores it with `score` (the Combat probability); `None`
/// clears it. The user's role is never read or written.
pub fn apply_auto_role(
    conn: &Connection,
    entry_id: &str,
    role: Option<Role>,
    score: f64,
) -> Result<bool, String> {
    apply_auto_facet(conn, Facet::Role, entry_id, role.map(|r| r.as_db_str()), score)
}

/// Sets the user's step on one mood scale for each entry. `None` clears the
/// user step, so the auto value shows again.
pub fn set_user_mood_step(
    conn: &Connection,
    entry_ids: &[String],
    scale: MoodScale,
    step: Option<MoodStep>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin mood transaction: {e}"))?;

    // The column comes from a closed enum, never from caller-supplied text.
    let sql = match scale {
        MoodScale::Bright => "UPDATE library_entries SET mood_bright_user = ?2 WHERE id = ?1",
        MoodScale::Grand => "UPDATE library_entries SET mood_grand_user = ?2 WHERE id = ?1",
        MoodScale::Energy => "UPDATE library_entries SET mood_energy_user = ?2 WHERE id = ?1",
    };
    let step_str = step.map(|s| s.as_db_str());
    for entry_id in entry_ids {
        conn.execute(sql, params![entry_id, step_str])
            .map_err(|e| format!("Failed to set mood step: {e}"))?;
    }

    tx.commit().map_err(|e| format!("Failed to commit mood transaction: {e}"))?;
    Ok(())
}

/// Sets the user's dark kind for each entry. `None` clears it, so the
/// analyzer's kind shows again. Stored whatever the entry's Bright ↔ Dark
/// step; it is only shown while that step is Dark.
pub fn set_user_dark_kind(
    conn: &Connection,
    entry_ids: &[String],
    kind: Option<DarkKind>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin dark kind transaction: {e}"))?;
    let kind_str = kind.map(|k| k.as_db_str());
    for entry_id in entry_ids {
        conn.execute(
            "UPDATE library_entries SET dark_kind_user = ?2 WHERE id = ?1",
            params![entry_id, kind_str],
        )
        .map_err(|e| format!("Failed to set dark kind: {e}"))?;
    }
    tx.commit().map_err(|e| format!("Failed to commit dark kind transaction: {e}"))?;
    Ok(())
}

/// Writes the analyzer's mood z-scores for an entry, the three scales and
/// the two dark kinds (`None` clears all five), and returns whether the row
/// changed. Only the auto columns are written; the user's steps and kind are
/// never touched. Opens no transaction of its own.
pub fn apply_auto_mood(conn: &Connection, entry_id: &str, mood: Option<MoodZ>) -> Result<bool, String> {
    let current: Option<[Option<f64>; 5]> = conn
        .query_row(
            "SELECT mood_bright, mood_grand, mood_energy, mood_sad, mood_scary
             FROM library_entries WHERE id = ?1",
            params![entry_id],
            |row| Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?]),
        )
        .optional()
        .map_err(|e| format!("Failed to read mood: {e}"))?;

    let Some(current) = current else {
        return Ok(false);
    };
    let new = [
        mood.map(|m| m.bright),
        mood.map(|m| m.grand),
        mood.map(|m| m.energy),
        mood.map(|m| m.sad),
        mood.map(|m| m.scary),
    ];
    if current == new {
        return Ok(false);
    }

    conn.execute(
        "UPDATE library_entries
         SET mood_bright = ?2, mood_grand = ?3, mood_energy = ?4, mood_sad = ?5, mood_scary = ?6
         WHERE id = ?1",
        params![entry_id, new[0], new[1], new[2], new[3], new[4]],
    )
    .map_err(|e| format!("Failed to write auto mood: {e}"))?;
    Ok(true)
}

/// Returns the names of the tags the given library entry shows, sorted.
#[cfg(test)]
pub fn get_tags_for_entry(
    conn: &Connection,
    entry_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            &format!(
                "SELECT t.name
                 FROM tags t
                 JOIN library_entry_tags lt ON lt.tag_id = t.id
                 WHERE lt.library_entry_id = ?1 AND {SHOWN}
                 ORDER BY t.name ASC"
            ),
        )
        .map_err(|e| format!("Failed to prepare get_tags_for_entry: {e}"))?;

    let names = stmt
        .query_map(params![entry_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("Failed to query tags for entry: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect tags for entry: {e}"))?;

    Ok(names)
}

/// Returns all tag names that at least one library entry shows, sorted. A
/// tag that is only voted no on is not listed.
pub fn list_all_tags(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT DISTINCT t.name FROM tags t
             JOIN library_entry_tags lt ON lt.tag_id = t.id
             WHERE {SHOWN}
             ORDER BY t.name ASC"
        ))
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

        bulk_edit_entry_tags(&conn, std::slice::from_ref(&entry_id), &[], &["battle".into()]).expect("remove battle");

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
        // A tag row nothing references, as a crash between writes could leave.
        ensure_tag(&conn, "stray").unwrap();

        set_entry_tags(&conn, &e, &["kept".to_string(), "new".to_string()], None).unwrap();

        assert_eq!(get_tags_for_entry(&conn, &e).unwrap(), vec!["kept", "new"]);
        // "old-only" now holds the user's no vote, so its tag row stays but
        // is no longer offered as a library tag.
        assert!(is_rejected(&conn, &e, "old-only").unwrap());
        assert_eq!(list_all_tags(&conn).unwrap(), vec!["kept", "new"]);
        // A tag nothing references is swept.
        let orphan: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE name = 'stray'", [], |r| r.get(0))
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

        // Removing a tag everywhere leaves no votes for it in the library tag list.
        bulk_edit_entry_tags(&conn, std::slice::from_ref(&c), &[], &["night".into()]).expect("bulk remove");
        assert_eq!(list_all_tags(&conn).unwrap(), vec!["boss", "combat"]);
    }

    // ── Tag layers ────────────────────────────────────────────────────────────

    fn new_entry(conn: &Connection, path: &str) -> String {
        insert_library_entry(conn, path, None, "T", None, None, None, "music").unwrap()
    }

    /// Gives `tag` on `entry` the analyzer score `score`, as a run would.
    fn add_auto(conn: &Connection, entry: &str, tag: &str, score: f64) {
        let tag_id = ensure_tag(conn, tag).unwrap();
        conn.execute(
            "INSERT INTO library_entry_tags (library_entry_id, tag_id, auto_score) VALUES (?1, ?2, ?3)
             ON CONFLICT (library_entry_id, tag_id) DO UPDATE SET auto_score = excluded.auto_score",
            params![entry, tag_id, score],
        )
        .unwrap();
    }

    /// (auto_score, user_vote) of a stored tag row, or None if absent.
    type TagRow = (Option<f64>, Option<i64>);

    fn row_of(conn: &Connection, entry: &str, tag: &str) -> Option<TagRow> {
        conn.query_row(
            "SELECT lt.auto_score, lt.user_vote FROM library_entry_tags lt
             JOIN tags t ON t.id = lt.tag_id
             WHERE lt.library_entry_id = ?1 AND t.name = ?2",
            params![entry, tag],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap()
    }

    /// The analyzer's tag with this score and no vote.
    fn auto_row(score: f64) -> Option<TagRow> {
        Some((Some(score), None))
    }

    /// A yes vote with no analyzer score.
    fn yes() -> Option<TagRow> {
        Some((None, Some(1)))
    }

    /// A no vote with no analyzer score.
    fn no() -> Option<TagRow> {
        Some((None, Some(0)))
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn type_change_from_music_drops_the_analyzer_layer() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        add_auto(&conn, &e, "dark", 0.7);
        add_auto(&conn, &e, "folk", 0.6);
        add_tag_to_entry(&conn, &e, "night").unwrap();
        reject_auto_tags(&conn, std::slice::from_ref(&e), Some(&["dark".to_string()])).unwrap();
        keep_auto_tags(&conn, std::slice::from_ref(&e), Some(&["folk".to_string()])).unwrap();

        assert!(!update_library_entry_type(&conn, &e, "ambiance").unwrap());

        // The unvoted analyzer tag goes; the votes stay without scores.
        assert_eq!(row_of(&conn, &e, "brass"), None);
        assert_eq!(row_of(&conn, &e, "dark"), no());
        assert_eq!(row_of(&conn, &e, "folk"), yes());
        assert_eq!(row_of(&conn, &e, "night"), yes());
        // "brass" is used by nothing and is swept; the no vote keeps "dark".
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 3);
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

        assert_eq!(row_of(&conn, &e, "brass"), auto_row(0.8));
    }

    #[test]
    fn set_tags_keeps_shown_auto_tags_as_they_are() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        set_entry_tags(&conn, &e, &["brass".into(), "night".into()], None).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), auto_row(0.8));
        assert_eq!(row_of(&conn, &e, "night"), yes());
    }

    #[test]
    fn set_tags_missing_auto_votes_no_and_keeps_the_score() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), Some((Some(0.8), Some(0))));
        assert!(is_rejected(&conn, &e, "brass").unwrap());
        assert!(get_tags_for_entry(&conn, &e).unwrap().is_empty());
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

        assert!(is_rejected(&conn, &e, "brass").unwrap());
        assert_eq!(row_of(&conn, &e, "dark"), auto_row(0.7));
        assert!(!is_rejected(&conn, &e, "dark").unwrap());
        assert_eq!(row_of(&conn, &e, "night"), no(), "user tags are voted no too");
    }

    #[test]
    fn set_tags_missing_user_tag_votes_no() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "night").unwrap();
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert_eq!(row_of(&conn, &e, "night"), no());
        assert!(is_rejected(&conn, &e, "night").unwrap());

        // A later analyzer score does not bring it back.
        add_auto(&conn, &e, "night", 0.9);
        assert!(get_tags_for_entry(&conn, &e).unwrap().is_empty());
    }

    #[test]
    fn user_add_overrides_a_no_vote() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert!(is_rejected(&conn, &e, "brass").unwrap());

        set_entry_tags(&conn, &e, &["brass".into()], None).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), Some((Some(0.8), Some(1))));
        assert!(!is_rejected(&conn, &e, "brass").unwrap());

        // Same rule for bulk add.
        let f = new_entry(&conn, "/b.mp3");
        add_auto(&conn, &f, "drums", 0.7);
        bulk_edit_entry_tags(&conn, std::slice::from_ref(&f), &[], &["drums".into()]).unwrap();
        assert!(is_rejected(&conn, &f, "drums").unwrap());
        bulk_edit_entry_tags(&conn, std::slice::from_ref(&f), &["drums".into()], &[]).unwrap();
        assert_eq!(row_of(&conn, &f, "drums"), Some((Some(0.7), Some(1))));
    }

    #[test]
    fn bulk_remove_votes_no_on_shown_tags_only() {
        let conn = open_memory_db();
        let a = new_entry(&conn, "/a.mp3");
        let b = new_entry(&conn, "/b.mp3");
        let c = new_entry(&conn, "/c.mp3");
        add_auto(&conn, &a, "brass", 0.8);
        add_tag_to_entry(&conn, &b, "brass").unwrap();
        add_tag_to_entry(&conn, &c, "night").unwrap();
        bulk_edit_entry_tags(&conn, &[a.clone(), b.clone(), c.clone()], &[], &["brass".into()]).unwrap();
        assert_eq!(row_of(&conn, &a, "brass"), Some((Some(0.8), Some(0))));
        assert_eq!(row_of(&conn, &b, "brass"), no());
        // c never showed the tag, so it gets no vote.
        assert_eq!(row_of(&conn, &c, "brass"), None);
    }

    #[test]
    fn bulk_add_existing_auto_stays_auto() {
        let conn = open_memory_db();
        let a = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &a, "brass", 0.8);
        bulk_edit_entry_tags(&conn, std::slice::from_ref(&a), &["brass".into()], &[]).unwrap();
        assert_eq!(row_of(&conn, &a, "brass"), auto_row(0.8));
    }

    #[test]
    fn keep_votes_yes_and_keeps_the_score() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        add_auto(&conn, &e, "drums", 0.7);

        keep_auto_tags(&conn, std::slice::from_ref(&e), Some(&["brass".to_string()])).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), Some((Some(0.8), Some(1))));
        assert_eq!(row_of(&conn, &e, "drums"), auto_row(0.7));

        keep_auto_tags(&conn, std::slice::from_ref(&e), None).unwrap();
        assert_eq!(row_of(&conn, &e, "drums"), Some((Some(0.7), Some(1))));
    }

    #[test]
    fn reject_auto_tags_votes_no_on_analyzer_tags_only() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        add_auto(&conn, &e, "drums", 0.7);
        add_tag_to_entry(&conn, &e, "night").unwrap();

        reject_auto_tags(&conn, std::slice::from_ref(&e), Some(&["brass".to_string()])).unwrap();
        assert_eq!(row_of(&conn, &e, "brass"), Some((Some(0.8), Some(0))));
        assert_eq!(row_of(&conn, &e, "drums"), auto_row(0.7));

        reject_auto_tags(&conn, std::slice::from_ref(&e), None).unwrap();
        assert!(is_rejected(&conn, &e, "drums").unwrap());
        // The user's own tags are not analyzer tags, so they are untouched.
        assert_eq!(row_of(&conn, &e, "night"), yes());
        assert_eq!(get_tags_for_entry(&conn, &e).unwrap(), vec!["night"]);
    }

    #[test]
    fn reset_votes_returns_tags_to_the_analyzer() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        let f = new_entry(&conn, "/b.mp3");
        add_auto(&conn, &e, "brass", 0.8);
        add_tag_to_entry(&conn, &e, "night").unwrap();
        add_tag_to_entry(&conn, &e, "folk").unwrap();
        reject_auto_tags(&conn, std::slice::from_ref(&e), None).unwrap();
        set_entry_tags(&conn, &e, &["folk".into()], None).unwrap();
        add_tag_to_entry(&conn, &f, "night").unwrap();
        assert_eq!(row_of(&conn, &e, "night"), no());

        reset_entry_tag_votes(&conn, std::slice::from_ref(&e), &["Brass".into(), "night".into(), "missing".into()])
            .unwrap();

        // The analyzer's tag shows again; the vote-only row is gone.
        assert_eq!(row_of(&conn, &e, "brass"), auto_row(0.8));
        assert_eq!(row_of(&conn, &e, "night"), None);
        // Other tags and other entries are untouched.
        assert_eq!(row_of(&conn, &e, "folk"), yes());
        assert_eq!(row_of(&conn, &f, "night"), yes());

        // A tag nothing references any more is swept.
        reset_entry_tag_votes(&conn, std::slice::from_ref(&e), &["folk".into()]).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'folk'"), 0);
    }

    fn auto(tag: &str, score: f64) -> crate::models::AutoTag {
        crate::models::AutoTag { tag: tag.into(), score }
    }

    #[test]
    fn apply_auto_scores_voted_tags_without_touching_votes() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "night").unwrap();
        add_auto(&conn, &e, "drums", 0.7);
        reject_auto_tags(&conn, std::slice::from_ref(&e), None).unwrap();

        let (added, removed) = apply_auto_tags(
            &conn,
            &e,
            &[auto("night", 0.9), auto("drums", 0.95), auto("brass", 0.8)],
        )
        .unwrap();
        // Only brass changes what the entry shows.
        assert_eq!((added, removed), (1, 0));
        assert_eq!(row_of(&conn, &e, "night"), Some((Some(0.9), Some(1))));
        assert_eq!(row_of(&conn, &e, "drums"), Some((Some(0.95), Some(0))));
        assert_eq!(row_of(&conn, &e, "brass"), auto_row(0.8));

        // An empty desired set drops every score but keeps the votes.
        let (added, removed) = apply_auto_tags(&conn, &e, &[]).unwrap();
        assert_eq!((added, removed), (0, 1));
        assert_eq!(row_of(&conn, &e, "night"), yes());
        assert_eq!(row_of(&conn, &e, "drums"), no());
        assert_eq!(row_of(&conn, &e, "brass"), None);
    }

    #[test]
    fn apply_auto_replaces_previous_auto_set() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "old", 0.6);
        add_auto(&conn, &e, "same", 0.6);

        let (added, removed) =
            apply_auto_tags(&conn, &e, &[auto("same", 0.9), auto("fresh", 0.8), auto("fresh", 0.1)])
                .unwrap();
        assert_eq!((added, removed), (1, 1));
        assert_eq!(row_of(&conn, &e, "old"), None);
        // A kept name gets its new score; a repeated name counts once.
        assert_eq!(row_of(&conn, &e, "same"), auto_row(0.9));
        assert_eq!(row_of(&conn, &e, "fresh"), auto_row(0.8));
        assert!(!is_rejected(&conn, &e, "old").unwrap());
        sweep_orphan_tags(&conn).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'old'"), 0);
    }

    #[test]
    fn removing_a_tag_leaves_the_other_auto_tags() {
        let conn = open_memory_db();

        // Rejecting one analyzer tag rejects only that tag.
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "encounter", 0.9);
        add_auto(&conn, &e, "brass", 0.8);
        reject_auto_tags(&conn, std::slice::from_ref(&e), Some(&["Encounter".to_string()])).unwrap();
        assert!(is_rejected(&conn, &e, "encounter").unwrap());
        assert_eq!(row_of(&conn, &e, "brass"), auto_row(0.8));

        // A full-list save that drops a user tag keeps the listed auto tags.
        let a = new_entry(&conn, "/b.mp3");
        add_tag_to_entry(&conn, &a, "encounter").unwrap();
        add_auto(&conn, &a, "brass", 0.8);
        set_entry_tags(&conn, &a, &["brass".into()], None).unwrap();
        assert_eq!(row_of(&conn, &a, "encounter"), no());
        assert_eq!(row_of(&conn, &a, "brass"), auto_row(0.8));

        // A bulk remove of a user tag keeps the auto tags.
        let b = new_entry(&conn, "/c.mp3");
        add_tag_to_entry(&conn, &b, "encounter").unwrap();
        add_auto(&conn, &b, "brass", 0.8);
        bulk_edit_entry_tags(&conn, std::slice::from_ref(&b), &[], &["encounter".into()]).unwrap();
        assert_eq!(row_of(&conn, &b, "encounter"), no());
        assert_eq!(row_of(&conn, &b, "brass"), auto_row(0.8));
    }

    #[test]
    fn no_vote_survives_orphan_sweep() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "x").unwrap();
        set_entry_tags(&conn, &e, &[], None).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'x'"), 1);
        // A tag only voted no on is not offered as a library tag.
        assert!(list_all_tags(&conn).unwrap().is_empty());
    }

    #[test]
    fn tag_rows_need_a_layer_and_a_valid_vote() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        let t = ensure_tag(&conn, "x").unwrap();
        let insert = |score: Option<f64>, vote: Option<i64>| {
            conn.execute(
                "INSERT OR REPLACE INTO library_entry_tags (library_entry_id, tag_id, auto_score, user_vote)
                 VALUES (?1, ?2, ?3, ?4)",
                params![e, t, score, vote],
            )
        };
        assert!(insert(None, None).is_err());
        assert!(insert(None, Some(2)).is_err());
        assert!(insert(Some(0.5), None).is_ok());
        assert!(insert(None, Some(0)).is_ok());
    }

    #[test]
    fn delete_entry_cascades_tag_rows() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_auto(&conn, &e, "x", 0.8);
        reject_auto_tags(&conn, std::slice::from_ref(&e), None).unwrap();
        conn.execute("DELETE FROM library_entries WHERE id = ?1", params![e]).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM library_entry_tags"), 0);
    }

    // ── Role facet ────────────────────────────────────────────────────────────

    /// (auto, auto score, user) of a facet as stored.
    type FacetRow = (Option<String>, Option<f64>, Option<String>);

    fn role_row(conn: &Connection, id: &str) -> FacetRow {
        conn.query_row(
            "SELECT role_auto, role_auto_score, role_user FROM library_entries WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn music_entry(conn: &Connection, path: &str) -> String {
        insert_library_entry(conn, path, None, "T", None, None, None, "music").unwrap()
    }

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn user_role_set_and_clear() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let b = music_entry(&conn, "/b.mp3");
        apply_auto_role(&conn, &a, Some(Role::Combat), 0.8).unwrap();

        set_user_role(&conn, &[a.clone(), b.clone()], Some(Role::NonCombat)).unwrap();
        assert_eq!(role_row(&conn, &a), (s("combat"), Some(0.8), s("non_combat")));
        assert_eq!(role_row(&conn, &b), (None, None, s("non_combat")));

        // Clearing is "use auto": only the user layer goes.
        set_user_role(&conn, std::slice::from_ref(&a), None).unwrap();
        assert_eq!(role_row(&conn, &a), (s("combat"), Some(0.8), None));
        assert_eq!(role_row(&conn, &b).2, s("non_combat"));
    }

    #[test]
    fn apply_auto_role_writes_the_auto_layer_even_when_user_set() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        set_user_role(&conn, std::slice::from_ref(&a), Some(Role::NonCombat)).unwrap();

        assert!(apply_auto_role(&conn, &a, Some(Role::Combat), 0.99).unwrap());
        assert_eq!(role_row(&conn, &a), (s("combat"), Some(0.99), s("non_combat")));
        assert!(apply_auto_role(&conn, &a, None, 0.1).unwrap());
        assert_eq!(role_row(&conn, &a), (None, None, s("non_combat")));
    }

    #[test]
    fn apply_auto_role_reports_change() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");

        assert!(apply_auto_role(&conn, &a, Some(Role::Combat), 0.8).unwrap());
        assert_eq!(role_row(&conn, &a), (s("combat"), Some(0.8), None));

        // Same value again changes nothing; a new score does.
        assert!(!apply_auto_role(&conn, &a, Some(Role::Combat), 0.8).unwrap());
        assert!(apply_auto_role(&conn, &a, Some(Role::Combat), 0.7).unwrap());

        assert!(apply_auto_role(&conn, &a, None, 0.5).unwrap());
        assert_eq!(role_row(&conn, &a), (None, None, None));
        // Nothing left to clear.
        assert!(!apply_auto_role(&conn, &a, None, 0.5).unwrap());
        // A missing entry is no change.
        assert!(!apply_auto_role(&conn, "nope", Some(Role::Combat), 0.5).unwrap());
    }

    #[test]
    fn keep_auto_role_copies_auto_into_user() {
        let conn = open_memory_db();
        let auto = music_entry(&conn, "/a.mp3");
        let user = music_entry(&conn, "/b.mp3");
        let unset = music_entry(&conn, "/c.mp3");
        apply_auto_role(&conn, &auto, Some(Role::Combat), 0.8).unwrap();
        apply_auto_role(&conn, &user, Some(Role::Combat), 0.8).unwrap();
        set_user_role(&conn, std::slice::from_ref(&user), Some(Role::NonCombat)).unwrap();

        keep_auto_role(&conn, &[auto.clone(), user.clone(), unset.clone()]).unwrap();

        assert_eq!(role_row(&conn, &auto), (s("combat"), Some(0.8), s("combat")));
        // Keep confirms the analyzer's value, overriding an older choice.
        assert_eq!(role_row(&conn, &user), (s("combat"), Some(0.8), s("combat")));
        assert_eq!(role_row(&conn, &unset), (None, None, None));
    }

    #[test]
    fn leaving_music_clears_both_role_layers() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        apply_auto_role(&conn, &a, Some(Role::Combat), 0.8).unwrap();
        set_user_role(&conn, std::slice::from_ref(&a), Some(Role::NonCombat)).unwrap();

        update_library_entry_type(&conn, &a, "ambiance").unwrap();

        assert_eq!(role_row(&conn, &a), (None, None, None));
    }

    // ── Mood scales ───────────────────────────────────────────────────────────

    /// (bright z, grand z, energy z) as stored.
    type MoodAuto = (Option<f64>, Option<f64>, Option<f64>);
    /// (bright step, grand step, energy step) as stored.
    type MoodUser = (Option<String>, Option<String>, Option<String>);

    fn mood_auto(conn: &Connection, id: &str) -> MoodAuto {
        conn.query_row(
            "SELECT mood_bright, mood_grand, mood_energy FROM library_entries WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    fn mood_user(conn: &Connection, id: &str) -> MoodUser {
        conn.query_row(
            "SELECT mood_bright_user, mood_grand_user, mood_energy_user FROM library_entries WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    /// z-scores with both dark kinds at 0.0.
    fn z(bright: f64, grand: f64, energy: f64) -> MoodZ {
        MoodZ { bright, grand, energy, sad: 0.0, scary: 0.0 }
    }

    /// (sad z, scary z, user kind) as stored.
    fn dark_kind_row(conn: &Connection, id: &str) -> (Option<f64>, Option<f64>, Option<String>) {
        conn.query_row(
            "SELECT mood_sad, mood_scary, dark_kind_user FROM library_entries WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }

    #[test]
    fn user_dark_kind_round_trips_and_clears() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let b = music_entry(&conn, "/b.mp3");

        set_user_dark_kind(&conn, &[a.clone(), b.clone()], Some(DarkKind::Scary)).unwrap();
        assert_eq!(dark_kind_row(&conn, &a), (None, None, s("scary")));
        set_user_dark_kind(&conn, std::slice::from_ref(&a), Some(DarkKind::Sad)).unwrap();
        assert_eq!(dark_kind_row(&conn, &a).2, s("sad"));
        assert_eq!(dark_kind_row(&conn, &b).2, s("scary"));

        // Clearing is "use auto"; the other entry keeps its kind.
        set_user_dark_kind(&conn, std::slice::from_ref(&a), None).unwrap();
        assert_eq!(dark_kind_row(&conn, &a).2, None);
        assert_eq!(dark_kind_row(&conn, &b).2, s("scary"));
        // No mood step is touched.
        assert_eq!(mood_user(&conn, &b), (None, None, None));
    }

    #[test]
    fn apply_auto_mood_writes_dark_kind_scores_and_keeps_user_kind() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        set_user_dark_kind(&conn, std::slice::from_ref(&a), Some(DarkKind::Sad)).unwrap();

        let mood = MoodZ { sad: -0.3, scary: 1.4, ..z(-1.0, 0.0, 0.5) };
        assert!(apply_auto_mood(&conn, &a, Some(mood)).unwrap());
        assert_eq!(dark_kind_row(&conn, &a), (Some(-0.3), Some(1.4), s("sad")));
        assert!(!apply_auto_mood(&conn, &a, Some(mood)).unwrap());
        // A change in one kind alone is a change.
        assert!(apply_auto_mood(&conn, &a, Some(MoodZ { scary: 1.5, ..mood })).unwrap());
        assert_eq!(dark_kind_row(&conn, &a).1, Some(1.5));

        assert!(apply_auto_mood(&conn, &a, None).unwrap());
        assert_eq!(dark_kind_row(&conn, &a), (None, None, s("sad")));
    }

    #[test]
    fn user_mood_step_round_trips_and_clears() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let b = music_entry(&conn, "/b.mp3");

        set_user_mood_step(&conn, &[a.clone(), b.clone()], MoodScale::Bright, Some(MoodStep::High))
            .unwrap();
        set_user_mood_step(&conn, std::slice::from_ref(&a), MoodScale::Grand, Some(MoodStep::Low)).unwrap();
        set_user_mood_step(&conn, std::slice::from_ref(&a), MoodScale::Energy, Some(MoodStep::Mid)).unwrap();
        assert_eq!(mood_user(&conn, &a), (s("high"), s("low"), s("mid")));
        assert_eq!(mood_user(&conn, &b), (s("high"), None, None));
        // The analyzer's values are untouched.
        assert_eq!(mood_auto(&conn, &a), (None, None, None));

        // Clearing one scale is "use auto" for that scale alone.
        set_user_mood_step(&conn, std::slice::from_ref(&a), MoodScale::Bright, None).unwrap();
        assert_eq!(mood_user(&conn, &a), (None, s("low"), s("mid")));
        set_user_mood_step(&conn, std::slice::from_ref(&a), MoodScale::Energy, None).unwrap();
        assert_eq!(mood_user(&conn, &a), (None, s("low"), None));
        assert_eq!(mood_user(&conn, &b).0, s("high"));
    }

    #[test]
    fn apply_auto_mood_leaves_user_steps() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        set_user_mood_step(&conn, std::slice::from_ref(&a), MoodScale::Bright, Some(MoodStep::Low)).unwrap();
        set_user_mood_step(&conn, std::slice::from_ref(&a), MoodScale::Grand, Some(MoodStep::High)).unwrap();

        assert!(apply_auto_mood(&conn, &a, Some(z(1.2, -0.4, 0.7))).unwrap());
        assert_eq!(mood_auto(&conn, &a), (Some(1.2), Some(-0.4), Some(0.7)));
        assert_eq!(mood_user(&conn, &a), (s("low"), s("high"), None));
        // Same values again change nothing; any one scale moving does.
        assert!(!apply_auto_mood(&conn, &a, Some(z(1.2, -0.4, 0.7))).unwrap());
        assert!(apply_auto_mood(&conn, &a, Some(z(1.2, -0.4, 0.8))).unwrap());
        assert_eq!(mood_auto(&conn, &a).2, Some(0.8));

        assert!(apply_auto_mood(&conn, &a, None).unwrap());
        assert_eq!(mood_auto(&conn, &a), (None, None, None));
        assert_eq!(mood_user(&conn, &a), (s("low"), s("high"), None));
        assert!(!apply_auto_mood(&conn, &a, None).unwrap());
        // A missing entry is no change.
        assert!(!apply_auto_mood(&conn, "nope", Some(z(1.0, 1.0, 1.0))).unwrap());
    }

    #[test]
    fn switching_to_ambiance_clears_mood() {
        let conn = open_memory_db();
        let auto = music_entry(&conn, "/a.mp3");
        let user = music_entry(&conn, "/b.mp3");
        let mood = MoodZ { sad: 0.4, scary: -0.4, ..z(1.0, -1.0, 0.5) };
        apply_auto_mood(&conn, &auto, Some(mood)).unwrap();
        apply_auto_mood(&conn, &user, Some(mood)).unwrap();
        for scale in [MoodScale::Bright, MoodScale::Grand, MoodScale::Energy] {
            set_user_mood_step(&conn, std::slice::from_ref(&user), scale, Some(MoodStep::Mid)).unwrap();
        }
        set_user_dark_kind(&conn, std::slice::from_ref(&user), Some(DarkKind::Scary)).unwrap();

        for id in [&auto, &user] {
            update_library_entry_type(&conn, id, "ambiance").unwrap();
        }

        for id in [&auto, &user] {
            assert_eq!(mood_auto(&conn, id), (None, None, None));
            assert_eq!(mood_user(&conn, id), (None, None, None));
            assert_eq!(dark_kind_row(&conn, id), (None, None, None));
        }
    }

}
