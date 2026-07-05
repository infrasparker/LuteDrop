pub mod backup;
pub mod configurations;
pub mod hierarchy;
pub mod library;

use rusqlite::{params, Connection, Result as SqlResult};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

use crate::models::{
    AppState, Clip, Comp, Configuration, EntryType, Group, LibraryEntry, Track, TrackMode, TrackTransition,
};

/// Current schema version. Bump when adding a migration below; fresh
/// databases are stamped with this directly.
pub const SCHEMA_VERSION: i64 = 4;

/// Index DDL shared by init_schema (fresh DBs) and the v3→v4 migration
/// (existing DBs). IF NOT EXISTS makes running both harmless.
const INDEX_DDL: &str = "
    CREATE INDEX IF NOT EXISTS idx_groups_configuration_id     ON groups(configuration_id);
    CREATE INDEX IF NOT EXISTS idx_comps_group_id              ON comps(group_id);
    CREATE INDEX IF NOT EXISTS idx_tracks_comp_id              ON tracks(comp_id);
    CREATE INDEX IF NOT EXISTS idx_clips_track_id              ON clips(track_id);
    CREATE INDEX IF NOT EXISTS idx_clips_library_entry_id      ON clips(library_entry_id);
    CREATE INDEX IF NOT EXISTS idx_library_entry_tags_tag_id   ON library_entry_tags(tag_id);
    CREATE INDEX IF NOT EXISTS idx_library_entries_content_hash ON library_entries(content_hash);
";

/// A migration upgrades a database from version N to N+1.
pub type Migration = fn(&Connection) -> SqlResult<()>;

fn migrate_v3_to_v4(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(INDEX_DDL)
}

/// Ordered migrations: entry `(n, f)` upgrades version n → n+1.
const MIGRATIONS: &[(i64, Migration)] = &[
    (3, migrate_v3_to_v4),
];

fn read_schema_version(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(0)
}

pub fn run_migrations(conn: &Connection) -> Result<(), String> {
    run_migrations_with(conn, MIGRATIONS, SCHEMA_VERSION)
}

fn run_migrations_with(
    conn: &Connection,
    migrations: &[(i64, Migration)],
    target: i64,
) -> Result<(), String> {
    let mut version = read_schema_version(conn);
    if version > target {
        return Err(format!(
            "Database schema is v{version}, newer than this app supports (v{target}). \
             Update LuteDrop before opening this library."
        ));
    }
    for (from, migrate) in migrations {
        if version == *from {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Migration v{from}: failed to open transaction: {e}"))?;
            migrate(conn).map_err(|e| format!("Migration v{from} → v{} failed: {e}", from + 1))?;
            conn.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                params![(from + 1).to_string()],
            )
            .map_err(|e| format!("Migration v{from}: failed to bump version: {e}"))?;
            tx.commit()
                .map_err(|e| format!("Migration v{from}: commit failed: {e}"))?;
            version = from + 1;
        }
    }
    Ok(())
}

/// Opens (or creates) the SQLite database at `path`, enables WAL mode and
/// foreign-key enforcement, then runs the schema initialiser.
pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("Failed to open database: {e}"))?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("Failed to set PRAGMAs: {e}"))?;

    // A database without a `meta` table is fresh: init_schema creates the
    // current schema, so stamp it SCHEMA_VERSION directly. An existing
    // database keeps (or is backfilled to) its stored version and is walked
    // forward by run_migrations. All pre-runner databases are v3.
    let fresh: bool = !conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meta'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .map_err(|e| format!("Failed to inspect database: {e}"))?;

    init_schema(&conn).map_err(|e| format!("Failed to initialise schema: {e}"))?;

    let stamp = if fresh { SCHEMA_VERSION } else { 3 };
    conn.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', ?1)",
        params![stamp.to_string()],
    )
    .map_err(|e| format!("Failed to stamp schema version: {e}"))?;

    run_migrations(&conn)?;

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
            created_at   TEXT NOT NULL,
            entry_type   TEXT NOT NULL DEFAULT 'music'
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
    ")?;
    conn.execute_batch(INDEX_DDL)
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
        missing_entry_ids: Vec::new(),
    })
}

