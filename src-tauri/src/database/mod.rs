pub mod configurations;
pub mod hierarchy;
pub mod library;

use rusqlite::{params, Connection, Result as SqlResult};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

use crate::models::{
    AppState, Clip, Comp, Configuration, Group, LibraryEntry, Track, TrackMode, TrackTransition,
};

/// Opens (or creates) the SQLite database at `path`, enables WAL mode and
/// foreign-key enforcement, then runs the schema initialiser.
pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open database: {e}"))?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("Failed to set PRAGMAs: {e}"))?;

    init_schema(&conn).map_err(|e| format!("Failed to initialise schema: {e}"))?;

    Ok(conn)
}

/// Creates all application tables (idempotent – uses `IF NOT EXISTS`).
pub fn init_schema(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS library_entries (
            id           TEXT PRIMARY KEY,
            file_path    TEXT NOT NULL UNIQUE,
            content_hash TEXT,
            name         TEXT NOT NULL,
            duration     REAL,
            sample_rate  INTEGER,
            file_size    INTEGER,
            volume       REAL NOT NULL DEFAULT 1.0,
            trim_start   REAL,
            trim_end     REAL,
            loop_start   REAL,
            loop_end     REAL,
            created_at   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tags (
            id   TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );

        CREATE TABLE IF NOT EXISTS library_entry_tags (
            library_entry_id TEXT NOT NULL REFERENCES library_entries(id) ON DELETE CASCADE,
            tag_id           TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
            PRIMARY KEY (library_entry_id, tag_id)
        );

        CREATE TABLE IF NOT EXISTS configurations (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            created_at TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS groups (
            id               TEXT PRIMARY KEY,
            name             TEXT NOT NULL,
            sort_order       INTEGER NOT NULL,
            configuration_id TEXT NOT NULL DEFAULT '' REFERENCES configurations(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS comps (
            id         TEXT PRIMARY KEY,
            group_id   TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tracks (
            id                 TEXT PRIMARY KEY,
            comp_id            TEXT NOT NULL REFERENCES comps(id) ON DELETE CASCADE,
            name               TEXT NOT NULL,
            volume             REAL NOT NULL DEFAULT 1.0,
            mode_type          TEXT NOT NULL DEFAULT 'sequence',
            repeat             INTEGER DEFAULT 0,
            transition_type    TEXT DEFAULT 'gapless',
            transition_gap_min REAL,
            transition_gap_max REAL,
            fade_in            REAL NOT NULL DEFAULT 0.0,
            fade_out           REAL NOT NULL DEFAULT 0.0,
            sort_order         INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS clips (
            id               TEXT PRIMARY KEY,
            track_id         TEXT NOT NULL REFERENCES tracks(id)          ON DELETE CASCADE,
            library_entry_id TEXT NOT NULL REFERENCES library_entries(id),
            volume           REAL NOT NULL DEFAULT 1.0,
            sort_order       INTEGER NOT NULL
        );

        INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '2');
    ")
}

/// Ensures at least one configuration exists and active_configuration_id is set.
/// Called after init_schema for fresh databases that skip the v1→v2 migration.
pub fn ensure_default_configuration(conn: &Connection) -> Result<(), String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM configurations", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count configurations: {e}"))?;

    if count > 0 {
        // Also ensure active_configuration_id is set
        let has_active: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key = 'active_configuration_id'",
                [],
                |row| row.get::<_, i64>(0).map(|c| c > 0),
            )
            .unwrap_or(false);
        if !has_active {
            let first_id: String = conn
                .query_row("SELECT id FROM configurations ORDER BY sort_order LIMIT 1", [], |row| row.get(0))
                .map_err(|e| format!("Failed to get first configuration: {e}"))?;
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
                params![first_id],
            ).map_err(|e| format!("Failed to set active configuration: {e}"))?;
        }
        return Ok(());
    }

    use std::time::{SystemTime, UNIX_EPOCH};
    let config_id = Uuid::new_v4().to_string();
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis()
        .to_string();

    conn.execute(
        "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Default', ?2, 1000)",
        params![config_id, created_at],
    ).map_err(|e| format!("Failed to insert default configuration: {e}"))?;

    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
        params![config_id],
    ).map_err(|e| format!("Failed to set active configuration: {e}"))?;

    Ok(())
}

