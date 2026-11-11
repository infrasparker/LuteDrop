use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::database::now_millis;
use crate::database::DbError;
use crate::database::settings;
use crate::models::{LibraryEntryUsage, MoodResetImpact, MoodVocabulary, Role, MOOD_LEVELS};

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

/// Updates the `entry_type` column for the given entry. Roles and moods only
/// apply to music, so when an entry stops being music its role and moods
/// (both layers) are cleared in the same transaction (changing the type is
/// itself a user action). Its tags are left alone.
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
        conn.execute("UPDATE library_entries SET role_user = NULL WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to clear role: {e}"))?;
        // Ambiance has no moods: both layers go, and so does the review
        // time (ambiance is never reviewed).
        conn.execute("DELETE FROM library_entry_moods WHERE library_entry_id = ?1", params![id])
            .map_err(|e| format!("Failed to clear moods: {e}"))?;
        conn.execute("UPDATE library_entries SET mood_reviewed_at = NULL WHERE id = ?1", params![id])
            .map_err(|e| format!("Failed to clear review time: {e}"))?;
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

/// Stores the ReplayGain values read from an entry's file (NULL for a value
/// the file does not have).
pub fn set_replaygain(
    conn: &Connection,
    id: &str,
    rg: &crate::replaygain::ReplayGain,
) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET rg_track_gain_db = ?1, rg_track_peak = ?2 WHERE id = ?3",
        params![rg.track_gain_db, rg.track_peak, id],
    )
    .map_err(|e| format!("Failed to store ReplayGain: {e}"))?;
    Ok(())
}

/// `(id, file_path)` of every entry with no ReplayGain gain stored yet: new
/// since v17, or a file without the tag (those are read again each start).
pub fn entries_without_replaygain(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, file_path FROM library_entries WHERE rg_track_gain_db IS NULL ORDER BY name")
        .map_err(|e| format!("Failed to prepare ReplayGain backfill query: {e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| format!("Failed to query ReplayGain backfill: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read ReplayGain backfill rows: {e}"))?;
    Ok(rows)
}

/// `(id, file_path)` of every entry with no duration stored: its file could
/// not be decoded when it was imported (offline share, or a format the
/// decoder did not support yet). The startup backfill measures them again.
pub fn entries_without_duration(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, file_path FROM library_entries WHERE duration IS NULL ORDER BY name")
        .map_err(|e| format!("Failed to prepare duration backfill query: {e}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| format!("Failed to query duration backfill: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to read duration backfill rows: {e}"))?;
    Ok(rows)
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

// ─── Entry tags ───────────────────────────────────────────────────────────────
//
// Each `library_entry_tags` row is one tag the user put on one entry.

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

/// Puts a tag on an entry; a tag it already has is left as it is.
fn link_tag(conn: &Connection, entry_id: &str, tag_id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO library_entry_tags (library_entry_id, tag_id) VALUES (?1, ?2)",
        params![entry_id, tag_id],
    )
    .map_err(|e| format!("Failed to add tag: {e}"))?;
    Ok(())
}

/// Takes a tag off an entry.
fn unlink_tag(conn: &Connection, entry_id: &str, tag_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM library_entry_tags WHERE library_entry_id = ?1 AND tag_id = ?2",
        params![entry_id, tag_id],
    )
    .map_err(|e| format!("Failed to remove tag: {e}"))?;
    Ok(())
}

/// Associates a tag (creating it if necessary) with the given library entry.
/// Used for tags chosen at import time.
pub fn add_tag_to_entry(
    conn: &Connection,
    entry_id: &str,
    tag_name: &str,
) -> Result<(), String> {
    let tag_id = ensure_tag(conn, tag_name)?;
    link_tag(conn, entry_id, &tag_id)
}

/// Deletes tag rows that no entry row references.
pub(crate) const SWEEP_ORPHAN_TAGS: &str =
    "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM library_entry_tags)";

pub fn sweep_orphan_tags(conn: &Connection) -> Result<(), String> {
    conn.execute(SWEEP_ORPHAN_TAGS, [])
        .map_err(|e| format!("Failed to sweep orphaned tags: {e}"))?;
    Ok(())
}

/// Atomically replaces an entry's tags with `tags`, in one transaction, and
/// sweeps tag rows no entry uses any more.
pub fn set_entry_tags(conn: &Connection, entry_id: &str, tags: &[String]) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin tag transaction: {e}"))?;

    let mut wanted_ids = Vec::new();
    for tag_name in tags {
        wanted_ids.push(ensure_tag(conn, tag_name)?);
    }
    let wanted = serde_json::to_string(&wanted_ids).map_err(|e| format!("Failed to encode tag ids: {e}"))?;
    conn.execute(
        "DELETE FROM library_entry_tags
         WHERE library_entry_id = ?1 AND tag_id NOT IN (SELECT value FROM json_each(?2))",
        params![entry_id, wanted],
    )
    .map_err(|e| format!("Failed to remove tags: {e}"))?;
    for tag_id in &wanted_ids {
        link_tag(conn, entry_id, tag_id)?;
    }

    sweep_orphan_tags(conn)?;

    tx.commit().map_err(|e| format!("Failed to commit tag transaction: {e}"))?;
    Ok(())
}

/// Adds `add` to and removes `remove` from every entry in `entry_ids`, in
/// one transaction. Orphaned tag rows are swept at the end.
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
        for tag_id in &remove_ids {
            unlink_tag(conn, entry_id, tag_id)?;
        }
        for tag_id in &add_ids {
            link_tag(conn, entry_id, tag_id)?;
        }
    }

    sweep_orphan_tags(conn)?;

    tx.commit()
        .map_err(|e| format!("Failed to commit bulk tag transaction: {e}"))?;
    Ok(())
}