/// Queries all library_entries ordered by name and fills in tags for each.
fn load_library_entries(conn: &Connection) -> Result<Vec<LibraryEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, file_path, content_hash, name, duration, sample_rate, file_size,
                    volume, trim_start, trim_end, loop_start, loop_end, created_at, entry_type
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
                row.get::<_, String>(13)?,          // entry_type
            ))
        })
        .map_err(|e| format!("Failed to query library_entries: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect library_entries: {e}"))?;

    // All entry→tag associations in one query, grouped in memory.
    let mut tag_map: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut tstmt = conn
            .prepare(
                "SELECT let.library_entry_id, t.name
                 FROM library_entry_tags let
                 JOIN tags t ON t.id = let.tag_id
                 ORDER BY t.name ASC",
            )
            .map_err(|e| format!("Failed to prepare tag map query: {e}"))?;
        let pairs = tstmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| format!("Failed to query tag map: {e}"))?;
        for pair in pairs {
            let (entry_id, name) = pair.map_err(|e| format!("Failed to read tag row: {e}"))?;
            tag_map.entry(entry_id).or_default().push(name);
        }
    }

    let mut entries = Vec::with_capacity(rows.len());
    for (id, file_path, content_hash, name, duration, sample_rate, file_size,
         volume, trim_start, trim_end, loop_start, loop_end, created_at, entry_type) in rows
    {
        let tags = tag_map.remove(&id).unwrap_or_default();
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
            entry_type: EntryType::from_str(&entry_type),
        });
    }

    Ok(entries)
}