/// Migrates from schema v1 to v2: creates the configurations table,
/// inserts a "Default" configuration, and adds configuration_id to groups.
pub fn migrate_v1_to_v2(conn: &Connection) -> Result<(), String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let version: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| row.get(0))
        .map_err(|e| format!("Failed to read schema_version: {e}"))?;

    if version != "1" {
        return Ok(()); // Already migrated or newer
    }

    let config_id = Uuid::new_v4().to_string();
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis()
        .to_string();

    // Disable foreign keys BEFORE the transaction — SQLite ignores
    // PRAGMA foreign_keys changes inside a transaction.  We must turn
    // them off so that DROP TABLE groups does not cascade-delete all
    // comps/tracks/clips via ON DELETE CASCADE.
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| format!("Failed to disable foreign keys: {e}"))?;

    conn.execute_batch("BEGIN TRANSACTION;")
        .map_err(|e| format!("Failed to begin migration transaction: {e}"))?;

    let result = (|| -> Result<(), String> {
        // 1. Create configurations table
        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS configurations (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL,
                created_at TEXT NOT NULL,
                sort_order INTEGER NOT NULL
            );
        ").map_err(|e| format!("Failed to create configurations table: {e}"))?;

        // 2. Insert Default configuration
        conn.execute(
            "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Default', ?2, 1000)",
            params![config_id, created_at],
        ).map_err(|e| format!("Failed to insert default configuration: {e}"))?;

        // 3. Recreate groups table with configuration_id FK
        conn.execute_batch("
            CREATE TABLE groups_new (
                id               TEXT PRIMARY KEY,
                name             TEXT NOT NULL,
                sort_order       INTEGER NOT NULL,
                configuration_id TEXT NOT NULL REFERENCES configurations(id) ON DELETE CASCADE
            );
        ").map_err(|e| format!("Failed to create groups_new: {e}"))?;

        conn.execute(
            "INSERT INTO groups_new (id, name, sort_order, configuration_id)
             SELECT id, name, sort_order, ?1 FROM groups",
            params![config_id],
        ).map_err(|e| format!("Failed to copy groups data: {e}"))?;

        conn.execute_batch("
            DROP TABLE groups;
            ALTER TABLE groups_new RENAME TO groups;
        ").map_err(|e| format!("Failed to swap groups table: {e}"))?;

        // 4. Set active configuration
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
            params![config_id],
        ).map_err(|e| format!("Failed to set active configuration: {e}"))?;

        // 5. Bump schema version
        conn.execute(
            "UPDATE meta SET value = '2' WHERE key = 'schema_version'",
            [],
        ).map_err(|e| format!("Failed to update schema version: {e}"))?;

        Ok(())
    })();

    match &result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .map_err(|e| format!("Failed to commit migration: {e}"))?;
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK;");
        }
    }

    // Re-enable foreign keys
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("Failed to re-enable foreign keys: {e}"))?;

    result
}

// ─── One-time Recovery from Backup JSON ──────────────────────────────────────