// ─── Role ─────────────────────────────────────────────────────────────────────

/// Sets the user's role on each entry. `None` clears it, so the entry shows
/// the default, Non-combat.
pub fn set_user_role(
    conn: &Connection,
    entry_ids: &[String],
    role: Option<Role>,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin role transaction: {e}"))?;
    let value = role.map(|r| r.as_db_str());
    for entry_id in entry_ids {
        conn.execute("UPDATE library_entries SET role_user = ?2 WHERE id = ?1", params![entry_id, value])
            .map_err(|e| format!("Failed to set role: {e}"))?;
    }
    tx.commit().map_err(|e| format!("Failed to commit role transaction: {e}"))?;
    Ok(())
}

/// The level a user may set, snapped exactly onto `MOOD_LEVELS` (a value
/// within 1e-9 of one counts, so float noise from the UI is harmless), or an
/// error naming the allowed levels.
fn checked_level(level: f64) -> Result<f64, String> {
    MOOD_LEVELS
        .into_iter()
        .find(|l| (l - level).abs() < 1e-9)
        .ok_or_else(|| format!("Mood level {level} is not one of 0, 0.25, 0.5, 0.75, 1"))
}

/// Sets the user's level of one mood on each music entry (`None` clears it,
/// so the analyzer's suggestion shows again) and marks each one reviewed now.
/// Ambiance entries have no moods and are skipped; so are missing ones. A
/// level outside `MOOD_LEVELS`, or a mood not in the vocabulary, is refused
/// before anything is written.
///
/// Levels do not train anything yet: learned mood heads are a later release,
/// which regresses them on `user_level`.
pub fn set_user_mood_level(
    conn: &Connection,
    entry_ids: &[String],
    mood: &str,
    level: Option<f64>,
) -> Result<(), String> {
    let level = level.map(checked_level).transpose()?;
    if !settings::load_mood_vocabulary(conn)?.contains(mood) {
        return Err(format!("There is no mood \"{mood}\""));
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin mood transaction: {e}"))?;
    let now = now_millis();
    for entry_id in entry_ids {
        let is_music = conn
            .execute(
                "UPDATE library_entries SET mood_reviewed_at = ?2 WHERE id = ?1 AND entry_type = 'music'",
                params![entry_id, now],
            )
            .map_err(|e| format!("Failed to mark entry reviewed: {e}"))?
            > 0;
        if !is_music {
            continue;
        }
        match level {
            Some(level) => conn.execute(
                "INSERT INTO library_entry_moods (library_entry_id, mood, user_level)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (library_entry_id, mood) DO UPDATE SET user_level = excluded.user_level",
                params![entry_id, mood, level],
            ),
            // No row means no user level already; never insert an empty one.
            None => conn.execute(
                "UPDATE library_entry_moods SET user_level = NULL WHERE library_entry_id = ?1 AND mood = ?2",
                params![entry_id, mood],
            ),
        }
        .map_err(|e| format!("Failed to set mood level: {e}"))?;
    }
    tx.commit().map_err(|e| format!("Failed to commit mood transaction: {e}"))?;
    Ok(())
}

/// Writes the analyzer's 0..1 value of each given mood for an entry. Only
/// `auto_score` is written; the user's levels and review time are never
/// touched. The caller checks the entry is music. Opens no transaction of
/// its own.
pub fn apply_auto_moods(conn: &Connection, entry_id: &str, scores: &[(String, f64)]) -> Result<(), String> {
    for (mood, score) in scores {
        conn.execute(
            "INSERT INTO library_entry_moods (library_entry_id, mood, auto_score)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (library_entry_id, mood) DO UPDATE SET auto_score = excluded.auto_score",
            params![entry_id, mood, score],
        )
        .map_err(|e| format!("Failed to write auto mood: {e}"))?;
    }
    Ok(())
}

/// How many user-set levels each mood id holds, over the whole library.
#[cfg(test)]
fn user_level_count(conn: &Connection, mood: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM library_entry_moods WHERE mood = ?1 AND user_level IS NOT NULL",
        params![mood],
        |r| r.get(0),
    )
    .unwrap()
}