/// Queries all groups/comps/tracks/clips for the given configuration_id in
/// four flat queries (one per level) instead of a per-parent N+1 cascade,
/// then assembles the nested hierarchy in memory.
fn load_groups(
    conn: &Connection,
    configuration_id: &str,
    entry_map: &HashMap<String, &LibraryEntry>,
) -> Result<Vec<Group>, String> {
    // One query per level, each scoped to the configuration and globally
    // ordered by sort_order — pushing rows into per-parent vecs in query
    // order preserves the per-parent ordering.
    let group_rows: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, name FROM groups WHERE configuration_id = ?1 ORDER BY sort_order ASC")
            .map_err(|e| format!("Failed to prepare groups query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("Failed to query groups: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect groups: {e}"))?;
        rows
    };

    // comps grouped by group_id
    let mut comps_by_group: HashMap<String, Vec<(String, String)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.group_id, c.name
                 FROM comps c
                 JOIN groups g ON g.id = c.group_id
                 WHERE g.configuration_id = ?1
                 ORDER BY c.sort_order ASC",
            )
            .map_err(|e| format!("Failed to prepare comps query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(|e| format!("Failed to query comps: {e}"))?;
        for row in rows {
            let (id, group_id, name) = row.map_err(|e| format!("Failed to read comp row: {e}"))?;
            comps_by_group.entry(group_id).or_default().push((id, name));
        }
    }

    // tracks grouped by comp_id (same tuple shape the old load_tracks used)
    type TrackRow = (String, String, f64, String, Option<i64>, Option<String>, Option<f64>, Option<f64>, f64, f64);
    let mut tracks_by_comp: HashMap<String, Vec<TrackRow>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.comp_id, t.name, t.volume, t.mode_type, t.repeat, t.transition_type,
                        t.transition_gap_min, t.transition_gap_max, t.fade_in, t.fade_out
                 FROM tracks t
                 JOIN comps c ON c.id = t.comp_id
                 JOIN groups g ON g.id = c.group_id
                 WHERE g.configuration_id = ?1
                 ORDER BY t.sort_order ASC",
            )
            .map_err(|e| format!("Failed to prepare tracks query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?, row.get::<_, Option<f64>>(7)?,
                    row.get::<_, Option<f64>>(8)?, row.get::<_, f64>(9)?, row.get::<_, f64>(10)?,
                ))
            })
            .map_err(|e| format!("Failed to query tracks: {e}"))?;
        for row in rows {
            let (id, comp_id, name, volume, mode_type, repeat, ttype, gmin, gmax, fin, fout) =
                row.map_err(|e| format!("Failed to read track row: {e}"))?;
            tracks_by_comp
                .entry(comp_id)
                .or_default()
                .push((id, name, volume, mode_type, repeat, ttype, gmin, gmax, fin, fout));
        }
    }

    // clips grouped by track_id
    let mut clips_by_track: HashMap<String, Vec<(String, String, f64)>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT cl.id, cl.track_id, cl.library_entry_id, cl.volume
                 FROM clips cl
                 JOIN tracks t ON t.id = cl.track_id
                 JOIN comps c ON c.id = t.comp_id
                 JOIN groups g ON g.id = c.group_id
                 WHERE g.configuration_id = ?1
                 ORDER BY cl.sort_order ASC",
            )
            .map_err(|e| format!("Failed to prepare clips query: {e}"))?;
        let rows = stmt
            .query_map(params![configuration_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, f64>(3)?))
            })
            .map_err(|e| format!("Failed to query clips: {e}"))?;
        for row in rows {
            let (id, track_id, library_entry_id, volume) =
                row.map_err(|e| format!("Failed to read clip row: {e}"))?;
            clips_by_track.entry(track_id).or_default().push((id, library_entry_id, volume));
        }
    }

    // Assemble, resolving clip fields from entry_map exactly as before.
    let mut groups = Vec::with_capacity(group_rows.len());
    for (group_id, group_name) in group_rows {
        let comp_rows = comps_by_group.remove(&group_id).unwrap_or_default();
        let mut comps = Vec::with_capacity(comp_rows.len());
        for (comp_id, comp_name) in comp_rows {
            let track_rows = tracks_by_comp.remove(&comp_id).unwrap_or_default();
            let mut tracks = Vec::with_capacity(track_rows.len());
            for (track_id, name, volume, mode_type, repeat_int, ttype, gmin, gmax, fade_in, fade_out) in track_rows {
                let clip_rows = clips_by_track.remove(&track_id).unwrap_or_default();
                let clips: Vec<Clip> = clip_rows
                    .into_iter()
                    .map(|(id, library_entry_id, volume)| {
                        let (name, file_path, library_volume, trim_start, trim_end) =
                            match entry_map.get(&library_entry_id) {
                                Some(entry) => (entry.name.clone(), entry.file_path.clone(), entry.volume, entry.trim_start, entry.trim_end),
                                None => ("[missing]".to_string(), String::new(), 1.0, None, None),
                            };
                        Clip { id, name, file_path, volume, library_entry_id, library_volume, trim_start, trim_end }
                    })
                    .collect();
                let repeat = repeat_int.map(|v| v != 0).unwrap_or(false);
                let ttype_str = ttype.as_deref().unwrap_or("gapless");
                let mode = build_track_mode(&mode_type, repeat, ttype_str, gmin.unwrap_or(0.0), gmax.unwrap_or(0.0), &clips, entry_map);
                tracks.push(Track { id: track_id, name, clips, volume, mode, fade_in, fade_out });
            }
            comps.push(Comp { id: comp_id, name: comp_name, tracks });
        }
        groups.push(Group { id: group_id, name: group_name, comps });
    }
    Ok(groups)
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
        // Loop points are SNAPSHOTTED from the first clip's library entry at
        // load time — editing the entry's markers changes track behavior only
        // after the next state reload (deliberate; the engine never re-reads
        // the DB mid-playback).
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

// ─── Targeted Play-Path Lookups ───────────────────────────────────────────────

/// Row shape shared by the two targeted loaders: a Clip plus the referenced
/// entry's loop markers (needed only for Seamless mode's first clip).
type ClipWithLoops = (Clip, Option<f64>, Option<f64>);