/// Imports data from a legacy backup JSON file into the current v2 database.
/// Only runs if the groups table is empty and the JSON file exists.
/// This restores data after a botched migration.
pub fn recover_from_backup_json(conn: &Connection, json_path: &std::path::Path) -> Result<(usize, usize), String> {
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use std::io::Read;

    if !json_path.exists() {
        return Ok((0, 0));
    }

    // Only run if DB is empty
    let group_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))
        .map_err(|e| format!("Failed to count groups: {e}"))?;
    if group_count > 0 {
        return Ok((0, 0));
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OldState { groups: Vec<OldGroup> }

    #[derive(serde::Deserialize)]
    struct OldGroup { id: String, name: String, comps: Vec<OldComp> }

    #[derive(serde::Deserialize)]
    struct OldComp { id: String, name: String, tracks: Vec<OldTrack> }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OldTrack {
        id: String, name: String, clips: Vec<OldClip>,
        volume: f64, mode: serde_json::Value, fade_in: f64, fade_out: f64,
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OldClip { id: String, name: String, file_path: String, volume: f64 }

    let raw = std::fs::read_to_string(json_path)
        .map_err(|e| format!("Failed to read backup JSON: {e}"))?;
    let old_state: OldState = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse backup JSON: {e}"))?;

    let config_id = get_active_configuration_id(conn)?;

    // Build file_path → entry_id map (dedup by hash)
    let mut path_to_entry: HashMap<String, String> = HashMap::new();
    let mut hash_to_entry: HashMap<String, String> = HashMap::new();
    let mut entries_created: usize = 0;

    for group in &old_state.groups {
        for comp in &group.comps {
            for track in &comp.tracks {
                for clip in &track.clips {
                    if path_to_entry.contains_key(&clip.file_path) { continue; }

                    let hash_opt = (|| -> Option<String> {
                        let mut file = std::fs::File::open(&clip.file_path).ok()?;
                        let mut hasher = Sha256::new();
                        let mut buf = [0u8; 65536];
                        loop {
                            match file.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => hasher.update(&buf[..n]),
                                Err(_) => return None,
                            }
                        }
                        Some(format!("{:x}", hasher.finalize()))
                    })();

                    let entry_id = if let Some(ref hash) = hash_opt {
                        if let Some(existing) = hash_to_entry.get(hash) {
                            existing.clone()
                        } else {
                            let id = library::insert_library_entry(
                                conn, &clip.file_path, Some(hash.as_str()), &clip.name, None, None, None,
                            )?;
                            hash_to_entry.insert(hash.clone(), id.clone());
                            entries_created += 1;
                            id
                        }
                    } else {
                        let id = library::insert_library_entry(
                            conn, &clip.file_path, None, &clip.name, None, None, None,
                        )?;
                        entries_created += 1;
                        id
                    };

                    // Tag from folder structure: .../Category/Subcategory/file.mp3
                    // Singularize and lowercase each folder component as a tag.
                    let normalized_path = clip.file_path.replace('\\', "/");
                    let path_parts: Vec<&str> = normalized_path
                        .split('/')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();
                    // Find "D&D Playlists" anchor and take folders after it (excluding filename)
                    if let Some(anchor) = path_parts.iter().position(|p| *p == "D&D Playlists") {
                        let tag_folders = &path_parts[anchor + 1..path_parts.len().saturating_sub(1)];
                        for folder in tag_folders {
                            let tag = folder.to_lowercase();
                            // Simple depluralization: strip trailing 's' for common cases
                            let tag = if tag.ends_with('s') && !tag.ends_with("ss") && tag.len() > 2 {
                                tag[..tag.len() - 1].to_string()
                            } else {
                                tag
                            };
                            let _ = library::add_tag_to_entry(conn, &entry_id, &tag);
                        }
                    }

                    path_to_entry.insert(clip.file_path.clone(), entry_id);
                }
            }
        }
    }

    // Recreate hierarchy
    let groups_count = old_state.groups.len();
    for (g_idx, group) in old_state.groups.iter().enumerate() {
        let g_sort = ((g_idx + 1) * 1000) as i64;
        conn.execute(
            "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES (?1, ?2, ?3, ?4)",
            params![group.id, group.name, g_sort, config_id],
        ).map_err(|e| format!("Failed to insert group: {e}"))?;

        for (c_idx, comp) in group.comps.iter().enumerate() {
            let c_sort = ((c_idx + 1) * 1000) as i64;
            conn.execute(
                "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
                params![comp.id, group.id, comp.name, c_sort],
            ).map_err(|e| format!("Failed to insert comp: {e}"))?;

            for (t_idx, track) in comp.tracks.iter().enumerate() {
                let t_sort = ((t_idx + 1) * 1000) as i64;
                let mode_type = track.mode.get("type").and_then(|v| v.as_str()).unwrap_or("sequence");
                let repeat = track.mode.get("repeat").and_then(|v| v.as_bool()).map(|b| if b { 1i64 } else { 0 });
                let transition = track.mode.get("transition");
                let transition_type = transition.and_then(|t| t.get("type")).and_then(|v| v.as_str());
                let gap_min = transition.and_then(|t| t.get("min")).and_then(|v| v.as_f64());
                let gap_max = transition.and_then(|t| t.get("max")).and_then(|v| v.as_f64());

                conn.execute(
                    "INSERT INTO tracks (id, comp_id, name, volume, mode_type, repeat,
                         transition_type, transition_gap_min, transition_gap_max,
                         fade_in, fade_out, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        track.id, comp.id, track.name, track.volume, mode_type,
                        repeat, transition_type, gap_min, gap_max,
                        track.fade_in, track.fade_out, t_sort,
                    ],
                ).map_err(|e| format!("Failed to insert track: {e}"))?;

                // For seamless tracks, store loop points on the library entry
                if mode_type == "seamless" {
                    if let Some(clip) = track.clips.first() {
                        if let Some(entry_id) = path_to_entry.get(&clip.file_path) {
                            let loop_start = track.mode.get("loopStart").and_then(|v| v.as_f64());
                            let loop_end = track.mode.get("loopEnd").and_then(|v| v.as_f64());
                            if loop_start.is_some() || loop_end.is_some() {
                                let _ = library::update_library_entry_markers(conn, entry_id, None, None, loop_start, loop_end);
                            }
                        }
                    }
                }

                for (cl_idx, clip) in track.clips.iter().enumerate() {
                    let cl_sort = ((cl_idx + 1) * 1000) as i64;
                    let entry_id = path_to_entry.get(&clip.file_path)
                        .ok_or_else(|| format!("No library entry for '{}'", clip.file_path))?;
                    conn.execute(
                        "INSERT INTO clips (id, track_id, library_entry_id, volume, sort_order)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![clip.id, track.id, entry_id, clip.volume, cl_sort],
                    ).map_err(|e| format!("Failed to insert clip: {e}"))?;
                }
            }
        }
    }

    Ok((groups_count, entries_created))
}