/// The error `save_mood_vocabulary` gives while the stored one is unreadable.
pub const MOOD_VOCABULARY_UNREADABLE: &str = "Stored mood list is unreadable; fix or reset it first";

/// Saves a new mood vocabulary (Settings → Moods), in one transaction. It is
/// normalized and checked first (see `MoodVocabulary::normalized`); a mood
/// left out of it is removed, so its `library_entry_moods` rows go, user
/// levels included (the UI confirms that first). Renaming a mood or editing
/// its prompts keeps its rows; the calibration is not touched, so changed
/// moods wait for Recalibrate. Returns the saved vocabulary. Refused while
/// the stored vocabulary is unreadable: the app then shows the defaults, and
/// saving a list built from them would delete the rows of the real moods.
pub fn save_mood_vocabulary(conn: &Connection, vocabulary: &MoodVocabulary) -> Result<MoodVocabulary, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin vocabulary transaction: {e}"))?;
    let vocabulary = write_mood_vocabulary(conn, vocabulary)?;
    tx.commit().map_err(|e| format!("Failed to commit vocabulary transaction: {e}"))?;
    Ok(vocabulary)
}

/// `save_mood_vocabulary`'s work without its own transaction, for a caller
/// that writes more in the same one (the Tune split). The caller must hold
/// a transaction, or a failure can leave the rows and the list out of step.
pub fn write_mood_vocabulary(conn: &Connection, vocabulary: &MoodVocabulary) -> Result<MoodVocabulary, String> {
    if let Some(problem) = settings::mood_vocabulary_problem(conn)? {
        log::warn!("Refusing to save the mood vocabulary over an unreadable one: {problem}");
        return Err(MOOD_VOCABULARY_UNREADABLE.to_string());
    }
    let vocabulary = vocabulary.normalized()?;
    let kept = serde_json::to_string(&vocabulary.ids()).map_err(|e| format!("Failed to encode mood ids: {e}"))?;
    conn.execute(
        "DELETE FROM library_entry_moods WHERE mood NOT IN (SELECT value FROM json_each(?1))",
        params![kept],
    )
    .map_err(|e| format!("Failed to delete removed moods: {e}"))?;
    settings::save_mood_vocabulary(conn, &vocabulary)?;
    Ok(vocabulary)
}

/// The default mood ids (Energy included) as a JSON array, for `json_each`.
fn default_mood_ids_json() -> Result<String, String> {
    serde_json::to_string(&MoodVocabulary::defaults().ids()).map_err(|e| format!("Failed to encode mood ids: {e}"))
}

