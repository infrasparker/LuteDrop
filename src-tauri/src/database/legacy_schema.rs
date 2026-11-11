// Schema v17, retained only to open historical databases before migration.
use super::*;
const INDEX_DDL: &str = "
    CREATE INDEX IF NOT EXISTS idx_groups_configuration_id     ON groups(configuration_id);
    CREATE INDEX IF NOT EXISTS idx_comps_group_id              ON comps(group_id);
    CREATE INDEX IF NOT EXISTS idx_tracks_comp_id              ON tracks(comp_id);
    CREATE INDEX IF NOT EXISTS idx_clips_track_id              ON clips(track_id);
    CREATE INDEX IF NOT EXISTS idx_clips_library_entry_id      ON clips(library_entry_id);
    CREATE INDEX IF NOT EXISTS idx_library_entry_tags_tag_id   ON library_entry_tags(tag_id);
    CREATE INDEX IF NOT EXISTS idx_library_entries_content_hash ON library_entries(content_hash);
";

pub(super) fn add_indexes(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(INDEX_DDL)
}

pub(super) fn init_legacy_schema(conn: &Connection) -> SqlResult<()> {
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
            entry_type   TEXT NOT NULL DEFAULT 'music',
            role_user        TEXT,
            mood_reviewed_at TEXT,
            rg_track_gain_db REAL,
            rg_track_peak    REAL
        );

        CREATE TABLE IF NOT EXISTS tags (
            id   TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
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
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS library_entry_tags ({LIBRARY_ENTRY_TAGS_COLUMNS});"
    ))?;
    conn.execute_batch(ENTRY_ANALYSIS_DDL)?;
    conn.execute_batch(LIBRARY_ENTRY_MOODS_DDL)?;
    conn.execute_batch(MOOD_FLAGS_DDL)?;
    conn.execute_batch(INDEX_DDL)
}