// ─── State Loading ────────────────────────────────────────────────────────────

/// Reads the active configuration ID from the meta table.
pub fn get_active_configuration_id(conn: &Connection) -> Result<String, String> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'active_configuration_id'",
        [],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to read active_configuration_id: {e}"))
}

/// Loads all configurations ordered by sort_order.
fn load_configurations(conn: &Connection) -> Result<Vec<Configuration>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name, created_at FROM configurations ORDER BY sort_order ASC")
        .map_err(|e| format!("Failed to prepare load_configurations: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Configuration {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
            })
        })
        .map_err(|e| format!("Failed to query configurations: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect configurations: {e}"))?;

    Ok(rows)
}

/// Builds the complete nested AppState from SQLite.
/// Called after every mutation to return the updated state to the frontend.
pub fn load_full_state(conn: &Connection) -> Result<AppState, String> {
    let active_configuration_id = get_active_configuration_id(conn)?;
    let configurations = load_configurations(conn)?;
    let library_entries = load_library_entries(conn)?;
    let tags = library::list_all_tags(conn)?;

    // Build id → entry reference map for lookups in hierarchy loading.
    let entry_map: HashMap<String, &LibraryEntry> = library_entries
        .iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    let groups = load_groups(conn, &active_configuration_id, &entry_map)?;

    Ok(AppState {
        groups,
        library_entries,
        tags,
        configurations,
        active_configuration_id,
    })
}