/// What `reset_mood_vocabulary` would delete (see `MoodResetImpact`).
pub fn mood_reset_impact(conn: &Connection) -> Result<MoodResetImpact, String> {
    let ids = default_mood_ids_json()?;
    let mut stmt = conn
        .prepare(
            "SELECT mood, COUNT(*), COUNT(user_level) FROM library_entry_moods
             WHERE mood NOT IN (SELECT value FROM json_each(?1))
             GROUP BY mood ORDER BY mood",
        )
        .map_err(|e| format!("Failed to count mood rows: {e}"))?;
    let groups = stmt
        .query_map(params![ids], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, u32>(2)?)))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("Failed to count mood rows: {e}"))?;
    Ok(MoodResetImpact {
        moods: groups.iter().map(|(m, _, _)| m.clone()).collect(),
        rows: groups.iter().map(|(_, n, _)| n).sum(),
        user_levels: groups.iter().map(|(_, _, n)| n).sum(),
    })
}

/// Reset to defaults (Settings → Moods), in one transaction: the shipped
/// default vocabulary replaces the stored one, and mood rows whose id is not
/// a default mood go (user levels included; the UI shows how many first).
/// Rows of the default ids keep their user levels and suggestions; moods
/// whose prompts differ from the calibration then show "needs
/// recalibration". Unlike `save_mood_vocabulary`, this writes even while the
/// stored vocabulary is unreadable: it is the way out of that state, and it
/// deletes only what the confirm dialog counted from the rows themselves.
pub fn reset_mood_vocabulary(conn: &Connection) -> Result<(), String> {
    if let Some(problem) = settings::mood_vocabulary_problem(conn)? {
        log::warn!("Resetting an unreadable mood vocabulary to the defaults: {problem}");
    }
    let ids = default_mood_ids_json()?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin reset transaction: {e}"))?;
    conn.execute(
        "DELETE FROM library_entry_moods WHERE mood NOT IN (SELECT value FROM json_each(?1))",
        params![ids],
    )
    .map_err(|e| format!("Failed to delete moods outside the defaults: {e}"))?;
    settings::save_mood_vocabulary(conn, &MoodVocabulary::defaults())?;
    tx.commit().map_err(|e| format!("Failed to commit reset transaction: {e}"))?;
    Ok(())
}

/// Clears what the analyzer said about moods, for Recalibrate, in one
/// transaction: the calibration and every `auto_score` (rows left with no
/// level go), so the next run takes a new calibration from the current
/// prompts and scores everything again. User-set levels and review times are
/// untouched.
pub fn clear_mood_analysis(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin recalibrate transaction: {e}"))?;
    settings::clear_mood_calibration(conn)?;
    conn.execute_batch(
        "UPDATE library_entry_moods SET auto_score = NULL;
         DELETE FROM library_entry_moods WHERE user_level IS NULL;",
    )
    .map_err(|e| format!("Failed to clear mood scores: {e}"))?;
    tx.commit().map_err(|e| format!("Failed to commit recalibrate transaction: {e}"))?;
    Ok(())
}