/// `where_sql` is a `&'static str` chosen from exactly two hardcoded literals
/// at the only two call sites below — no user data reaches `format!`.
fn load_clips_resolved(
    conn: &Connection,
    where_sql: &'static str, // "cl.track_id = ?1" | "cl.track_id IN (SELECT id FROM tracks WHERE comp_id = ?1)"
    param: &str,
) -> Result<Vec<(String, ClipWithLoops)>, String> {
    let sql = format!(
        "SELECT cl.id, cl.track_id, cl.library_entry_id, cl.volume,
                le.name, le.file_path, le.volume, le.trim_start, le.trim_end,
                le.loop_start, le.loop_end
         FROM clips cl
         LEFT JOIN library_entries le ON le.id = cl.library_entry_id
         WHERE {where_sql}
         ORDER BY cl.sort_order ASC"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Failed to prepare resolved clips query: {e}"))?;
    let rows = stmt
        .query_map(params![param], |row| {
            Ok((
                row.get::<_, String>(0)?,          // clip id
                row.get::<_, String>(1)?,          // track_id
                row.get::<_, String>(2)?,          // library_entry_id
                row.get::<_, f64>(3)?,             // clip volume
                row.get::<_, Option<String>>(4)?,  // entry name (None when entry missing)
                row.get::<_, Option<String>>(5)?,  // file_path
                row.get::<_, Option<f64>>(6)?,     // library volume
                row.get::<_, Option<f64>>(7)?,     // trim_start
                row.get::<_, Option<f64>>(8)?,     // trim_end
                row.get::<_, Option<f64>>(9)?,     // loop_start
                row.get::<_, Option<f64>>(10)?,    // loop_end
            ))
        })
        .map_err(|e| format!("Failed to query resolved clips: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect resolved clips: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|(id, track_id, library_entry_id, volume, name, file_path, lib_vol, ts, te, ls, le)| {
            let clip = Clip {
                id,
                name: name.unwrap_or_else(|| "[missing]".to_string()),
                file_path: file_path.unwrap_or_default(),
                volume,
                library_entry_id,
                library_volume: lib_vol.unwrap_or(1.0),
                trim_start: ts,
                trim_end: te,
            };
            (track_id, (clip, ls, le))
        })
        .collect())
}

/// Builds a Track from a raw track row plus its resolved clips.
/// Mirrors `build_track_mode`, but takes the first clip's loop markers
/// directly (already joined) instead of an entry_map.
fn assemble_track(
    row: (String, String, f64, String, Option<i64>, Option<String>, Option<f64>, Option<f64>, f64, f64),
    clips_with_loops: Vec<ClipWithLoops>,
) -> Track {
    let (id, name, volume, mode_type, repeat_int, ttype, gmin, gmax, fade_in, fade_out) = row;
    let first_loops = clips_with_loops.first().map(|(_, ls, le)| (*ls, *le));
    let clips: Vec<Clip> = clips_with_loops.into_iter().map(|(c, _, _)| c).collect();
    let repeat = repeat_int.map(|v| v != 0).unwrap_or(false);
    let ttype_str = ttype.as_deref().unwrap_or("gapless");
    let mode = match mode_type.as_str() {
        // Loop points are SNAPSHOTTED from the first clip's library entry at
        // load time — editing the entry's markers changes track behavior only
        // after the next state reload (deliberate; the engine never re-reads
        // the DB mid-playback).
        "seamless" => {
            let (loop_start, loop_end) = match first_loops {
                Some((Some(ls), Some(le))) => (ls, le),
                _ => (0.0, 0.0),
            };
            TrackMode::Seamless { loop_start, loop_end }
        }
        "shuffle" => TrackMode::Shuffle { repeat, transition: build_transition(ttype_str, gmin.unwrap_or(0.0), gmax.unwrap_or(0.0)) },
        _ => TrackMode::Sequence { repeat, transition: build_transition(ttype_str, gmin.unwrap_or(0.0), gmax.unwrap_or(0.0)) },
    };
    Track { id, name, clips, volume, mode, fade_in, fade_out }
}

const TRACK_COLUMNS: &str =
    "id, name, volume, mode_type, repeat, transition_type, transition_gap_min, transition_gap_max, fade_in, fade_out";

/// Loads a single track by id with its clips, without loading the rest of
/// the app state. Used on the play/pause/stop path.
pub fn load_track_by_id(conn: &Connection, track_id: &str) -> Result<Track, String> {
    use rusqlite::OptionalExtension;
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1");
    let row = conn
        .query_row(&sql, params![track_id], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?, row.get::<_, Option<i64>>(4)?, row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<f64>>(6)?, row.get::<_, Option<f64>>(7)?,
                row.get::<_, f64>(8)?, row.get::<_, f64>(9)?,
            ))
        })
        .optional()
        .map_err(|e| format!("Failed to query track: {e}"))?
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    let clips = load_clips_resolved(conn, "cl.track_id = ?1", track_id)?
        .into_iter()
        .map(|(_, cwl)| cwl)
        .collect();
    Ok(assemble_track(row, clips))
}