/// Queries all library_entries ordered by name and fills in tags for each.
fn load_library_entries(conn: &Connection) -> Result<Vec<LibraryEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, file_path, content_hash, name, duration, sample_rate, file_size,
                    volume, trim_start, trim_end, loop_start, loop_end, created_at
             FROM library_entries
             ORDER BY name ASC",
        )
        .map_err(|e| format!("Failed to prepare load_library_entries: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,           // id
                row.get::<_, String>(1)?,           // file_path
                row.get::<_, Option<String>>(2)?,   // content_hash
                row.get::<_, String>(3)?,           // name
                row.get::<_, Option<f64>>(4)?,      // duration
                row.get::<_, Option<i64>>(5)?,      // sample_rate (INTEGER → i64)
                row.get::<_, Option<i64>>(6)?,      // file_size   (INTEGER → i64)
                row.get::<_, f64>(7)?,              // volume
                row.get::<_, Option<f64>>(8)?,      // trim_start
                row.get::<_, Option<f64>>(9)?,      // trim_end
                row.get::<_, Option<f64>>(10)?,     // loop_start
                row.get::<_, Option<f64>>(11)?,     // loop_end
                row.get::<_, String>(12)?,          // created_at
            ))
        })
        .map_err(|e| format!("Failed to query library_entries: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect library_entries: {e}"))?;

    let mut entries = Vec::with_capacity(rows.len());
    for (id, file_path, content_hash, name, duration, sample_rate, file_size,
         volume, trim_start, trim_end, loop_start, loop_end, created_at) in rows
    {
        let tags = library::get_tags_for_entry(conn, &id)?;
        entries.push(LibraryEntry {
            id,
            file_path,
            content_hash,
            name,
            duration,
            sample_rate: sample_rate.map(|v| v as u32),
            file_size: file_size.map(|v| v as u64),
            volume,
            trim_start,
            trim_end,
            loop_start,
            loop_end,
            created_at,
            tags,
        });
    }

    Ok(entries)
}

/// Queries all groups for the given configuration_id ordered by sort_order and loads comps for each.
fn load_groups(
    conn: &Connection,
    configuration_id: &str,
    entry_map: &HashMap<String, &LibraryEntry>,
) -> Result<Vec<Group>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM groups WHERE configuration_id = ?1 ORDER BY sort_order ASC")
        .map_err(|e| format!("Failed to prepare load_groups: {e}"))?;

    let rows = stmt
        .query_map(params![configuration_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Failed to query groups: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect groups: {e}"))?;

    let mut groups = Vec::with_capacity(rows.len());
    for (id, name) in rows {
        let comps = load_comps(conn, &id, entry_map)?;
        groups.push(Group { id, name, comps });
    }

    Ok(groups)
}

/// Queries comps for a given group_id ordered by sort_order and loads tracks for each.
fn load_comps(
    conn: &Connection,
    group_id: &str,
    entry_map: &HashMap<String, &LibraryEntry>,
) -> Result<Vec<Comp>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name FROM comps WHERE group_id = ?1 ORDER BY sort_order ASC",
        )
        .map_err(|e| format!("Failed to prepare load_comps: {e}"))?;

    let rows = stmt
        .query_map(params![group_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Failed to query comps: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect comps: {e}"))?;

    let mut comps = Vec::with_capacity(rows.len());
    for (id, name) in rows {
        let tracks = load_tracks(conn, &id, entry_map)?;
        comps.push(Comp { id, name, tracks });
    }

    Ok(comps)
}

/// Queries tracks for a given comp_id ordered by sort_order and builds each Track.
fn load_tracks(
    conn: &Connection,
    comp_id: &str,
    entry_map: &HashMap<String, &LibraryEntry>,
) -> Result<Vec<Track>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, volume, mode_type, repeat, transition_type,
                    transition_gap_min, transition_gap_max, fade_in, fade_out
             FROM tracks
             WHERE comp_id = ?1
             ORDER BY sort_order ASC",
        )
        .map_err(|e| format!("Failed to prepare load_tracks: {e}"))?;

    let rows = stmt
        .query_map(params![comp_id], |row| {
            Ok((
                row.get::<_, String>(0)?,           // id
                row.get::<_, String>(1)?,           // name
                row.get::<_, f64>(2)?,              // volume
                row.get::<_, String>(3)?,           // mode_type
                row.get::<_, Option<i64>>(4)?,      // repeat (stored as INTEGER)
                row.get::<_, Option<String>>(5)?,   // transition_type
                row.get::<_, Option<f64>>(6)?,      // gap_min
                row.get::<_, Option<f64>>(7)?,      // gap_max
                row.get::<_, f64>(8)?,              // fade_in
                row.get::<_, f64>(9)?,              // fade_out
            ))
        })
        .map_err(|e| format!("Failed to query tracks: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect tracks: {e}"))?;

    let mut tracks = Vec::with_capacity(rows.len());
    for (id, name, volume, mode_type, repeat_int, transition_type, gap_min, gap_max,
         fade_in, fade_out) in rows
    {
        let clips = load_clips(conn, &id, entry_map)?;
        let repeat = repeat_int.map(|v| v != 0).unwrap_or(false);
        let transition_type_str = transition_type.as_deref().unwrap_or("gapless");
        let mode = build_track_mode(
            &mode_type,
            repeat,
            transition_type_str,
            gap_min.unwrap_or(0.0),
            gap_max.unwrap_or(0.0),
            &clips,
            entry_map,
        );
        tracks.push(Track {
            id,
            name,
            clips,
            volume,
            mode,
            fade_in,
            fade_out,
        });
    }

    Ok(tracks)
}