// Returns the names of the given library entry's tags, sorted.
#[cfg(test)]
pub fn get_tags_for_entry(
    conn: &Connection,
    entry_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.name
             FROM tags t
             JOIN library_entry_tags lt ON lt.tag_id = t.id
             WHERE lt.library_entry_id = ?1
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

/// Returns all tag names that at least one library entry has, sorted.
pub fn list_all_tags(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT t.name FROM tags t
             JOIN library_entry_tags lt ON lt.tag_id = t.id
             ORDER BY t.name ASC",
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

        set_entry_tags(&conn, &e, &["kept".to_string(), "new".to_string()]).unwrap();

        assert_eq!(get_tags_for_entry(&conn, &e).unwrap(), vec!["kept", "new"]);
        assert_eq!(list_all_tags(&conn).unwrap(), vec!["kept", "new"]);
        // Tags nothing references any more are swept, "old-only" included.
        let orphans: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags WHERE name IN ('stray', 'old-only')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0);

        // An empty list removes every tag.
        set_entry_tags(&conn, &e, &[]).unwrap();
        assert!(get_tags_for_entry(&conn, &e).unwrap().is_empty());
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 0);
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

        set_entry_tags(&conn, &a, &["combat".into(), "night".into()]).expect("tag a");
        set_entry_tags(&conn, &b, &["night".into()]).expect("tag b");
        set_entry_tags(&conn, &c, &["night".into()]).expect("tag c");

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

        // Removing a tag everywhere drops it from the library tag list.
        bulk_edit_entry_tags(&conn, std::slice::from_ref(&c), &[], &["night".into()]).expect("bulk remove");
        assert_eq!(list_all_tags(&conn).unwrap(), vec!["boss", "combat"]);
    }

    fn new_entry(conn: &Connection, path: &str) -> String {
        insert_library_entry(conn, path, None, "T", None, None, None, "music").unwrap()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn type_change_keeps_tags() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "night").unwrap();

        assert!(!update_library_entry_type(&conn, &e, "ambiance").unwrap());
        assert_eq!(get_tags_for_entry(&conn, &e).unwrap(), vec!["night"]);
        assert!(update_library_entry_type(&conn, &e, "music").unwrap());
        assert_eq!(get_tags_for_entry(&conn, &e).unwrap(), vec!["night"]);
    }

    #[test]
    fn bulk_remove_only_touches_entries_with_the_tag() {
        let conn = open_memory_db();
        let a = new_entry(&conn, "/a.mp3");
        let b = new_entry(&conn, "/b.mp3");
        add_tag_to_entry(&conn, &a, "brass").unwrap();
        add_tag_to_entry(&conn, &b, "night").unwrap();
        bulk_edit_entry_tags(&conn, &[a.clone(), b.clone()], &[], &["Brass ".into(), "missing".into()]).unwrap();
        assert!(get_tags_for_entry(&conn, &a).unwrap().is_empty());
        assert_eq!(get_tags_for_entry(&conn, &b).unwrap(), vec!["night"]);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags WHERE name = 'brass'"), 0);
        // Adding a tag an entry already has changes nothing.
        bulk_edit_entry_tags(&conn, std::slice::from_ref(&b), &["night".into()], &[]).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM library_entry_tags"), 1);
    }

    #[test]
    fn delete_entry_cascades_tag_rows() {
        let conn = open_memory_db();
        let e = new_entry(&conn, "/a.mp3");
        add_tag_to_entry(&conn, &e, "x").unwrap();
        conn.execute("DELETE FROM library_entries WHERE id = ?1", params![e]).unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM library_entry_tags"), 0);
    }

    // ── Role ──────────────────────────────────────────────────────────────────

    fn role_user(conn: &Connection, id: &str) -> Option<String> {
        conn.query_row("SELECT role_user FROM library_entries WHERE id = ?1", params![id], |r| r.get(0))
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

        set_user_role(&conn, &[a.clone(), b.clone()], Some(Role::Combat)).unwrap();
        assert_eq!(role_user(&conn, &a).as_deref(), Some("combat"));
        assert_eq!(role_user(&conn, &b).as_deref(), Some("combat"));

        set_user_role(&conn, std::slice::from_ref(&a), None).unwrap();
        assert_eq!(role_user(&conn, &a), None);
        assert_eq!(role_user(&conn, &b).as_deref(), Some("combat"));
    }

    #[test]
    fn leaving_music_clears_the_role() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        set_user_role(&conn, std::slice::from_ref(&a), Some(Role::NonCombat)).unwrap();

        update_library_entry_type(&conn, &a, "ambiance").unwrap();

        assert_eq!(role_user(&conn, &a), None);
    }

    // ── Moods ─────────────────────────────────────────────────────────────────

    /// (auto_score, user_level) of one mood as stored; `None` without a row.
    fn mood_row(conn: &Connection, id: &str, mood: &str) -> Option<(Option<f64>, Option<f64>)> {
        conn.query_row(
            "SELECT auto_score, user_level FROM library_entry_moods WHERE library_entry_id = ?1 AND mood = ?2",
            params![id, mood],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap()
    }

    fn mood_rows(conn: &Connection, id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM library_entry_moods WHERE library_entry_id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn reviewed_at(conn: &Connection, id: &str) -> Option<String> {
        conn.query_row("SELECT mood_reviewed_at FROM library_entries WHERE id = ?1", params![id], |r| r.get(0))
            .unwrap()
    }

    /// Every id of the default vocabulary at `v`.
    fn all_moods(v: f64) -> Vec<(String, f64)> {
        MoodVocabulary::defaults().ids().into_iter().map(|m| (m, v)).collect()
    }

    /// How many ids the default vocabulary has (nine moods plus Energy).
    const MOOD_COUNT: i64 = 10;

    #[test]
    fn user_mood_level_sets_review_time_and_clears_to_suggestion() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let b = music_entry(&conn, "/b.mp3");
        let amb = insert_library_entry(&conn, "/r.ogg", None, "R", None, None, None, "ambiance").unwrap();
        apply_auto_moods(&conn, &a, &all_moods(0.6)).unwrap();

        let before = now_millis().parse::<u128>().unwrap();
        set_user_mood_level(&conn, &[a.clone(), b.clone(), amb.clone(), "nope".into()], "grim", Some(0.75))
            .unwrap();
        assert_eq!(mood_row(&conn, &a, "grim"), Some((Some(0.6), Some(0.75))));
        assert!(reviewed_at(&conn, &a).unwrap().parse::<u128>().unwrap() >= before);
        // An entry without analyzer values gets a row with the level only.
        assert_eq!(mood_row(&conn, &b, "grim"), Some((None, Some(0.75))));
        assert_eq!(mood_rows(&conn, &b), 1);
        assert!(reviewed_at(&conn, &b).is_some());
        // Ambiance has no moods.
        assert_eq!(mood_rows(&conn, &amb), 0);
        assert_eq!(reviewed_at(&conn, &amb), None);
        // Other moods are untouched.
        assert_eq!(mood_row(&conn, &a, "heroic"), Some((Some(0.6), None)));

        // `None` goes back to the suggestion; the entry stays reviewed.
        set_user_mood_level(&conn, std::slice::from_ref(&a), "grim", None).unwrap();
        assert_eq!(mood_row(&conn, &a, "grim"), Some((Some(0.6), None)));
        assert!(reviewed_at(&conn, &a).is_some());
        assert_eq!(mood_row(&conn, &b, "grim"), Some((None, Some(0.75))));
        // Clearing a mood with no row adds none.
        set_user_mood_level(&conn, std::slice::from_ref(&b), "quirky", None).unwrap();
        assert_eq!(mood_row(&conn, &b, "quirky"), None);
    }

    #[test]
    fn user_mood_level_takes_only_the_five_levels() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        for level in [0.0, 0.25, 0.5, 0.75, 1.0] {
            set_user_mood_level(&conn, std::slice::from_ref(&a), "energy", Some(level)).unwrap();
            assert_eq!(mood_row(&conn, &a, "energy"), Some((None, Some(level))));
        }
        // Float noise snaps onto the level.
        set_user_mood_level(&conn, std::slice::from_ref(&a), "energy", Some(0.1 + 0.15)).unwrap();
        assert_eq!(mood_row(&conn, &a, "energy"), Some((None, Some(0.25))));
        for bad in [0.3, -0.25, 1.25, f64::NAN] {
            assert!(set_user_mood_level(&conn, std::slice::from_ref(&a), "energy", Some(bad)).is_err());
        }
        assert_eq!(mood_row(&conn, &a, "energy"), Some((None, Some(0.25))));
        // The table refuses anything else too.
        assert!(conn
            .execute(
                "UPDATE library_entry_moods SET user_level = 0.3 WHERE library_entry_id = ?1",
                params![a],
            )
            .is_err());
    }

    #[test]
    fn apply_auto_moods_writes_only_the_analyzer_layer() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        set_user_mood_level(&conn, std::slice::from_ref(&a), "peaceful", Some(1.0)).unwrap();
        let reviewed = reviewed_at(&conn, &a);

        apply_auto_moods(&conn, &a, &all_moods(0.4)).unwrap();
        assert_eq!(mood_rows(&conn, &a), MOOD_COUNT);
        assert_eq!(mood_row(&conn, &a, "peaceful"), Some((Some(0.4), Some(1.0))));
        apply_auto_moods(&conn, &a, &[("peaceful".into(), 0.9), ("energy".into(), 0.1)]).unwrap();
        assert_eq!(mood_row(&conn, &a, "peaceful"), Some((Some(0.9), Some(1.0))));
        assert_eq!(mood_row(&conn, &a, "energy"), Some((Some(0.1), None)));
        assert_eq!(mood_row(&conn, &a, "grim"), Some((Some(0.4), None)));
        // The user's levels and review time are never touched.
        assert_eq!(reviewed_at(&conn, &a), reviewed);
    }

    #[test]
    fn switching_to_ambiance_deletes_moods() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let other = music_entry(&conn, "/b.mp3");
        for id in [&a, &other] {
            apply_auto_moods(&conn, id, &all_moods(0.5)).unwrap();
            set_user_mood_level(&conn, std::slice::from_ref(id), "wondrous", Some(0.25)).unwrap();
        }

        update_library_entry_type(&conn, &a, "ambiance").unwrap();

        assert_eq!(mood_rows(&conn, &a), 0);
        assert_eq!(reviewed_at(&conn, &a), None);
        assert_eq!(mood_rows(&conn, &other), MOOD_COUNT);
    }

    #[test]
    fn deleting_an_entry_deletes_its_moods() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        apply_auto_moods(&conn, &a, &all_moods(0.5)).unwrap();
        delete_library_entry(&conn, &a).unwrap();
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM library_entry_moods", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn user_mood_level_refuses_a_mood_outside_the_vocabulary() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let err = set_user_mood_level(&conn, std::slice::from_ref(&a), "menacing", Some(1.0)).unwrap_err();
        assert!(err.contains("menacing"));
        assert_eq!(mood_rows(&conn, &a), 0);
        assert_eq!(reviewed_at(&conn, &a), None);
    }

    #[test]
    fn saving_the_vocabulary_deletes_removed_moods_only() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        apply_auto_moods(&conn, &a, &all_moods(0.5)).unwrap();
        set_user_mood_level(&conn, std::slice::from_ref(&a), "quirky", Some(1.0)).unwrap();
        set_user_mood_level(&conn, std::slice::from_ref(&a), "grim", Some(0.25)).unwrap();

        // Rename Grim, drop Quirky, add Spooky.
        let mut v = MoodVocabulary::defaults();
        v.moods.retain(|m| m.id != "quirky");
        v.moods.iter_mut().find(|m| m.id == "grim").unwrap().name = "Dread".into();
        v.moods.push(crate::models::MoodDef {
            id: "spooky".into(),
            name: " Spooky ".into(),
            prompts: vec!["ghosts".into(), "".into()],
        });
        let saved = save_mood_vocabulary(&conn, &v).unwrap();
        assert_eq!(saved.moods.last().unwrap().name, "Spooky");
        assert_eq!(settings::load_mood_vocabulary(&conn).unwrap(), saved);

        assert_eq!(mood_row(&conn, &a, "quirky"), None);
        assert_eq!(user_level_count(&conn, "quirky"), 0);
        assert_eq!(mood_row(&conn, &a, "grim"), Some((Some(0.5), Some(0.25))));
        assert_eq!(mood_row(&conn, &a, "energy"), Some((Some(0.5), None)));
        assert_eq!(mood_rows(&conn, &a), MOOD_COUNT - 1);
        // The new mood can be set right away.
        set_user_mood_level(&conn, std::slice::from_ref(&a), "spooky", Some(0.75)).unwrap();

        // A bad vocabulary changes nothing.
        let mut bad = saved.clone();
        bad.moods.clear();
        assert!(save_mood_vocabulary(&conn, &bad).is_err());
        assert_eq!(settings::load_mood_vocabulary(&conn).unwrap(), saved);
        assert_eq!(mood_rows(&conn, &a), MOOD_COUNT);
    }

    #[test]
    fn saving_over_an_unreadable_mood_vocabulary_is_refused() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let mut v = MoodVocabulary::defaults();
        v.moods.push(crate::models::MoodDef { id: "spooky".into(), name: "Spooky".into(), prompts: vec!["ghosts".into()] });
        save_mood_vocabulary(&conn, &v).unwrap();
        set_user_mood_level(&conn, std::slice::from_ref(&a), "spooky", Some(0.75)).unwrap();

        // A corrupt row reads as the defaults, which lack Spooky; saving them
        // back would delete the user's level, so it is refused.
        conn.execute("UPDATE meta SET value = '{oops' WHERE key = 'mood_vocabulary'", []).unwrap();
        assert_eq!(settings::load_mood_vocabulary(&conn).unwrap(), MoodVocabulary::defaults());
        let err = save_mood_vocabulary(&conn, &MoodVocabulary::defaults()).unwrap_err();
        assert_eq!(err, MOOD_VOCABULARY_UNREADABLE);
        assert_eq!(mood_row(&conn, &a, "spooky"), Some((None, Some(0.75))));
    }

    #[test]
    fn reset_mood_vocabulary_works_over_an_unreadable_row() {
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        let b = music_entry(&conn, "/b.mp3");
        let mut v = MoodVocabulary::defaults();
        v.moods.retain(|m| m.id != "quirky");
        v.moods[0].prompts = vec!["calm".into()];
        v.moods.push(crate::models::MoodDef { id: "spooky".into(), name: "Spooky".into(), prompts: vec!["ghosts".into()] });
        save_mood_vocabulary(&conn, &v).unwrap();
        apply_auto_moods(&conn, &a, &[("spooky".into(), 0.4), ("peaceful".into(), 0.6)]).unwrap();
        set_user_mood_level(&conn, std::slice::from_ref(&a), "spooky", Some(0.75)).unwrap();
        set_user_mood_level(&conn, std::slice::from_ref(&b), "spooky", Some(0.25)).unwrap();
        set_user_mood_level(&conn, std::slice::from_ref(&a), "peaceful", Some(1.0)).unwrap();
        conn.execute("UPDATE meta SET value = '{oops' WHERE key = 'mood_vocabulary'", []).unwrap();
        assert!(settings::mood_vocabulary_problem(&conn).unwrap().is_some());

        let impact = mood_reset_impact(&conn).unwrap();
        assert_eq!(impact, MoodResetImpact { moods: vec!["spooky".into()], rows: 2, user_levels: 2 });

        reset_mood_vocabulary(&conn).unwrap();
        assert_eq!(settings::mood_vocabulary_problem(&conn).unwrap(), None);
        assert_eq!(settings::load_mood_vocabulary(&conn).unwrap(), MoodVocabulary::defaults());
        // Spooky's rows are gone; Peaceful keeps both layers.
        assert_eq!(mood_row(&conn, &a, "spooky"), None);
        assert_eq!(mood_row(&conn, &b, "spooky"), None);
        assert_eq!(mood_row(&conn, &a, "peaceful"), Some((Some(0.6), Some(1.0))));
        assert_eq!(mood_reset_impact(&conn).unwrap(), MoodResetImpact { moods: vec![], rows: 0, user_levels: 0 });
        // Saving works again.
        save_mood_vocabulary(&conn, &MoodVocabulary::defaults()).unwrap();
    }

    #[test]
    fn clearing_mood_analysis_keeps_user_levels() {
        use crate::autotag::plan::MoodCalibration;
        let conn = open_memory_db();
        let a = music_entry(&conn, "/a.mp3");
        apply_auto_moods(&conn, &a, &all_moods(0.5)).unwrap();
        set_user_mood_level(&conn, std::slice::from_ref(&a), "grim", Some(0.25)).unwrap();
        let reviewed = reviewed_at(&conn, &a);
        conn.execute("INSERT INTO entry_analysis (library_entry_id, status) VALUES (?1, 'embedded')", params![a])
            .unwrap();
        settings::save_mood_calibration(
            &conn,
            &MoodCalibration { calibrated_at: "1".into(), tracks: 50, prompts: Default::default(), stats: Default::default(), source: Default::default(), reference_set: None },
        )
        .unwrap();

        clear_mood_analysis(&conn).unwrap();

        assert_eq!(settings::load_mood_calibration(&conn).unwrap(), None);
        assert_eq!(mood_row(&conn, &a, "grim"), Some((None, Some(0.25))));
        assert_eq!(mood_rows(&conn, &a), 1);
        assert_eq!(reviewed_at(&conn, &a), reviewed);
        // The embedding is still good.
        let status: String = conn
            .query_row("SELECT status FROM entry_analysis WHERE library_entry_id = ?1", params![a], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "embedded");
    }

}
