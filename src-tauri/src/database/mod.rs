pub mod hierarchy;
pub mod library;

use rusqlite::{params, Connection, Result as SqlResult};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

use crate::models::{
    AppState, Clip, Comp, Group, LibraryEntry, Track, TrackMode, TrackTransition,
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

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")
                .map_err(|e| format!("Failed to commit migration: {e}"))?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(e)
        }
    }
}

// ─── State Loading ────────────────────────────────────────────────────────────

/// Builds the complete nested AppState from SQLite.
/// Called after every mutation to return the updated state to the frontend.
pub fn load_full_state(conn: &Connection) -> Result<AppState, String> {
    let library_entries = load_library_entries(conn)?;
    let tags = library::list_all_tags(conn)?;

    // Build id → entry reference map for lookups in hierarchy loading.
    let entry_map: HashMap<String, &LibraryEntry> = library_entries
        .iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    let groups = load_groups(conn, &entry_map)?;

    Ok(AppState {
        groups,
        library_entries,
        tags,
        configurations: Vec::new(),      // TODO Task 3: load from DB
        active_configuration_id: String::new(), // TODO Task 3: load from meta
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

/// Queries all groups ordered by sort_order and loads comps for each.
fn load_groups(
    conn: &Connection,
    entry_map: &HashMap<String, &LibraryEntry>,
) -> Result<Vec<Group>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM groups ORDER BY sort_order ASC")
        .map_err(|e| format!("Failed to prepare load_groups: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
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
        let state = load_full_state(&conn).expect("load_full_state should succeed on empty DB");
        assert!(state.groups.is_empty(), "groups should be empty");
        assert!(state.library_entries.is_empty(), "library_entries should be empty");
        assert!(state.tags.is_empty(), "tags should be empty");
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
        let group_id = hierarchy::insert_group(&conn, "Combat").expect("insert group");
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