/// Loads all tracks for a comp with their clips, without loading the rest of
/// the app state. Used on the play/pause/stop path.
pub fn load_tracks_for_comp(conn: &Connection, comp_id: &str) -> Result<Vec<Track>, String> {
    use rusqlite::OptionalExtension;
    let exists: Option<String> = conn
        .query_row("SELECT id FROM comps WHERE id = ?1", params![comp_id], |r| r.get(0))
        .optional()
        .map_err(|e| format!("Failed to check comp existence: {e}"))?;
    if exists.is_none() {
        return Err(format!("Comp not found: {}", comp_id));
    }

    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE comp_id = ?1 ORDER BY sort_order ASC");
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("Failed to prepare comp tracks query: {e}"))?;
    let rows = stmt
        .query_map(params![comp_id], |row| {
            Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, f64>(2)?,
                row.get::<_, String>(3)?, row.get::<_, Option<i64>>(4)?, row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<f64>>(6)?, row.get::<_, Option<f64>>(7)?,
                row.get::<_, f64>(8)?, row.get::<_, f64>(9)?,
            ))
        })
        .map_err(|e| format!("Failed to query comp tracks: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect comp tracks: {e}"))?;

    let mut clips_by_track: HashMap<String, Vec<ClipWithLoops>> = HashMap::new();
    for (track_id, cwl) in load_clips_resolved(
        conn,
        "cl.track_id IN (SELECT id FROM tracks WHERE comp_id = ?1)",
        comp_id,
    )? {
        clips_by_track.entry(track_id).or_default().push(cwl);
    }

    Ok(rows
        .into_iter()
        .map(|row| {
            let clips = clips_by_track.remove(&row.0).unwrap_or_default();
            assemble_track(row, clips)
        })
        .collect())
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

    fn index_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
    }

    const EXPECTED_INDEXES: [&str; 7] = [
        "idx_clips_library_entry_id",
        "idx_clips_track_id",
        "idx_comps_group_id",
        "idx_groups_configuration_id",
        "idx_library_entries_content_hash",
        "idx_library_entry_tags_tag_id",
        "idx_tracks_comp_id",
    ];

    #[test]
    fn fresh_database_has_indexes_and_v4_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("fresh.db")).unwrap();
        assert_eq!(index_names(&conn), EXPECTED_INDEXES);
        assert_eq!(read_schema_version(&conn), 4);
    }

    #[test]
    fn v3_database_gains_indexes_via_migration() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // Simulate a genuine pre-index v3 database.
        for name in EXPECTED_INDEXES {
            conn.execute_batch(&format!("DROP INDEX IF EXISTS {name}")).unwrap();
        }
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '3')", []).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(index_names(&conn), EXPECTED_INDEXES);
        assert_eq!(read_schema_version(&conn), 4);
    }

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
        // The stamp now happens in open_db (fresh-database detection), not in
        // init_schema's batch, so exercise open_db on a real temp-file DB.
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("lutedrop.db");
        let conn = open_db(&db_path).expect("open_db should succeed on fresh db");
        let version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema_version row should exist");
        assert_eq!(version, "4");
    }

    #[test]
    fn migrations_step_version_and_apply_in_order() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // init_schema no longer stamps a version (open_db does) — insert one.
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '1')", []).unwrap();

        fn mig1(c: &Connection) -> rusqlite::Result<()> {
            c.execute_batch("CREATE TABLE mig_one (id TEXT)")
        }
        fn mig2(c: &Connection) -> rusqlite::Result<()> {
            c.execute_batch("CREATE TABLE mig_two (id TEXT)")
        }
        run_migrations_with(&conn, &[(1, mig1 as Migration), (2, mig2 as Migration)], 3).unwrap();

        assert_eq!(read_schema_version(&conn), 3);
        // Both tables exist:
        conn.execute("INSERT INTO mig_one VALUES ('x')", []).unwrap();
        conn.execute("INSERT INTO mig_two VALUES ('x')", []).unwrap();
        // Idempotent: running again is a no-op.
        run_migrations_with(&conn, &[(1, mig1 as Migration), (2, mig2 as Migration)], 3).unwrap();
    }

    #[test]
    fn newer_database_than_app_is_rejected() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute("INSERT INTO meta (key, value) VALUES ('schema_version', '99')", []).unwrap();
        assert!(run_migrations_with(&conn, &[], 3).is_err());
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
            "music",
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
        let config_id = ensure_test_config(&conn);

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
    fn load_full_state_returns_ordered_nested_hierarchy_with_tags() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);

        let e1 = library::insert_library_entry(&conn, "/a.mp3", Some("h1"), "Alpha", None, None, None, "music").unwrap();
        let e2 = library::insert_library_entry(&conn, "/b.mp3", Some("h2"), "Beta", None, None, None, "ambiance").unwrap();
        library::add_tag_to_entry(&conn, &e1, "battle").unwrap();
        library::add_tag_to_entry(&conn, &e1, "ambient").unwrap();
        library::add_tag_to_entry(&conn, &e2, "battle").unwrap();

        let g1 = hierarchy::insert_group(&conn, "G1", &config_id).unwrap();
        let g2 = hierarchy::insert_group(&conn, "G2", &config_id).unwrap();
        let c1 = hierarchy::insert_comp(&conn, &g1, "C1").unwrap();
        let c2 = hierarchy::insert_comp(&conn, &g1, "C2").unwrap();
        let t1 = hierarchy::insert_track(&conn, &c1, "T1").unwrap();
        let t2 = hierarchy::insert_track(&conn, &c1, "T2").unwrap();
        hierarchy::insert_clip(&conn, &t1, &e1).unwrap();
        hierarchy::insert_clip(&conn, &t1, &e2).unwrap();

        let state = load_full_state(&conn).unwrap();

        assert_eq!(state.groups.len(), 2);
        assert_eq!(state.groups[0].name, "G1");
        assert_eq!(state.groups[1].name, "G2");
        assert_eq!(state.groups[0].comps.len(), 2);
        assert_eq!(state.groups[0].comps[0].name, "C1");
        assert_eq!(state.groups[0].comps[1].name, "C2");
        let tracks = &state.groups[0].comps[0].tracks;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "T1");
        assert_eq!(tracks[1].name, "T2");
        assert_eq!(tracks[0].clips.len(), 2);
        assert_eq!(tracks[0].clips[0].name, "Alpha");
        assert_eq!(tracks[0].clips[1].name, "Beta");
        // Clip resolves entry fields:
        assert_eq!(tracks[0].clips[0].file_path, "/a.mp3");
        // Entries are name-ordered with sorted tags:
        assert_eq!(state.library_entries[0].name, "Alpha");
        assert_eq!(state.library_entries[0].tags, vec!["ambient", "battle"]);
        assert_eq!(state.library_entries[1].tags, vec!["battle"]);
        assert_eq!(state.tags, vec!["ambient", "battle"]);
        // Second comp/track/group are empty but present:
        assert!(state.groups[0].comps[1].tracks.is_empty());
        assert!(state.groups[1].comps.is_empty());
        let _ = (t2, c2, g2);
    }

    #[test]
    fn targeted_track_lookup_matches_full_state() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);
        let e1 = library::insert_library_entry(&conn, "/x.mp3", None, "X", None, None, None, "music").unwrap();
        let g = hierarchy::insert_group(&conn, "G", &config_id).unwrap();
        let c = hierarchy::insert_comp(&conn, &g, "C").unwrap();
        let t1 = hierarchy::insert_track(&conn, &c, "T1").unwrap();
        let t2 = hierarchy::insert_track(&conn, &c, "T2").unwrap();
        hierarchy::insert_clip(&conn, &t1, &e1).unwrap();

        let state = load_full_state(&conn).unwrap();
        let full_t1 = state.groups[0].comps[0].tracks[0].clone();

        let direct = load_track_by_id(&conn, &t1).unwrap();
        assert_eq!(serde_json::to_value(&direct).unwrap(), serde_json::to_value(&full_t1).unwrap());

        let comp_tracks = load_tracks_for_comp(&conn, &c).unwrap();
        assert_eq!(comp_tracks.len(), 2);
        assert_eq!(comp_tracks[0].id, t1);
        assert_eq!(comp_tracks[1].id, t2);
        assert_eq!(comp_tracks[0].clips.len(), 1);
        assert_eq!(comp_tracks[0].clips[0].file_path, "/x.mp3");

        assert!(load_track_by_id(&conn, "nope").unwrap_err().contains("Track not found"));
        assert!(load_tracks_for_comp(&conn, "nope").unwrap_err().contains("Comp not found"));
    }

}