/// Queries clips for a given track_id ordered by sort_order.
/// Resolves name, file_path, and library_volume from entry_map.
fn load_clips(
    conn: &Connection,
    track_id: &str,
    entry_map: &HashMap<String, &LibraryEntry>,
) -> Result<Vec<Clip>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, library_entry_id, volume
             FROM clips
             WHERE track_id = ?1
             ORDER BY sort_order ASC",
        )
        .map_err(|e| format!("Failed to prepare load_clips: {e}"))?;

    let rows = stmt
        .query_map(params![track_id], |row| {
            Ok((
                row.get::<_, String>(0)?,   // id
                row.get::<_, String>(1)?,   // library_entry_id
                row.get::<_, f64>(2)?,      // volume
            ))
        })
        .map_err(|e| format!("Failed to query clips: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect clips: {e}"))?;

    let clips = rows
        .into_iter()
        .map(|(id, library_entry_id, volume)| {
            let (name, file_path, library_volume, trim_start, trim_end) = match entry_map.get(&library_entry_id) {
                Some(entry) => (entry.name.clone(), entry.file_path.clone(), entry.volume, entry.trim_start, entry.trim_end),
                None => ("[missing]".to_string(), String::new(), 1.0, None, None),
            };
            Clip {
                id,
                name,
                file_path,
                volume,
                library_entry_id,
                library_volume,
                trim_start,
                trim_end,
            }
        })
        .collect();

    Ok(clips)
}

/// Builds a TrackMode from the raw DB columns plus the already-loaded clips.
fn build_track_mode(
    mode_type: &str,
    repeat: bool,
    transition_type: &str,
    gap_min: f64,
    gap_max: f64,
    clips: &[Clip],
    entry_map: &HashMap<String, &LibraryEntry>,
) -> TrackMode {
    match mode_type {
        "seamless" => {
            let (loop_start, loop_end) = clips
                .first()
                .and_then(|clip| entry_map.get(&clip.library_entry_id))
                .and_then(|entry| {
                    if let (Some(ls), Some(le)) = (entry.loop_start, entry.loop_end) {
                        Some((ls, le))
                    } else {
                        None
                    }
                })
                .unwrap_or((0.0, 0.0));
            TrackMode::Seamless { loop_start, loop_end }
        }
        "shuffle" => TrackMode::Shuffle {
            repeat,
            transition: build_transition(transition_type, gap_min, gap_max),
        },
        _ => TrackMode::Sequence {
            repeat,
            transition: build_transition(transition_type, gap_min, gap_max),
        },
    }
}

/// Builds a TrackTransition from raw DB column values.
fn build_transition(transition_type: &str, gap_min: f64, gap_max: f64) -> TrackTransition {
    match transition_type {
        "gap" => TrackTransition::Gap { min: gap_min, max: gap_max },
        "crossfade" => TrackTransition::Crossfade,
        _ => TrackTransition::Gapless,
    }
}

// ─────────────────────────────────────────────────────────────────────────────

/// Opens an in-memory SQLite database with the full schema applied.
/// Intended for unit tests only.
#[cfg(test)]
pub fn open_memory_db() -> Connection {
    let conn = Connection::open_in_memory().expect("failed to open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .expect("failed to enable foreign keys");
    init_schema(&conn).expect("failed to init schema");
    conn
}

#[cfg(test)]
pub fn ensure_test_config(conn: &Connection) -> String {
    let config_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Test', '0', 1000)",
        params![config_id],
    ).expect("insert test config");
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
        params![config_id],
    ).expect("set active config");
    config_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_creates_all_tables() {
        let conn = open_memory_db();

        let expected_tables = [
            "meta",
            "library_entries",
            "tags",
            "library_entry_tags",
            "configurations",
            "groups",
            "comps",
            "tracks",
            "clips",
        ];

        for table in &expected_tables {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .expect("query failed");
            assert_eq!(count, 1, "table '{}' should exist", table);
        }
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = open_memory_db();
        // Running init_schema a second time must not return an error.
        init_schema(&conn).expect("second call to init_schema should succeed");
    }

    #[test]
    fn schema_version_is_set() {
        let conn = open_memory_db();
        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema_version row should exist");
        assert_eq!(version, "2");
    }

    #[test]
    fn load_full_state_empty_db() {
        let conn = open_memory_db();
        ensure_test_config(&conn);
        let state = load_full_state(&conn).expect("load_full_state should succeed on empty DB");
        assert!(state.groups.is_empty(), "groups should be empty");
        assert!(state.library_entries.is_empty(), "library_entries should be empty");
        assert!(state.tags.is_empty(), "tags should be empty");
        assert_eq!(state.configurations.len(), 1);
    }

    #[test]
    fn load_full_state_with_data() {
        let conn = open_memory_db();

        // Insert a library entry with loop points and tags.
        let entry_id = library::insert_library_entry(
            &conn,
            "/tmp/theme.mp3",
            Some("hash-abc"),
            "Main Theme",
            Some(180.0),
            Some(44100),
            Some(2048),
        )
        .expect("insert library entry");

        // Set loop points on the entry.
        library::update_library_entry_loop(&conn, &entry_id, Some(1.5), Some(30.0))
            .expect("update loop");

        // Add tags.
        library::add_tag_to_entry(&conn, &entry_id, "ambient").expect("add ambient");
        library::add_tag_to_entry(&conn, &entry_id, "boss").expect("add boss");

        // Build hierarchy: group → comp → track → clip.
        let config_id = ensure_test_config(&conn);
        let group_id = hierarchy::insert_group(&conn, "Combat", &config_id).expect("insert group");
        let comp_id = hierarchy::insert_comp(&conn, &group_id, "Act 1").expect("insert comp");
        let track_id = hierarchy::insert_track(&conn, &comp_id, "Battle Theme").expect("insert track");
        let _clip_id = hierarchy::insert_clip(&conn, &track_id, &entry_id).expect("insert clip");

        // Set track to seamless mode so loop points are resolved.
        hierarchy::update_track_mode(&conn, &track_id, "seamless", None, None, None, None)
            .expect("set seamless mode");

        // Load full state.
        let state = load_full_state(&conn).expect("load_full_state should succeed");

        // Verify library entries.
        assert_eq!(state.library_entries.len(), 1);
        let entry = &state.library_entries[0];
        assert_eq!(entry.name, "Main Theme");
        assert_eq!(entry.tags, vec!["ambient", "boss"]);
        assert_eq!(entry.loop_start, Some(1.5));
        assert_eq!(entry.loop_end, Some(30.0));

        // Verify all-tags list.
        assert_eq!(state.tags, vec!["ambient", "boss"]);

        // Verify group hierarchy.
        assert_eq!(state.groups.len(), 1);
        let group = &state.groups[0];
        assert_eq!(group.name, "Combat");
        assert_eq!(group.comps.len(), 1, "group should have one comp");

        let comp = &group.comps[0];
        assert_eq!(comp.name, "Act 1");
        assert_eq!(comp.tracks.len(), 1, "comp should have one track");

        let track = &comp.tracks[0];
        assert_eq!(track.name, "Battle Theme");
        assert_eq!(track.clips.len(), 1, "track should have one clip");

        // Verify clip resolution.
        let clip = &track.clips[0];
        assert_eq!(clip.name, "Main Theme");
        assert_eq!(clip.file_path, "/tmp/theme.mp3");
        assert_eq!(clip.library_entry_id, entry_id);

        // Verify seamless mode resolved loop points from library entry.
        match &track.mode {
            crate::models::TrackMode::Seamless { loop_start, loop_end } => {
                assert!(
                    (loop_start - 1.5).abs() < 1e-9,
                    "loop_start should be 1.5, got {loop_start}"
                );
                assert!(
                    (loop_end - 30.0).abs() < 1e-9,
                    "loop_end should be 30.0, got {loop_end}"
                );
            }
            other => panic!("expected Seamless mode, got {other:?}"),
        }
    }

    #[test]
    fn load_full_state_scoped_by_config() {
        let conn = open_memory_db();
        // Reset to v1 so migrate_v1_to_v2 actually runs and seeds the default config.
        conn.execute("UPDATE meta SET value = '1' WHERE key = 'schema_version'", [])
            .expect("set v1");
        conn.execute_batch("PRAGMA foreign_keys=OFF;").expect("disable fk");
        conn.execute_batch("PRAGMA foreign_keys=ON;").expect("re-enable fk");
        migrate_v1_to_v2(&conn).expect("migrate");

        let config_id: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'active_configuration_id'",
                [],
                |row| row.get(0),
            )
            .expect("get active config");

        let config2_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Other', '0', 2000)",
            params![config2_id],
        ).expect("insert config2");

        conn.execute(
            "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES ('g1', 'Default Group', 1000, ?1)",
            params![config_id],
        ).expect("insert group in default config");

        conn.execute(
            "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES ('g2', 'Other Group', 1000, ?1)",
            params![config2_id],
        ).expect("insert group in other config");

        let state = load_full_state(&conn).expect("load state");
        assert_eq!(state.groups.len(), 1, "should only see active config's groups");
        assert_eq!(state.groups[0].name, "Default Group");
        assert_eq!(state.active_configuration_id, config_id);
        assert_eq!(state.configurations.len(), 2, "should see all configurations");
    }

    #[test]
    fn migrate_v1_to_v2_creates_default_config() {
        // Create a v1 database (current schema)
        let conn = open_memory_db();

        // Reset to v1 schema version for migration testing
        conn.execute("UPDATE meta SET value = '1' WHERE key = 'schema_version'", []).expect("set v1");

        // Insert some v1 data — a group with no configuration_id.
        // Disable FK enforcement temporarily so the empty DEFAULT '' is allowed.
        conn.execute_batch("PRAGMA foreign_keys=OFF;").expect("disable fk");
        conn.execute(
            "INSERT INTO groups (id, name, sort_order) VALUES ('g1', 'Combat', 1000)",
            [],
        ).expect("insert v1 group");
        conn.execute_batch("PRAGMA foreign_keys=ON;").expect("re-enable fk");

        // Run migration
        migrate_v1_to_v2(&conn).expect("migration should succeed");

        // Verify configurations table has one row
        let config_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM configurations", [], |row| row.get(0))
            .expect("count configs");
        assert_eq!(config_count, 1, "should have one default configuration");

        // Verify the default config name
        let config_name: String = conn
            .query_row("SELECT name FROM configurations LIMIT 1", [], |row| row.get(0))
            .expect("get config name");
        assert_eq!(config_name, "Default");

        // Verify the group now has configuration_id set
        let group_config_id: String = conn
            .query_row(
                "SELECT configuration_id FROM groups WHERE id = 'g1'",
                [],
                |row| row.get(0),
            )
            .expect("get group config_id");

        let config_id: String = conn
            .query_row("SELECT id FROM configurations LIMIT 1", [], |row| row.get(0))
            .expect("get config id");
        assert_eq!(group_config_id, config_id, "group should reference the default config");

        // Verify active_configuration_id is set in meta
        let active_id: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'active_configuration_id'",
                [],
                |row| row.get(0),
            )
            .expect("get active config id");
        assert_eq!(active_id, config_id);

        // Verify schema version bumped to 2
        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("get schema version");
        assert_eq!(version, "2");
    }
}
