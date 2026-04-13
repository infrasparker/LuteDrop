# Media Library & SQLite Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace JSON persistence with SQLite, introduce a media library with freeform tagging, and deduplicate audio file references across playlists.

**Architecture:** All data (playlist hierarchy + library + tags) moves into a single SQLite database via `rusqlite`. The existing nested AppState model stays as the in-memory/frontend representation, built from SQL queries that join and resolve references. Clips become thin join rows pointing to library entries; names, file paths, and loop points are resolved at query time so engine and frontend code see familiar shapes.

**Tech Stack:** Rust (rusqlite, sha2), Vue 3 + Vuetify 4, ts-rs, Tauri 2

**Spec:** `docs/superpowers/specs/2026-04-12-media-library-design.md`

---

## File Structure

### New files

| File | Responsibility |
|------|---------------|
| `src-tauri/src/database/mod.rs` | Schema init, connection factory, `load_full_state()` |
| `src-tauri/src/database/library.rs` | Library entry + tag CRUD |
| `src-tauri/src/database/hierarchy.rs` | Group/comp/track/clip CRUD, sort order management |
| `src-tauri/src/database/migration.rs` | One-time JSON → SQLite migration |
| `src/views/LibraryView.vue` | Library management UI |
| `src/components/LibraryBrowser.vue` | Inline dialog for adding clips from library |
| `src/components/LibraryEntryRow.vue` | Single library entry in list |
| `src/components/TagFilter.vue` | Tag chip filter bar |
| `src/composables/useLibraryImport.ts` | File picker → library entry creation with dedup |

### Modified files

| File | What changes |
|------|-------------|
| `src-tauri/Cargo.toml` | Add `rusqlite`, `sha2` |
| `src-tauri/src/lib.rs` | Init SQLite instead of JSON, run migration, register new commands |
| `src-tauri/src/models.rs` | Add `LibraryEntry`, `Tag`; update `Clip` (add `libraryEntryId`), update `TrackMode::Seamless`; update `AppState` (add `libraryEntries`, `tags`); drop `lastSavedAt` |
| `src-tauri/src/state.rs` | Replace `save_path: PathBuf` with `db: Mutex<Connection>` |
| `src-tauri/src/actions.rs` | Rewrite all mutations to use database operations |
| `src-tauri/src/commands.rs` | Add library/import commands, update existing commands |
| `src-tauri/src/persistence.rs` | Keep `data_file_path` for migration detection; remove `save_to_disk` |
| `src-tauri/src/audio/engine.rs` | No structural changes — `Clip.file_path` and `TrackMode::Seamless { loop_start, loop_end }` still exist in the model, resolved from library entries at load time |
| `src-tauri/src/server.rs` | Update `StateSnapshot` to include library entries |
| `src-tauri/src/waveform.rs` | No changes — still keyed by file path |
| `src/stores/appStore.ts` | Add `libraryEntries`, `tags` state; add `findLibraryEntry()` lookup |
| `src/composables/useTrackImport.ts` | Replace with library-based workflow |
| `src/composables/useTrackUpdates.ts` | Loop point edits → update library entry instead of track |
| `src/components/TrackClipList.vue` | "Add Clips" opens LibraryBrowser instead of file picker |
| `src/components/TrackSettings.vue` | Loop point controls route to library entry update |
| `src/components/ClipRow.vue` | Resolve name/path from library entry |
| `src/components/WaveformEditor.vue` | Emit library entry update for loop points |
| `src/App.vue` | Add Library tab/navigation |
| `generated/` | Regenerated types from ts-rs |

### Deleted files

| File | Why |
|------|-----|
| `src-tauri/src/persistence.rs` | Replaced by `database/` module (after migration code extracted) |

---

## Task 1: Add dependencies and create database module with schema

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/database/mod.rs`

- [ ] **Step 1: Add rusqlite and sha2 to Cargo.toml**

Add to `[dependencies]`:

```toml
rusqlite = { version = "0.34", features = ["bundled"] }
sha2 = "0.10"
```

- [ ] **Step 2: Create database module with schema initialization**

Create `src-tauri/src/database/mod.rs`:

```rust
pub mod hierarchy;
pub mod library;
pub mod migration;

use rusqlite::{Connection, Result as SqlResult};
use std::path::Path;

/// Open (or create) the SQLite database and ensure schema is up to date.
pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("Failed to open database: {}", e))?;

    // Enable WAL mode for better concurrent read performance
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| format!("Failed to set WAL mode: {}", e))?;

    // Enable foreign key enforcement
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("Failed to enable foreign keys: {}", e))?;

    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS library_entries (
            id            TEXT PRIMARY KEY,
            file_path     TEXT NOT NULL UNIQUE,
            content_hash  TEXT,
            name          TEXT NOT NULL,
            duration      REAL,
            sample_rate   INTEGER,
            file_size     INTEGER,
            volume        REAL NOT NULL DEFAULT 1.0,
            trim_start    REAL,
            trim_end      REAL,
            loop_start    REAL,
            loop_end      REAL,
            created_at    TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tags (
            id   TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE
        );

        CREATE TABLE IF NOT EXISTS library_entry_tags (
            library_entry_id TEXT REFERENCES library_entries(id) ON DELETE CASCADE,
            tag_id           TEXT REFERENCES tags(id) ON DELETE CASCADE,
            PRIMARY KEY (library_entry_id, tag_id)
        );

        CREATE TABLE IF NOT EXISTS groups (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS comps (
            id         TEXT PRIMARY KEY,
            group_id   TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tracks (
            id                  TEXT PRIMARY KEY,
            comp_id             TEXT NOT NULL REFERENCES comps(id) ON DELETE CASCADE,
            name                TEXT NOT NULL,
            volume              REAL NOT NULL DEFAULT 1.0,
            mode_type           TEXT NOT NULL DEFAULT 'sequence',
            repeat              INTEGER DEFAULT 0,
            transition_type     TEXT DEFAULT 'gapless',
            transition_gap_min  REAL,
            transition_gap_max  REAL,
            fade_in             REAL NOT NULL DEFAULT 0.0,
            fade_out            REAL NOT NULL DEFAULT 0.0,
            sort_order          INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS clips (
            id                TEXT PRIMARY KEY,
            track_id          TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
            library_entry_id  TEXT NOT NULL REFERENCES library_entries(id),
            volume            REAL NOT NULL DEFAULT 1.0,
            sort_order        INTEGER NOT NULL
        );

        INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1');
        ",
    )
    .map_err(|e| format!("Failed to initialize schema: {}", e))?;

    Ok(())
}

#[cfg(test)]
pub fn open_memory_db() -> Connection {
    let conn = Connection::open_in_memory().expect("Failed to open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    init_schema(&conn).unwrap();
    conn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_creates_all_tables() {
        let conn = open_memory_db();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"library_entries".to_string()));
        assert!(tables.contains(&"tags".to_string()));
        assert!(tables.contains(&"library_entry_tags".to_string()));
        assert!(tables.contains(&"groups".to_string()));
        assert!(tables.contains(&"comps".to_string()));
        assert!(tables.contains(&"tracks".to_string()));
        assert!(tables.contains(&"clips".to_string()));
        assert!(tables.contains(&"meta".to_string()));
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = open_memory_db();
        // Running init_schema again should not error
        init_schema(&conn).unwrap();
    }

    #[test]
    fn schema_version_is_set() {
        let conn = open_memory_db();
        let version: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, "1");
    }
}
```

- [ ] **Step 3: Register the database module**

In `src-tauri/src/lib.rs`, add after the existing module declarations:

```rust
mod database;
```

- [ ] **Step 4: Run tests to verify schema creation**

Run: `cd src-tauri && cargo test database::tests -v`

Expected: All 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/database/mod.rs src-tauri/src/lib.rs
git commit -m "feat: add rusqlite dependency and database schema initialization"
```

---

## Task 2: Library entry and tag database operations

**Files:**
- Create: `src-tauri/src/database/library.rs`

- [ ] **Step 1: Write tests for library entry CRUD**

Create `src-tauri/src/database/library.rs` with tests first:

```rust
use rusqlite::{params, Connection};
use uuid::Uuid;

/// Sort-order gap for new items.
const SORT_GAP: i64 = 1000;

// -- Library Entries --

pub fn insert_library_entry(
    conn: &Connection,
    file_path: &str,
    content_hash: Option<&str>,
    name: &str,
    duration: Option<f64>,
    sample_rate: Option<u32>,
    file_size: Option<u64>,
) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let now = chrono_now();
    conn.execute(
        "INSERT INTO library_entries (id, file_path, content_hash, name, duration, sample_rate, file_size, volume, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1.0, ?8)",
        params![id, file_path, content_hash, name, duration, sample_rate.map(|v| v as i64), file_size.map(|v| v as i64), now],
    )
    .map_err(|e| format!("Failed to insert library entry: {}", e))?;
    Ok(id)
}

pub fn get_library_entry_by_hash(conn: &Connection, content_hash: &str) -> Result<Option<(String, String)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM library_entries WHERE content_hash = ?1")
        .map_err(|e| e.to_string())?;
    let result = stmt
        .query_row(params![content_hash], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(result)
}

pub fn update_library_entry_volume(conn: &Connection, id: &str, volume: f64) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET volume = ?1 WHERE id = ?2",
        params![volume, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_library_entry_name(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

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
    .map_err(|e| e.to_string())?;
    Ok(())
}

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
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_library_entry_file_path(conn: &Connection, id: &str, file_path: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE library_entries SET file_path = ?1 WHERE id = ?2",
        params![file_path, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_library_entry(conn: &Connection, id: &str) -> Result<(), String> {
    let clip_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM clips WHERE library_entry_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    if clip_count > 0 {
        return Err(format!(
            "Cannot delete library entry: still referenced by {} clip(s)",
            clip_count
        ));
    }

    conn.execute("DELETE FROM library_entries WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// -- Tags --

/// Ensures a tag with the given name exists. Returns its ID.
/// Tag names are normalized to lowercase.
pub fn ensure_tag(conn: &Connection, name: &str) -> Result<String, String> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return Err("Tag name cannot be empty".to_string());
    }

    // Check if tag already exists
    let existing: Option<String> = conn
        .prepare("SELECT id FROM tags WHERE name = ?1")
        .map_err(|e| e.to_string())?
        .query_row(params![normalized], |row| row.get(0))
        .optional()
        .map_err(|e| e.to_string())?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO tags (id, name) VALUES (?1, ?2)",
        params![id, normalized],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn add_tag_to_entry(conn: &Connection, entry_id: &str, tag_name: &str) -> Result<(), String> {
    let tag_id = ensure_tag(conn, tag_name)?;
    conn.execute(
        "INSERT OR IGNORE INTO library_entry_tags (library_entry_id, tag_id) VALUES (?1, ?2)",
        params![entry_id, tag_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_tag_from_entry(conn: &Connection, entry_id: &str, tag_name: &str) -> Result<(), String> {
    let normalized = tag_name.trim().to_lowercase();
    conn.execute(
        "DELETE FROM library_entry_tags
         WHERE library_entry_id = ?1
         AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
        params![entry_id, normalized],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_tags_for_entry(conn: &Connection, entry_id: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.name FROM tags t
             JOIN library_entry_tags lt ON lt.tag_id = t.id
             WHERE lt.library_entry_id = ?1
             ORDER BY t.name",
        )
        .map_err(|e| e.to_string())?;

    let tags: Vec<String> = stmt
        .query_map(params![entry_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(tags)
}

pub fn list_all_tags(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM tags ORDER BY name")
        .map_err(|e| e.to_string())?;

    let tags: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    Ok(tags)
}

fn chrono_now() -> String {
    use std::time::SystemTime;
    let d = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", d.as_millis())
}

use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_memory_db;

    #[test]
    fn insert_and_find_by_hash() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/music/song.mp3", Some("abc123"), "Song", Some(180.0), Some(44100), Some(5000000)).unwrap();
        let found = get_library_entry_by_hash(&conn, "abc123").unwrap();
        assert!(found.is_some());
        let (found_id, found_name) = found.unwrap();
        assert_eq!(found_id, id);
        assert_eq!(found_name, "Song");
    }

    #[test]
    fn duplicate_file_path_rejected() {
        let conn = open_memory_db();
        insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();
        let result = insert_library_entry(&conn, "/music/song.mp3", Some("def"), "Song 2", None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn update_volume() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();
        update_library_entry_volume(&conn, &id, 0.75).unwrap();
        let vol: f64 = conn.query_row("SELECT volume FROM library_entries WHERE id = ?1", params![id], |r| r.get(0)).unwrap();
        assert!((vol - 0.75).abs() < 0.001);
    }

    #[test]
    fn delete_blocked_by_clips() {
        let conn = open_memory_db();
        let entry_id = insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();

        // Create a group/comp/track/clip referencing this entry
        conn.execute("INSERT INTO groups (id, name, sort_order) VALUES ('g1', 'Group', 1000)", []).unwrap();
        conn.execute("INSERT INTO comps (id, group_id, name, sort_order) VALUES ('c1', 'g1', 'Comp', 1000)", []).unwrap();
        conn.execute("INSERT INTO tracks (id, comp_id, name, volume, mode_type, fade_in, fade_out, sort_order) VALUES ('t1', 'c1', 'Track', 1.0, 'sequence', 0.0, 0.0, 1000)", []).unwrap();
        conn.execute("INSERT INTO clips (id, track_id, library_entry_id, volume, sort_order) VALUES ('cl1', 't1', ?1, 1.0, 1000)", params![entry_id]).unwrap();

        let result = delete_library_entry(&conn, &entry_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("still referenced"));
    }

    #[test]
    fn delete_succeeds_when_unreferenced() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();
        delete_library_entry(&conn, &id).unwrap();
        let found = get_library_entry_by_hash(&conn, "abc").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn tag_lifecycle() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();

        add_tag_to_entry(&conn, &id, "Somber").unwrap();
        add_tag_to_entry(&conn, &id, "Tavern").unwrap();

        let tags = get_tags_for_entry(&conn, &id).unwrap();
        assert_eq!(tags, vec!["somber", "tavern"]); // normalized lowercase, sorted

        let all_tags = list_all_tags(&conn).unwrap();
        assert_eq!(all_tags, vec!["somber", "tavern"]);

        remove_tag_from_entry(&conn, &id, "somber").unwrap();
        let tags = get_tags_for_entry(&conn, &id).unwrap();
        assert_eq!(tags, vec!["tavern"]);
    }

    #[test]
    fn duplicate_tag_is_idempotent() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();
        add_tag_to_entry(&conn, &id, "combat").unwrap();
        add_tag_to_entry(&conn, &id, "combat").unwrap(); // no error
        let tags = get_tags_for_entry(&conn, &id).unwrap();
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn tag_normalization() {
        let conn = open_memory_db();
        let id = insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();
        add_tag_to_entry(&conn, &id, "  Combat  ").unwrap();
        let tags = get_tags_for_entry(&conn, &id).unwrap();
        assert_eq!(tags, vec!["combat"]);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd src-tauri && cargo test database::library::tests -v`

Expected: All 7 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/database/library.rs
git commit -m "feat: library entry and tag database operations"
```

---

## Task 3: Hierarchy database operations and sort order

**Files:**
- Create: `src-tauri/src/database/hierarchy.rs`

- [ ] **Step 1: Write hierarchy CRUD with sort order management**

Create `src-tauri/src/database/hierarchy.rs`:

```rust
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

const SORT_GAP: i64 = 1000;

// -- Sort Order Utilities --

/// Get the next sort_order value for appending to a list.
fn next_sort_order(conn: &Connection, table: &str, parent_col: &str, parent_id: &str) -> Result<i64, String> {
    let sql = format!(
        "SELECT COALESCE(MAX(sort_order), 0) + {} FROM {} WHERE {} = ?1",
        SORT_GAP, table, parent_col
    );
    conn.query_row(&sql, params![parent_id], |row| row.get(0))
        .map_err(|e| e.to_string())
}

/// Get the next sort_order for a top-level table (no parent column).
fn next_sort_order_top(conn: &Connection, table: &str) -> Result<i64, String> {
    let sql = format!(
        "SELECT COALESCE(MAX(sort_order), 0) + {} FROM {}",
        SORT_GAP, table
    );
    conn.query_row(&sql, [], |row| row.get(0))
        .map_err(|e| e.to_string())
}

/// Move an item to a new position (0-indexed). Calculates midpoint between neighbors.
/// If no gap remains, renumbers the entire list.
pub fn reorder_item(
    conn: &Connection,
    table: &str,
    parent_col: Option<&str>,
    parent_id: Option<&str>,
    item_id: &str,
    new_index: usize,
) -> Result<(), String> {
    // Get all items in order (excluding the one being moved)
    let (items, _moved_sort) = {
        let (where_clause, param_val): (String, Option<&str>) = match (parent_col, parent_id) {
            (Some(col), Some(pid)) => (format!("WHERE {} = ?1", col), Some(pid)),
            _ => ("".to_string(), None),
        };

        let sql = format!(
            "SELECT id, sort_order FROM {} {} ORDER BY sort_order",
            table, where_clause
        );

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows: Vec<(String, i64)> = if let Some(p) = param_val {
            stmt.query_map(params![p], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect()
        };

        let moved_sort = rows.iter().find(|(id, _)| id == item_id).map(|(_, s)| *s);
        let others: Vec<(String, i64)> = rows.into_iter().filter(|(id, _)| id != item_id).collect();
        (others, moved_sort)
    };

    if items.is_empty() {
        // Only item, set to SORT_GAP
        conn.execute(
            &format!("UPDATE {} SET sort_order = ?1 WHERE id = ?2", table),
            params![SORT_GAP, item_id],
        )
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let new_index = new_index.min(items.len());

    let new_sort = if new_index == 0 {
        // Before first item
        let first = items[0].1;
        if first > 1 {
            first / 2
        } else {
            // Need to renumber
            renumber_list(conn, table, parent_col, parent_id, item_id, new_index)?;
            return Ok(());
        }
    } else if new_index >= items.len() {
        // After last item
        items[items.len() - 1].1 + SORT_GAP
    } else {
        // Between two items
        let before = items[new_index - 1].1;
        let after = items[new_index].1;
        if after - before > 1 {
            before + (after - before) / 2
        } else {
            renumber_list(conn, table, parent_col, parent_id, item_id, new_index)?;
            return Ok(());
        }
    };

    conn.execute(
        &format!("UPDATE {} SET sort_order = ?1 WHERE id = ?2", table),
        params![new_sort, item_id],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Renumber all items in a list with fresh gaps, inserting item_id at new_index.
fn renumber_list(
    conn: &Connection,
    table: &str,
    parent_col: Option<&str>,
    parent_id: Option<&str>,
    item_id: &str,
    new_index: usize,
) -> Result<(), String> {
    let (where_clause, param_val): (String, Option<&str>) = match (parent_col, parent_id) {
        (Some(col), Some(pid)) => (format!("WHERE {} = ?1", col), Some(pid)),
        _ => ("".to_string(), None),
    };

    let sql = format!(
        "SELECT id FROM {} {} ORDER BY sort_order",
        table, where_clause
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut ids: Vec<String> = if let Some(p) = param_val {
        stmt.query_map(params![p], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        stmt.query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect()
    };

    // Remove the moved item and reinsert at new_index
    ids.retain(|id| id != item_id);
    let idx = new_index.min(ids.len());
    ids.insert(idx, item_id.to_string());

    let update_sql = format!("UPDATE {} SET sort_order = ?1 WHERE id = ?2", table);
    for (i, id) in ids.iter().enumerate() {
        let sort = ((i as i64) + 1) * SORT_GAP;
        conn.execute(&update_sql, params![sort, id])
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// -- Groups --

pub fn insert_group(conn: &Connection, name: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let sort = next_sort_order_top(conn, "groups")?;
    conn.execute(
        "INSERT INTO groups (id, name, sort_order) VALUES (?1, ?2, ?3)",
        params![id, name, sort],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn delete_group(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM groups WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn rename_group(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute("UPDATE groups SET name = ?1 WHERE id = ?2", params![name, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn reorder_group(conn: &Connection, group_id: &str, new_index: usize) -> Result<(), String> {
    reorder_item(conn, "groups", None, None, group_id, new_index)
}

// -- Comps --

pub fn insert_comp(conn: &Connection, group_id: &str, name: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let sort = next_sort_order(conn, "comps", "group_id", group_id)?;
    conn.execute(
        "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
        params![id, group_id, name, sort],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn delete_comp(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM comps WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn rename_comp(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute("UPDATE comps SET name = ?1 WHERE id = ?2", params![name, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn reorder_comp(conn: &Connection, group_id: &str, comp_id: &str, new_index: usize) -> Result<(), String> {
    reorder_item(conn, "comps", Some("group_id"), Some(group_id), comp_id, new_index)
}

// -- Tracks --

pub fn insert_track(conn: &Connection, comp_id: &str, name: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let sort = next_sort_order(conn, "tracks", "comp_id", comp_id)?;
    conn.execute(
        "INSERT INTO tracks (id, comp_id, name, volume, mode_type, repeat, transition_type, fade_in, fade_out, sort_order)
         VALUES (?1, ?2, ?3, 1.0, 'sequence', 1, 'gapless', 0.0, 0.0, ?4)",
        params![id, comp_id, name, sort],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn delete_track(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM tracks WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn rename_track(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute("UPDATE tracks SET name = ?1 WHERE id = ?2", params![name, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_track_volume(conn: &Connection, id: &str, volume: f64) -> Result<(), String> {
    conn.execute("UPDATE tracks SET volume = ?1 WHERE id = ?2", params![volume, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_track_mode(
    conn: &Connection,
    id: &str,
    mode_type: &str,
    repeat: Option<bool>,
    transition_type: Option<&str>,
    transition_gap_min: Option<f64>,
    transition_gap_max: Option<f64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE tracks SET mode_type = ?1, repeat = ?2, transition_type = ?3,
         transition_gap_min = ?4, transition_gap_max = ?5 WHERE id = ?6",
        params![
            mode_type,
            repeat.map(|b| b as i32),
            transition_type,
            transition_gap_min,
            transition_gap_max,
            id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_track_fades(conn: &Connection, id: &str, fade_in: f64, fade_out: f64) -> Result<(), String> {
    conn.execute(
        "UPDATE tracks SET fade_in = ?1, fade_out = ?2 WHERE id = ?3",
        params![fade_in, fade_out, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn reorder_track(conn: &Connection, comp_id: &str, track_id: &str, new_index: usize) -> Result<(), String> {
    reorder_item(conn, "tracks", Some("comp_id"), Some(comp_id), track_id, new_index)
}

// -- Clips --

pub fn insert_clip(conn: &Connection, track_id: &str, library_entry_id: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let sort = next_sort_order(conn, "clips", "track_id", track_id)?;
    conn.execute(
        "INSERT INTO clips (id, track_id, library_entry_id, volume, sort_order)
         VALUES (?1, ?2, ?3, 1.0, ?4)",
        params![id, track_id, library_entry_id, sort],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn delete_clip(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM clips WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_clip_volume(conn: &Connection, id: &str, volume: f64) -> Result<(), String> {
    conn.execute("UPDATE clips SET volume = ?1 WHERE id = ?2", params![volume, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn reorder_clip(conn: &Connection, track_id: &str, clip_id: &str, new_index: usize) -> Result<(), String> {
    reorder_item(conn, "clips", Some("track_id"), Some(track_id), clip_id, new_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_memory_db;
    use crate::database::library::insert_library_entry;

    #[test]
    fn group_crud() {
        let conn = open_memory_db();
        let id = insert_group(&conn, "Combat").unwrap();
        rename_group(&conn, &id, "Battle").unwrap();
        let name: String = conn.query_row("SELECT name FROM groups WHERE id = ?1", params![id], |r| r.get(0)).unwrap();
        assert_eq!(name, "Battle");
        delete_group(&conn, &id).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM groups", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn sort_order_increments() {
        let conn = open_memory_db();
        let g1 = insert_group(&conn, "First").unwrap();
        let g2 = insert_group(&conn, "Second").unwrap();
        let g3 = insert_group(&conn, "Third").unwrap();

        let orders: Vec<(String, i64)> = conn
            .prepare("SELECT name, sort_order FROM groups ORDER BY sort_order")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(orders[0].0, "First");
        assert_eq!(orders[1].0, "Second");
        assert_eq!(orders[2].0, "Third");
        assert!(orders[0].1 < orders[1].1);
        assert!(orders[1].1 < orders[2].1);
    }

    #[test]
    fn reorder_moves_to_middle() {
        let conn = open_memory_db();
        let _g1 = insert_group(&conn, "A").unwrap();
        let _g2 = insert_group(&conn, "B").unwrap();
        let g3 = insert_group(&conn, "C").unwrap();

        // Move C to position 1 (between A and B)
        reorder_group(&conn, &g3, 1).unwrap();

        let names: Vec<String> = conn
            .prepare("SELECT name FROM groups ORDER BY sort_order")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(names, vec!["A", "C", "B"]);
    }

    #[test]
    fn cascade_deletes() {
        let conn = open_memory_db();
        let entry_id = insert_library_entry(&conn, "/music/song.mp3", Some("abc"), "Song", None, None, None).unwrap();
        let gid = insert_group(&conn, "G").unwrap();
        let cid = insert_comp(&conn, &gid, "C").unwrap();
        let tid = insert_track(&conn, &cid, "T").unwrap();
        insert_clip(&conn, &tid, &entry_id).unwrap();

        // Deleting the group should cascade to comp, track, clip
        delete_group(&conn, &gid).unwrap();

        let comp_count: i64 = conn.query_row("SELECT COUNT(*) FROM comps", [], |r| r.get(0)).unwrap();
        let track_count: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0)).unwrap();
        let clip_count: i64 = conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0)).unwrap();
        assert_eq!(comp_count, 0);
        assert_eq!(track_count, 0);
        assert_eq!(clip_count, 0);

        // Library entry should still exist (not cascaded)
        let entry_count: i64 = conn.query_row("SELECT COUNT(*) FROM library_entries", [], |r| r.get(0)).unwrap();
        assert_eq!(entry_count, 1);
    }

    #[test]
    fn clip_insert_and_reorder() {
        let conn = open_memory_db();
        let e1 = insert_library_entry(&conn, "/a.mp3", Some("a"), "A", None, None, None).unwrap();
        let e2 = insert_library_entry(&conn, "/b.mp3", Some("b"), "B", None, None, None).unwrap();
        let e3 = insert_library_entry(&conn, "/c.mp3", Some("c"), "C", None, None, None).unwrap();

        let gid = insert_group(&conn, "G").unwrap();
        let cid = insert_comp(&conn, &gid, "C").unwrap();
        let tid = insert_track(&conn, &cid, "T").unwrap();

        let cl1 = insert_clip(&conn, &tid, &e1).unwrap();
        let cl2 = insert_clip(&conn, &tid, &e2).unwrap();
        let cl3 = insert_clip(&conn, &tid, &e3).unwrap();

        // Move clip 3 to first position
        reorder_clip(&conn, &tid, &cl3, 0).unwrap();

        let clip_entries: Vec<String> = conn
            .prepare("SELECT library_entry_id FROM clips WHERE track_id = ?1 ORDER BY sort_order")
            .unwrap()
            .query_map(params![tid], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(clip_entries, vec![e3, e1, e2]);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src-tauri && cargo test database::hierarchy::tests -v`

Expected: All 5 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/database/hierarchy.rs
git commit -m "feat: hierarchy database operations with sort order management"
```

---

## Task 4: Update Rust models

**Files:**
- Modify: `src-tauri/src/models.rs`

- [ ] **Step 1: Add LibraryEntry and Tag models, update existing models**

Replace `src-tauri/src/models.rs` with:

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub id: String,
    pub file_path: String,
    pub content_hash: Option<String>,
    pub name: String,
    pub duration: Option<f64>,
    pub sample_rate: Option<u32>,
    pub file_size: Option<u64>,
    pub volume: f64,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub loop_start: Option<f64>,
    pub loop_end: Option<f64>,
    pub created_at: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub library_entry_id: String,
    pub name: String,
    pub file_path: String,
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrackTransition {
    Gapless,
    Gap { min: f64, max: f64 },
    Crossfade,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrackMode {
    Seamless {
        #[serde(rename = "loopStart")]
        loop_start: f64,
        #[serde(rename = "loopEnd")]
        loop_end: f64,
    },
    Sequence {
        repeat: bool,
        transition: TrackTransition,
    },
    Shuffle {
        repeat: bool,
        transition: TrackTransition,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub clips: Vec<Clip>,
    pub volume: f64,
    pub mode: TrackMode,
    pub fade_in: f64,
    pub fade_out: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
pub struct Comp {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub comps: Vec<Comp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub groups: Vec<Group>,
    pub library_entries: Vec<LibraryEntry>,
    pub tags: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            library_entries: Vec::new(),
            tags: Vec::new(),
        }
    }
}

/// Sent from Rust to the frontend when track playback state changes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TrackPlaybackState {
    pub track_id: String,
    pub is_playing: bool,
    pub is_paused: bool,
    pub current_clip_id: Option<String>,
    pub current_time: f64,
}
```

Key changes:
- `Clip` gains `library_entry_id`, keeps `name` and `file_path` (resolved from library entry at load time)
- `AppState` gains `library_entries` and `tags`, drops `last_saved_at`
- `LibraryEntry` is a new struct with all library fields + embedded tags
- `TrackMode::Seamless` is unchanged (loop points resolved from library entry during state loading)

- [ ] **Step 2: Verify it compiles**

Run: `cd src-tauri && cargo check`

Expected: Compilation errors in `actions.rs`, `commands.rs`, `server.rs`, and `persistence.rs` (they reference old model shapes). This is expected — we'll fix those in later tasks. For now, confirm that `models.rs` itself is valid.

Note: The build will not pass yet. This is intentional — later tasks fix the remaining files.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/models.rs
git commit -m "feat: update models for library entries, tags, and SQLite-backed state"
```

---

## Task 5: Full state loading from SQLite

**Files:**
- Modify: `src-tauri/src/database/mod.rs`

- [ ] **Step 1: Add load_full_state function**

Add to `src-tauri/src/database/mod.rs`, after the existing `init_schema` function:

```rust
use crate::models::{
    AppState, Clip, Comp, Group, LibraryEntry, Track, TrackMode, TrackTransition,
};
use std::collections::HashMap;

/// Build the complete AppState from SQLite.
/// Resolves all references: clips get names/paths from library entries,
/// Seamless mode gets loop points from the first clip's library entry.
pub fn load_full_state(conn: &Connection) -> Result<AppState, String> {
    let library_entries = load_library_entries(conn)?;
    let tags = library::list_all_tags(conn)?;

    // Build a lookup map for resolving clip references
    let entry_map: HashMap<String, &LibraryEntry> = library_entries.iter().map(|e| (e.id.clone(), e)).collect();

    // Load hierarchy
    let groups = load_groups(conn, &entry_map)?;

    Ok(AppState {
        groups,
        library_entries,
        tags,
    })
}

fn load_library_entries(conn: &Connection) -> Result<Vec<LibraryEntry>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, file_path, content_hash, name, duration, sample_rate, file_size,
                    volume, trim_start, trim_end, loop_start, loop_end, created_at
             FROM library_entries ORDER BY name",
        )
        .map_err(|e| e.to_string())?;

    let entries: Vec<LibraryEntry> = stmt
        .query_map([], |row| {
            Ok(LibraryEntry {
                id: row.get(0)?,
                file_path: row.get(1)?,
                content_hash: row.get(2)?,
                name: row.get(3)?,
                duration: row.get(4)?,
                sample_rate: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                file_size: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                volume: row.get(7)?,
                trim_start: row.get(8)?,
                trim_end: row.get(9)?,
                loop_start: row.get(10)?,
                loop_end: row.get(11)?,
                created_at: row.get(12)?,
                tags: Vec::new(), // filled below
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    // Fill in tags for each entry
    let mut result = entries;
    for entry in &mut result {
        entry.tags = library::get_tags_for_entry(conn, &entry.id)?;
    }

    Ok(result)
}

fn load_groups(conn: &Connection, entry_map: &HashMap<String, &LibraryEntry>) -> Result<Vec<Group>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM groups ORDER BY sort_order")
        .map_err(|e| e.to_string())?;

    let groups: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut result = Vec::new();
    for (group_id, group_name) in groups {
        let comps = load_comps(conn, &group_id, entry_map)?;
        result.push(Group {
            id: group_id,
            name: group_name,
            comps,
        });
    }
    Ok(result)
}

fn load_comps(conn: &Connection, group_id: &str, entry_map: &HashMap<String, &LibraryEntry>) -> Result<Vec<Comp>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM comps WHERE group_id = ?1 ORDER BY sort_order")
        .map_err(|e| e.to_string())?;

    let comps: Vec<(String, String)> = stmt
        .query_map(params![group_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut result = Vec::new();
    for (comp_id, comp_name) in comps {
        let tracks = load_tracks(conn, &comp_id, entry_map)?;
        result.push(Comp {
            id: comp_id,
            name: comp_name,
            tracks,
        });
    }
    Ok(result)
}

fn load_tracks(conn: &Connection, comp_id: &str, entry_map: &HashMap<String, &LibraryEntry>) -> Result<Vec<Track>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, volume, mode_type, repeat, transition_type,
                    transition_gap_min, transition_gap_max, fade_in, fade_out
             FROM tracks WHERE comp_id = ?1 ORDER BY sort_order",
        )
        .map_err(|e| e.to_string())?;

    let tracks: Vec<_> = stmt
        .query_map(params![comp_id], |row| {
            Ok((
                row.get::<_, String>(0)?,      // id
                row.get::<_, String>(1)?,      // name
                row.get::<_, f64>(2)?,         // volume
                row.get::<_, String>(3)?,      // mode_type
                row.get::<_, Option<i32>>(4)?, // repeat
                row.get::<_, Option<String>>(5)?, // transition_type
                row.get::<_, Option<f64>>(6)?,    // gap_min
                row.get::<_, Option<f64>>(7)?,    // gap_max
                row.get::<_, f64>(8)?,         // fade_in
                row.get::<_, f64>(9)?,         // fade_out
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut result = Vec::new();
    for (id, name, volume, mode_type, repeat, transition_type, gap_min, gap_max, fade_in, fade_out) in tracks {
        let clips = load_clips(conn, &id, entry_map)?;

        let mode = build_track_mode(&mode_type, repeat, transition_type.as_deref(), gap_min, gap_max, &clips, entry_map);

        result.push(Track {
            id,
            name,
            clips,
            volume,
            mode,
            fade_in,
            fade_out,
        });
    }
    Ok(result)
}

fn load_clips(conn: &Connection, track_id: &str, entry_map: &HashMap<String, &LibraryEntry>) -> Result<Vec<Clip>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, library_entry_id, volume FROM clips WHERE track_id = ?1 ORDER BY sort_order",
        )
        .map_err(|e| e.to_string())?;

    let clips: Vec<Clip> = stmt
        .query_map(params![track_id], |row| {
            let clip_id: String = row.get(0)?;
            let entry_id: String = row.get(1)?;
            let volume: f64 = row.get(2)?;
            Ok((clip_id, entry_id, volume))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .map(|(clip_id, entry_id, volume)| {
            let (name, file_path) = entry_map
                .get(&entry_id)
                .map(|e| (e.name.clone(), e.file_path.clone()))
                .unwrap_or_else(|| ("[missing]".to_string(), String::new()));

            Clip {
                id: clip_id,
                library_entry_id: entry_id,
                name,
                file_path,
                volume,
            }
        })
        .collect();

    Ok(clips)
}

/// Build a TrackMode from flattened database columns.
/// For Seamless mode, resolves loop_start/loop_end from the first clip's library entry.
fn build_track_mode(
    mode_type: &str,
    repeat: Option<i32>,
    transition_type: Option<&str>,
    gap_min: Option<f64>,
    gap_max: Option<f64>,
    clips: &[Clip],
    entry_map: &HashMap<String, &LibraryEntry>,
) -> TrackMode {
    match mode_type {
        "seamless" => {
            // Resolve loop points from the first clip's library entry
            let (ls, le) = clips
                .first()
                .and_then(|c| entry_map.get(&c.library_entry_id))
                .map(|e| (e.loop_start.unwrap_or(0.0), e.loop_end.unwrap_or(0.0)))
                .unwrap_or((0.0, 0.0));

            TrackMode::Seamless {
                loop_start: ls,
                loop_end: le,
            }
        }
        "shuffle" => {
            let transition = build_transition(transition_type, gap_min, gap_max);
            TrackMode::Shuffle {
                repeat: repeat.unwrap_or(0) != 0,
                transition,
            }
        }
        _ => {
            // Default: sequence
            let transition = build_transition(transition_type, gap_min, gap_max);
            TrackMode::Sequence {
                repeat: repeat.unwrap_or(1) != 0,
                transition,
            }
        }
    }
}

fn build_transition(
    transition_type: Option<&str>,
    gap_min: Option<f64>,
    gap_max: Option<f64>,
) -> TrackTransition {
    match transition_type {
        Some("gap") => TrackTransition::Gap {
            min: gap_min.unwrap_or(0.0),
            max: gap_max.unwrap_or(0.0),
        },
        Some("crossfade") => TrackTransition::Crossfade,
        _ => TrackTransition::Gapless,
    }
}
```

- [ ] **Step 2: Write tests for full state loading**

Add to the `#[cfg(test)] mod tests` block in `database/mod.rs`:

```rust
#[test]
fn load_full_state_empty_db() {
    let conn = open_memory_db();
    let state = load_full_state(&conn).unwrap();
    assert!(state.groups.is_empty());
    assert!(state.library_entries.is_empty());
    assert!(state.tags.is_empty());
}

#[test]
fn load_full_state_with_data() {
    let conn = open_memory_db();

    // Create a library entry with tags
    let entry_id = library::insert_library_entry(&conn, "/music/tavern.mp3", Some("hash1"), "Tavern Ambience", Some(180.0), Some(44100), Some(5000000)).unwrap();
    library::add_tag_to_entry(&conn, &entry_id, "peaceful").unwrap();
    library::add_tag_to_entry(&conn, &entry_id, "tavern").unwrap();

    // Set loop points on the library entry
    conn.execute(
        "UPDATE library_entries SET loop_start = 30.0, loop_end = 170.0 WHERE id = ?1",
        params![entry_id],
    ).unwrap();

    // Create hierarchy
    let gid = hierarchy::insert_group(&conn, "Exploration").unwrap();
    let cid = hierarchy::insert_comp(&conn, &gid, "Tavern Scene").unwrap();
    let tid = hierarchy::insert_track(&conn, &cid, "Ambience").unwrap();

    // Set track to seamless mode
    hierarchy::update_track_mode(&conn, &tid, "seamless", None, None, None, None).unwrap();

    // Add clip
    hierarchy::insert_clip(&conn, &tid, &entry_id).unwrap();

    let state = load_full_state(&conn).unwrap();

    assert_eq!(state.groups.len(), 1);
    assert_eq!(state.groups[0].name, "Exploration");
    assert_eq!(state.groups[0].comps.len(), 1);
    assert_eq!(state.groups[0].comps[0].tracks.len(), 1);

    let track = &state.groups[0].comps[0].tracks[0];
    assert_eq!(track.clips.len(), 1);
    assert_eq!(track.clips[0].name, "Tavern Ambience");
    assert_eq!(track.clips[0].file_path, "/music/tavern.mp3");
    assert_eq!(track.clips[0].library_entry_id, entry_id);

    // Seamless mode should have resolved loop points from library entry
    match &track.mode {
        TrackMode::Seamless { loop_start, loop_end } => {
            assert!((loop_start - 30.0).abs() < 0.001);
            assert!((loop_end - 170.0).abs() < 0.001);
        }
        _ => panic!("Expected Seamless mode"),
    }

    // Library entries should include tags
    assert_eq!(state.library_entries.len(), 1);
    assert_eq!(state.library_entries[0].tags, vec!["peaceful", "tavern"]);

    assert_eq!(state.tags, vec!["peaceful", "tavern"]);
}
```

- [ ] **Step 3: Run tests**

Run: `cd src-tauri && cargo test database::tests -v`

Expected: All 5 tests PASS (3 from Task 1 + 2 new).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/database/mod.rs
git commit -m "feat: full AppState loading from SQLite with reference resolution"
```

---

## Task 6: JSON-to-SQLite migration

**Files:**
- Create: `src-tauri/src/database/migration.rs`

- [ ] **Step 1: Create the old model types for reading legacy JSON**

The migration needs to read the old JSON format (where Clip has `file_path` and `name`, AppState has `lastSavedAt`). Define these as private types in the migration module so the rest of the codebase uses the new models:

```rust
use rusqlite::{params, Connection};
use serde::Deserialize;
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use uuid::Uuid;

use crate::database::{hierarchy, library};

// -- Legacy JSON types (read-only, for migration) --

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OldAppState {
    groups: Vec<OldGroup>,
    #[allow(dead_code)]
    last_saved_at: Option<String>,
}

#[derive(Deserialize)]
struct OldGroup {
    id: String,
    name: String,
    comps: Vec<OldComp>,
}

#[derive(Deserialize)]
struct OldComp {
    id: String,
    name: String,
    tracks: Vec<OldTrack>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OldTrack {
    id: String,
    name: String,
    clips: Vec<OldClip>,
    volume: f64,
    mode: serde_json::Value, // Keep as raw JSON to parse mode_type + fields
    fade_in: f64,
    fade_out: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OldClip {
    id: String,
    name: String,
    file_path: String,
    volume: f64,
}

const SORT_GAP: i64 = 1000;

/// Check whether we need to migrate from JSON.
/// Returns true if the JSON file exists and the database has no groups.
pub fn needs_migration(conn: &Connection, json_path: &Path) -> bool {
    if !json_path.exists() {
        return false;
    }
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM groups", [], |r| r.get(0))
        .unwrap_or(0);
    count == 0
}

/// Migrate from the legacy JSON file into the SQLite database.
/// Deduplicates audio files by content hash.
/// Returns (groups_migrated, entries_created, files_missing).
pub fn migrate_from_json(
    conn: &Connection,
    json_path: &Path,
) -> Result<(usize, usize, usize), String> {
    let json_str = fs::read_to_string(json_path)
        .map_err(|e| format!("Failed to read JSON file: {}", e))?;

    let old_state: OldAppState = serde_json::from_str(&json_str)
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Phase 1: Collect all unique audio files, deduplicate by hash
    let mut file_path_to_entry_id: HashMap<String, String> = HashMap::new();
    let mut hash_to_entry_id: HashMap<String, String> = HashMap::new();
    let mut entries_created: usize = 0;
    let mut files_missing: usize = 0;

    for group in &old_state.groups {
        for comp in &group.comps {
            for track in &comp.tracks {
                for clip in &track.clips {
                    if file_path_to_entry_id.contains_key(&clip.file_path) {
                        continue;
                    }

                    let (content_hash, is_missing) = hash_file(&clip.file_path);

                    if is_missing {
                        files_missing += 1;
                    }

                    // Check for dedup by hash
                    if let Some(ref hash) = content_hash {
                        if let Some(existing_id) = hash_to_entry_id.get(hash) {
                            file_path_to_entry_id.insert(clip.file_path.clone(), existing_id.clone());
                            continue;
                        }
                    }

                    let entry_id = library::insert_library_entry(
                        conn,
                        &clip.file_path,
                        content_hash.as_deref(),
                        &clip.name,
                        None, // duration populated on first waveform render
                        None,
                        None,
                    )?;

                    file_path_to_entry_id.insert(clip.file_path.clone(), entry_id.clone());
                    if let Some(hash) = content_hash {
                        hash_to_entry_id.insert(hash, entry_id);
                    }
                    entries_created += 1;
                }
            }
        }
    }

    // Phase 2: Recreate the hierarchy
    for (gi, group) in old_state.groups.iter().enumerate() {
        let group_sort = ((gi as i64) + 1) * SORT_GAP;
        conn.execute(
            "INSERT INTO groups (id, name, sort_order) VALUES (?1, ?2, ?3)",
            params![group.id, group.name, group_sort],
        ).map_err(|e| e.to_string())?;

        for (ci, comp) in group.comps.iter().enumerate() {
            let comp_sort = ((ci as i64) + 1) * SORT_GAP;
            conn.execute(
                "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
                params![comp.id, group.id, comp.name, comp_sort],
            ).map_err(|e| e.to_string())?;

            for (ti, track) in comp.tracks.iter().enumerate() {
                let track_sort = ((ti as i64) + 1) * SORT_GAP;
                let (mode_type, repeat, transition_type, gap_min, gap_max) = parse_old_mode(&track.mode);

                conn.execute(
                    "INSERT INTO tracks (id, comp_id, name, volume, mode_type, repeat, transition_type,
                     transition_gap_min, transition_gap_max, fade_in, fade_out, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        track.id, comp.id, track.name, track.volume,
                        mode_type, repeat, transition_type, gap_min, gap_max,
                        track.fade_in, track.fade_out, track_sort,
                    ],
                ).map_err(|e| e.to_string())?;

                // If seamless mode had loop points, store them on the library entry
                if mode_type == "seamless" {
                    if let Some(clip) = track.clips.first() {
                        if let Some(entry_id) = file_path_to_entry_id.get(&clip.file_path) {
                            let ls = track.mode.get("loopStart").and_then(|v| v.as_f64());
                            let le = track.mode.get("loopEnd").and_then(|v| v.as_f64());
                            if ls.is_some() || le.is_some() {
                                library::update_library_entry_loop(conn, entry_id, ls, le)?;
                            }
                        }
                    }
                }

                for (cli, clip) in track.clips.iter().enumerate() {
                    let clip_sort = ((cli as i64) + 1) * SORT_GAP;
                    let entry_id = file_path_to_entry_id
                        .get(&clip.file_path)
                        .ok_or_else(|| format!("No library entry for clip {}", clip.id))?;

                    conn.execute(
                        "INSERT INTO clips (id, track_id, library_entry_id, volume, sort_order)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![clip.id, track.id, entry_id, clip.volume, clip_sort],
                    ).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    Ok((old_state.groups.len(), entries_created, files_missing))
}

fn hash_file(path: &str) -> (Option<String>, bool) {
    let file_path = Path::new(path);
    if !file_path.exists() {
        return (None, true);
    }

    let mut file = match fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return (None, true),
    };

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return (None, true),
        }
    }

    let hash = format!("{:x}", hasher.finalize());
    (Some(hash), false)
}

fn parse_old_mode(mode_json: &serde_json::Value) -> (String, Option<i32>, Option<String>, Option<f64>, Option<f64>) {
    let mode_type = mode_json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("sequence")
        .to_lowercase();

    let repeat = mode_json.get("repeat").and_then(|v| v.as_bool()).map(|b| b as i32);

    let transition = mode_json.get("transition");
    let transition_type = transition
        .and_then(|t| t.get("type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());

    let gap_min = transition.and_then(|t| t.get("min")).and_then(|v| v.as_f64());
    let gap_max = transition.and_then(|t| t.get("max")).and_then(|v| v.as_f64());

    (mode_type, repeat, transition_type, gap_min, gap_max)
}

/// Rename the old JSON file to a backup.
pub fn backup_json(json_path: &Path) -> Result<(), String> {
    let backup_path = json_path.with_extension("backup.json");
    fs::rename(json_path, &backup_path)
        .map_err(|e| format!("Failed to backup JSON file: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_memory_db;

    fn sample_json() -> &'static str {
        r#"{
            "groups": [{
                "id": "g1",
                "name": "Exploration",
                "comps": [{
                    "id": "c1",
                    "name": "Tavern",
                    "tracks": [{
                        "id": "t1",
                        "name": "Ambience",
                        "clips": [{
                            "id": "cl1",
                            "name": "Tavern Music",
                            "filePath": "/nonexistent/tavern.mp3",
                            "volume": 0.8
                        }],
                        "volume": 1.0,
                        "mode": {"type": "sequence", "repeat": true, "transition": {"type": "gapless"}},
                        "fadeIn": 2.0,
                        "fadeOut": 1.5
                    }]
                }]
            }],
            "lastSavedAt": "1234567890"
        }"#
    }

    #[test]
    fn parse_old_json() {
        let old: OldAppState = serde_json::from_str(sample_json()).unwrap();
        assert_eq!(old.groups.len(), 1);
        assert_eq!(old.groups[0].comps[0].tracks[0].clips[0].name, "Tavern Music");
    }

    #[test]
    fn migrate_with_missing_files() {
        let conn = open_memory_db();
        let dir = std::env::temp_dir().join("lutedrop_migration_test");
        let _ = fs::create_dir_all(&dir);
        let json_path = dir.join("test-data.json");
        fs::write(&json_path, sample_json()).unwrap();

        assert!(needs_migration(&conn, &json_path));

        let (groups, entries, missing) = migrate_from_json(&conn, &json_path).unwrap();
        assert_eq!(groups, 1);
        assert_eq!(entries, 1);
        assert_eq!(missing, 1); // file doesn't exist

        // Verify hierarchy was created
        let group_count: i64 = conn.query_row("SELECT COUNT(*) FROM groups", [], |r| r.get(0)).unwrap();
        assert_eq!(group_count, 1);

        let clip_count: i64 = conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get(0)).unwrap();
        assert_eq!(clip_count, 1);

        // Verify library entry exists (with null hash since file missing)
        let entry_hash: Option<String> = conn.query_row(
            "SELECT content_hash FROM library_entries LIMIT 1", [], |r| r.get(0)
        ).unwrap();
        assert!(entry_hash.is_none());

        // Should not need migration again
        assert!(!needs_migration(&conn, &json_path));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_old_mode_variants() {
        let seamless = serde_json::json!({"type": "seamless", "loopStart": 10.0, "loopEnd": 50.0});
        let (mt, rep, tt, _, _) = parse_old_mode(&seamless);
        assert_eq!(mt, "seamless");
        assert!(rep.is_none());
        assert!(tt.is_none());

        let gap = serde_json::json!({"type": "sequence", "repeat": false, "transition": {"type": "gap", "min": 1.0, "max": 3.0}});
        let (mt, rep, tt, gmin, gmax) = parse_old_mode(&gap);
        assert_eq!(mt, "sequence");
        assert_eq!(rep, Some(0));
        assert_eq!(tt, Some("gap".to_string()));
        assert!((gmin.unwrap() - 1.0).abs() < 0.001);
        assert!((gmax.unwrap() - 3.0).abs() < 0.001);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cd src-tauri && cargo test database::migration::tests -v`

Expected: All 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/database/migration.rs
git commit -m "feat: JSON-to-SQLite migration with dedup and missing file handling"
```

---

## Task 7: Swap persistence layer

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/persistence.rs`

- [ ] **Step 1: Update AppStateContainer to hold a SQLite connection**

Replace `src-tauri/src/state.rs`:

```rust
use std::sync::Mutex;
use rusqlite::Connection;
use tokio::sync::broadcast;
use crate::audio::AudioEngine;
use crate::waveform::PeakCache;

pub struct AppStateContainer {
    pub db: Mutex<Connection>,
    pub audio: Mutex<AudioEngine>,
    pub broadcast_tx: broadcast::Sender<String>,
    pub peak_cache: PeakCache,
}

impl AppStateContainer {
    pub fn new(db: Connection, audio: AudioEngine, peak_cache_dir: std::path::PathBuf) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            db: Mutex::new(db),
            audio: Mutex::new(audio),
            broadcast_tx,
            peak_cache: PeakCache::new(peak_cache_dir),
        }
    }
}
```

- [ ] **Step 2: Trim persistence.rs to just the path helper (needed for migration)**

Replace `src-tauri/src/persistence.rs`:

```rust
use std::path::PathBuf;
use tauri::Manager;

/// Returns the path to the legacy JSON data file (used for migration detection).
pub fn json_data_file_path(app: &tauri::AppHandle) -> PathBuf {
    let app_data = app
        .path()
        .app_data_dir()
        .expect("failed to get app data dir");
    std::fs::create_dir_all(&app_data).ok();
    app_data.join("lutedrop-data.json")
}

/// Returns the path to the SQLite database file.
pub fn db_file_path(app: &tauri::AppHandle) -> PathBuf {
    let app_data = app
        .path()
        .app_data_dir()
        .expect("failed to get app data dir");
    std::fs::create_dir_all(&app_data).ok();
    app_data.join("lutedrop.db")
}
```

- [ ] **Step 3: Update lib.rs to initialize SQLite and run migration**

Replace `src-tauri/src/lib.rs`:

```rust
mod actions;
mod audio;
mod commands;
mod database;
mod models;
mod persistence;
mod server;
mod state;
mod waveform;

use std::sync::Arc;
use std::time::Duration;

use tauri::Emitter;
use tauri::Manager;
use state::AppStateContainer;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let json_path = persistence::json_data_file_path(&app.handle());
            let db_path = persistence::db_file_path(&app.handle());
            let peak_cache_dir = db_path.parent().unwrap_or(db_path.as_ref()).join("peaks");

            // Open (or create) the SQLite database
            let conn = database::open_db(&db_path)
                .expect("Failed to open database");

            // Run migration from JSON if needed
            if database::migration::needs_migration(&conn, &json_path) {
                log::info!("Migrating from JSON to SQLite...");
                match database::migration::migrate_from_json(&conn, &json_path) {
                    Ok((groups, entries, missing)) => {
                        log::info!(
                            "Migration complete: {} groups, {} library entries, {} missing files",
                            groups, entries, missing
                        );
                        if let Err(e) = database::migration::backup_json(&json_path) {
                            log::warn!("Failed to backup JSON: {}", e);
                        }
                    }
                    Err(e) => {
                        log::error!("Migration failed: {}", e);
                        // Continue with empty database rather than crashing
                    }
                }
            }

            let audio_engine = audio::AudioEngine::new()
                .expect("Failed to initialize audio engine");

            let container = Arc::new(AppStateContainer::new(conn, audio_engine, peak_cache_dir));

            app.manage(container.clone());

            // Start the axum WebSocket server
            let server_container = container.clone();
            let remote_dist = std::env::current_dir()
                .unwrap_or_default()
                .join("dist-remote")
                .to_string_lossy()
                .to_string();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create tokio runtime for server");
                rt.block_on(server::start_server(server_container, remote_dist));
            });

            // Spawn the playback state polling task
            let poll_container = container.clone();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                playback_polling_loop(app_handle, poll_container);
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::load_data,
            commands::add_group,
            commands::delete_group,
            commands::rename_group,
            commands::add_comp,
            commands::rename_comp,
            commands::delete_comp,
            commands::add_track,
            commands::delete_track,
            commands::update_track,
            commands::add_clip,
            commands::delete_clip,
            commands::play_track,
            commands::play_track_from,
            commands::play_track_clip,
            commands::pause_track,
            commands::stop_track,
            commands::set_master_volume,
            commands::set_track_volume,
            commands::set_clip_volume,
            commands::activate_comp,
            commands::stop_comp,
            commands::stop_all,
            commands::pause_all,
            commands::resume_all,
            commands::get_waveform_data,
            commands::get_server_info,
            // New library commands (added in Task 9)
        ])
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}

fn playback_polling_loop(app_handle: tauri::AppHandle, container: Arc<AppStateContainer>) {
    loop {
        std::thread::sleep(Duration::from_millis(50));

        let mut audio = match container.audio.lock() {
            Ok(a) => a,
            Err(_) => continue,
        };

        let crossfades = audio.check_crossfade_triggers();
        for (track_id, next_idx, duration) in crossfades {
            if let Err(e) = audio.begin_crossfade(&track_id, next_idx, duration) {
                log::warn!("Failed to start crossfade for track {}: {}", track_id, e);
            }
        }

        let finished = audio.check_finished_sounds();
        for (track_id, next_clip_index) in finished {
            if let Err(e) = audio.advance_track(&track_id, next_clip_index) {
                log::warn!("Failed to advance track {}: {}", track_id, e);
            }
        }

        let states = audio.get_all_playback_states();
        let master_vol = audio.get_master_volume();
        drop(audio);

        for state in &states {
            let _ = app_handle.emit("playback-state-update", state);
        }
        let _ = app_handle.emit("master-volume-update", master_vol);

        server::broadcast_state(&container);
    }
}
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/persistence.rs
git commit -m "feat: swap persistence from JSON to SQLite connection"
```

---

## Task 8: Rewrite actions.rs for SQLite

**Files:**
- Modify: `src-tauri/src/actions.rs`

- [ ] **Step 1: Rewrite all data mutation actions to use database operations**

Replace `src-tauri/src/actions.rs`:

```rust
use crate::database::{self, hierarchy, library};
use crate::models::{AppState, Track};
use crate::state::AppStateContainer;

// --- Helpers ---

fn load_state(container: &AppStateContainer) -> Result<AppState, String> {
    let conn = container.db.lock().map_err(|e| e.to_string())?;
    database::load_full_state(&conn)
}

fn with_db<F, T>(container: &AppStateContainer, f: F) -> Result<T, String>
where
    F: FnOnce(&rusqlite::Connection) -> Result<T, String>,
{
    let conn = container.db.lock().map_err(|e| e.to_string())?;
    f(&conn)
}

// --- Data mutation actions (return updated AppState) ---

pub fn load_data(container: &AppStateContainer) -> Result<AppState, String> {
    load_state(container)
}

pub fn add_group(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_group(conn, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_group(container: &AppStateContainer, group_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_group(conn, group_id)?;
        database::load_full_state(conn)
    })
}

pub fn rename_group(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_group(conn, group_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn rename_comp(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_comp(conn, comp_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn add_comp(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_comp(conn, group_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_comp(container: &AppStateContainer, _group_id: &str, comp_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_comp(conn, comp_id)?;
        database::load_full_state(conn)
    })
}

pub fn add_track(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_track(conn, comp_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_track(container: &AppStateContainer, _comp_id: &str, track_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_track(conn, track_id)?;
        database::load_full_state(conn)
    })
}

pub fn update_track(container: &AppStateContainer, _comp_id: &str, track_id: &str, updates: serde_json::Value) -> Result<AppState, String> {
    with_db(container, |conn| {
        let obj = updates.as_object()
            .ok_or_else(|| "updates must be a JSON object".to_string())?;

        if let Some(v) = obj.get("name") {
            let name = v.as_str().ok_or("name must be a string")?;
            hierarchy::rename_track(conn, track_id, name)?;
        }
        if let Some(v) = obj.get("volume") {
            let vol = v.as_f64().ok_or("volume must be a number")?;
            hierarchy::update_track_volume(conn, track_id, vol)?;
        }
        if let Some(v) = obj.get("fadeIn") {
            let fi = v.as_f64().ok_or("fadeIn must be a number")?;
            let fo_val: f64 = conn.query_row(
                "SELECT fade_out FROM tracks WHERE id = ?1", rusqlite::params![track_id], |r| r.get(0)
            ).map_err(|e| e.to_string())?;
            hierarchy::update_track_fades(conn, track_id, fi, fo_val)?;
        }
        if let Some(v) = obj.get("fadeOut") {
            let fo = v.as_f64().ok_or("fadeOut must be a number")?;
            let fi_val: f64 = conn.query_row(
                "SELECT fade_in FROM tracks WHERE id = ?1", rusqlite::params![track_id], |r| r.get(0)
            ).map_err(|e| e.to_string())?;
            hierarchy::update_track_fades(conn, track_id, fi_val, fo)?;
        }
        if let Some(v) = obj.get("mode") {
            let mode = v.as_object().ok_or("mode must be an object")?;
            let mode_type = mode.get("type").and_then(|v| v.as_str()).unwrap_or("sequence").to_lowercase();
            let repeat = mode.get("repeat").and_then(|v| v.as_bool());
            let transition = mode.get("transition");
            let tt = transition.and_then(|t| t.get("type")).and_then(|v| v.as_str()).map(|s| s.to_lowercase());
            let gmin = transition.and_then(|t| t.get("min")).and_then(|v| v.as_f64());
            let gmax = transition.and_then(|t| t.get("max")).and_then(|v| v.as_f64());

            hierarchy::update_track_mode(conn, track_id, &mode_type, repeat, tt.as_deref(), gmin, gmax)?;
        }
        if let Some(v) = obj.get("clips") {
            // Clip reorder: receives array of clip objects in new order
            let clips = v.as_array().ok_or("clips must be an array")?;
            for (i, clip_val) in clips.iter().enumerate() {
                let clip_id = clip_val.get("id").and_then(|v| v.as_str()).ok_or("clip must have id")?;
                // Get the track's comp_id to pass to reorder — but we actually just need track_id
                hierarchy::reorder_clip(conn, track_id, clip_id, i)?;
            }
        }

        database::load_full_state(conn)
    })
}

pub fn add_clip(container: &AppStateContainer, track_id: &str, library_entry_id: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_clip(conn, track_id, &library_entry_id)?;
        database::load_full_state(conn)
    })
}

pub fn delete_clip(container: &AppStateContainer, _track_id: &str, clip_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_clip(conn, clip_id)?;
        database::load_full_state(conn)
    })
}

// --- Library actions ---

pub fn import_library_entry(
    container: &AppStateContainer,
    file_path: String,
    content_hash: Option<String>,
    name: String,
    duration: Option<f64>,
    sample_rate: Option<u32>,
    file_size: Option<u64>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::insert_library_entry(conn, &file_path, content_hash.as_deref(), &name, duration, sample_rate, file_size)?;
        database::load_full_state(conn)
    })
}

pub fn update_library_entry_tags(
    container: &AppStateContainer,
    entry_id: &str,
    tags: Vec<String>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        // Remove all existing tags, then add the new set
        let existing = library::get_tags_for_entry(conn, entry_id)?;
        for tag in &existing {
            library::remove_tag_from_entry(conn, entry_id, tag)?;
        }
        for tag in &tags {
            library::add_tag_to_entry(conn, entry_id, tag)?;
        }
        database::load_full_state(conn)
    })
}

pub fn update_library_entry_loop_points(
    container: &AppStateContainer,
    entry_id: &str,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_loop(conn, entry_id, loop_start, loop_end)?;
        database::load_full_state(conn)
    })
}

pub fn update_library_entry_volume_action(
    container: &AppStateContainer,
    entry_id: &str,
    volume: f64,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_volume(conn, entry_id, volume)?;
        database::load_full_state(conn)
    })
}

pub fn rename_library_entry(
    container: &AppStateContainer,
    entry_id: &str,
    name: String,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_name(conn, entry_id, &name)?;
        database::load_full_state(conn)
    })
}

pub fn delete_library_entry_action(
    container: &AppStateContainer,
    entry_id: &str,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::delete_library_entry(conn, entry_id)?;
        database::load_full_state(conn)
    })
}

pub fn relocate_library_entry(
    container: &AppStateContainer,
    entry_id: &str,
    new_file_path: String,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_file_path(conn, entry_id, &new_file_path)?;
        database::load_full_state(conn)
    })
}

// --- Audio actions (no state return, unchanged pattern) ---

pub fn find_track_ref(container: &AppStateContainer, track_id: &str) -> Result<Track, String> {
    let state = load_state(container)?;
    for group in &state.groups {
        for comp in &group.comps {
            if let Some(track) = comp.tracks.iter().find(|t| t.id == track_id) {
                return Ok(track.clone());
            }
        }
    }
    Err(format!("Track not found: {}", track_id))
}

pub fn find_comp_track_ids(container: &AppStateContainer, comp_id: &str) -> Result<Vec<Track>, String> {
    let state = load_state(container)?;
    for group in &state.groups {
        if let Some(comp) = group.comps.iter().find(|c| c.id == comp_id) {
            return Ok(comp.tracks.clone());
        }
    }
    Err(format!("Comp not found: {}", comp_id))
}

pub fn play_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track(track_id)
}

pub fn play_track_clip(container: &AppStateContainer, track_id: &str, clip_id: &str) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track_clip(track_id, clip_id)
}

pub fn play_track_from_position(container: &AppStateContainer, track_id: &str, position: f64) -> Result<(), String> {
    let track_config = find_track_ref(container, track_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    if !audio.has_track(track_id) {
        audio.setup_track(&track_config)?;
    } else {
        audio.update_track_config(&track_config);
    }
    audio.play_track_from(track_id, position)
}

pub fn pause_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.pause_track(track_id)
}

pub fn stop_track_audio(container: &AppStateContainer, track_id: &str) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_track(track_id)
}

pub fn set_master_volume_action(container: &AppStateContainer, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_master_volume(volume);
    Ok(())
}

pub fn set_track_volume_action(container: &AppStateContainer, track_id: &str, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_track_volume(track_id, volume)
}

pub fn set_clip_volume_action(container: &AppStateContainer, clip_id: &str, volume: f64) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.set_clip_volume(clip_id, volume)
}

pub fn activate_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_track_ids(container, comp_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track in &track_configs {
        if !audio.has_track(&track.id) {
            audio.setup_track(track)?;
        } else {
            audio.update_track_config(track);
        }
        audio.play_track(&track.id)?;
    }
    Ok(())
}

pub fn stop_comp_audio(container: &AppStateContainer, comp_id: &str) -> Result<(), String> {
    let track_configs = find_comp_track_ids(container, comp_id)?;
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    for track in &track_configs {
        if audio.has_track(&track.id) {
            audio.stop_track(&track.id)?;
        }
    }
    Ok(())
}

pub fn stop_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_all();
    Ok(())
}

pub fn pause_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.pause_all();
    Ok(())
}

pub fn resume_all_audio(container: &AppStateContainer) -> Result<(), String> {
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    audio.resume_all();
    Ok(())
}
```

- [ ] **Step 2: Update commands.rs for new action signatures**

The main change: `add_clip` now takes `library_entry_id` instead of `name` + `file_path`. Update `src-tauri/src/commands.rs`:

Replace the `add_clip` command:

```rust
#[tauri::command]
pub fn add_clip(state: State<'_, Arc<AppStateContainer>>, track_id: String, library_entry_id: String) -> Result<AppState, String> {
    actions::add_clip(&state, &track_id, library_entry_id)
}
```

Add new library commands at the end of the file:

```rust
// --- Library commands ---

#[tauri::command]
pub fn import_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    file_path: String,
    content_hash: Option<String>,
    name: String,
    duration: Option<f64>,
    sample_rate: Option<u32>,
    file_size: Option<u64>,
) -> Result<AppState, String> {
    actions::import_library_entry(&state, file_path, content_hash, name, duration, sample_rate, file_size)
}

#[tauri::command]
pub fn update_library_entry_tags(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    tags: Vec<String>,
) -> Result<AppState, String> {
    actions::update_library_entry_tags(&state, &entry_id, tags)
}

#[tauri::command]
pub fn update_library_entry_loop_points(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<AppState, String> {
    actions::update_library_entry_loop_points(&state, &entry_id, loop_start, loop_end)
}

#[tauri::command]
pub fn update_library_entry_volume(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    volume: f64,
) -> Result<AppState, String> {
    actions::update_library_entry_volume_action(&state, &entry_id, volume)
}

#[tauri::command]
pub fn rename_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    name: String,
) -> Result<AppState, String> {
    actions::rename_library_entry(&state, &entry_id, name)
}

#[tauri::command]
pub fn delete_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
) -> Result<AppState, String> {
    actions::delete_library_entry_action(&state, &entry_id)
}

#[tauri::command]
pub fn relocate_library_entry(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    new_file_path: String,
) -> Result<AppState, String> {
    actions::relocate_library_entry(&state, &entry_id, new_file_path)
}

#[tauri::command]
pub fn check_missing_files(
    state: State<'_, Arc<AppStateContainer>>,
) -> Result<Vec<String>, String> {
    let app_state = actions::load_data(&state)?;
    let missing: Vec<String> = app_state
        .library_entries
        .iter()
        .filter(|e| !std::path::Path::new(&e.file_path).exists())
        .map(|e| e.id.clone())
        .collect();
    Ok(missing)
}
```

- [ ] **Step 3: Register new commands in lib.rs invoke_handler**

Add to the `invoke_handler` macro call in `lib.rs`:

```rust
commands::import_library_entry,
commands::update_library_entry_tags,
commands::update_library_entry_loop_points,
commands::update_library_entry_volume,
commands::rename_library_entry,
commands::delete_library_entry,
commands::relocate_library_entry,
commands::check_missing_files,
```

- [ ] **Step 4: Update server.rs for new AppState shape**

In `src-tauri/src/server.rs`, the `build_snapshot` function reads from `container.data`. It now needs to read from the database. Update `build_snapshot`:

```rust
fn build_snapshot(container: &AppStateContainer) -> Option<StateSnapshot> {
    let app_state = {
        let conn = container.db.lock().ok()?;
        crate::database::load_full_state(&conn).ok()?
    };
    let audio = container.audio.lock().ok()?;
    let playback_states = audio.get_all_playback_states();
    let master_volume = audio.get_master_volume();
    Some(StateSnapshot {
        r#type: "state".to_string(),
        app_state,
        playback_states,
        master_volume,
    })
}
```

Also update `handle_remote_command` — the `play_track` etc. commands call `actions::` functions which no longer take `data.read()` locks. Since the action functions now handle their own DB access, the remote command handler should work with minimal changes. Review the `handle_remote_command` function and ensure all action calls match the new signatures.

- [ ] **Step 5: Verify the Rust backend compiles**

Run: `cd src-tauri && cargo check`

Expected: No errors. Warnings about unused imports are OK.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/actions.rs src-tauri/src/commands.rs src-tauri/src/lib.rs src-tauri/src/server.rs
git commit -m "feat: rewrite actions and commands for SQLite persistence"
```

---

## Task 9: Update audio engine volume chain

**Files:**
- Modify: `src-tauri/src/audio/engine.rs`

The audio engine reads `clip.volume` and `track.volume` to compute playback volume. With the library, the volume chain is `library_entry.volume * clip.volume * track.volume * master_volume`. The library entry volume needs to be factored in.

- [ ] **Step 1: Pass library entry volume through the Clip model**

The `Clip` struct is already resolved from the library entry when loaded. The simplest approach: add a `library_volume` field to `Clip` so the engine can compute the full chain.

In `src-tauri/src/models.rs`, add to the `Clip` struct:

```rust
pub struct Clip {
    pub id: String,
    pub library_entry_id: String,
    pub name: String,
    pub file_path: String,
    pub volume: f64,
    pub library_volume: f64,
}
```

- [ ] **Step 2: Populate library_volume in load_clips**

In `src-tauri/src/database/mod.rs`, update the `load_clips` function's map closure:

```rust
.map(|(clip_id, entry_id, volume)| {
    let (name, file_path, library_volume) = entry_map
        .get(&entry_id)
        .map(|e| (e.name.clone(), e.file_path.clone(), e.volume))
        .unwrap_or_else(|| ("[missing]".to_string(), String::new(), 1.0));

    Clip {
        id: clip_id,
        library_entry_id: entry_id,
        name,
        file_path,
        volume,
        library_volume,
    }
})
```

- [ ] **Step 3: Update engine volume calculation**

In `src-tauri/src/audio/engine.rs`, find the line in `start_clip_inner` that computes combined volume:

```rust
let combined_volume = self.master_volume * active.config.volume * clip.volume;
```

Replace with:

```rust
let combined_volume = self.master_volume * active.config.volume * clip.volume * clip.library_volume;
```

Do the same in `update_sound_volume` if it exists, and in `begin_crossfade` where volume is computed for the incoming clip.

- [ ] **Step 4: Verify compilation**

Run: `cd src-tauri && cargo check`

Expected: Clean compilation.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/models.rs src-tauri/src/database/mod.rs src-tauri/src/audio/engine.rs
git commit -m "feat: include library entry volume in audio playback chain"
```

---

## Task 10: Regenerate TypeScript types and update Pinia store

**Files:**
- Regenerate: `generated/*.ts`
- Modify: `src/stores/appStore.ts`
- Modify: `generated/index.ts`

- [ ] **Step 1: Regenerate TypeScript types**

Run: `cd src-tauri && cargo test export_bindings`

Or manually build to trigger ts-rs exports: `cd src-tauri && cargo build`

Then check `generated/` for new files: `LibraryEntry.ts` should appear. Update `generated/index.ts` to re-export it:

```typescript
export type { AppState } from "./AppState";
export type { Clip } from "./Clip";
export type { Comp } from "./Comp";
export type { Group } from "./Group";
export type { LibraryEntry } from "./LibraryEntry";
export type { Track } from "./Track";
export type { TrackMode } from "./TrackMode";
export type { TrackPlaybackState } from "./TrackPlaybackState";
export type { TrackTransition } from "./TrackTransition";

export type CompStatus = "active" | "partial" | "inactive";
```

- [ ] **Step 2: Update Pinia store**

Modify `src/stores/appStore.ts`:

```typescript
import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Group, Comp, Track, AppState, TrackPlaybackState, CompStatus, LibraryEntry } from '@generated/index';
import { useToast } from '../composables/useToast';

export const useAppStore = defineStore('app', () => {
  const groups = ref<Group[]>([]);
  const libraryEntries = ref<LibraryEntry[]>([]);
  const tags = ref<string[]>([]);
  const missingEntryIds = ref<Set<string>>(new Set());
  const playbackStates = ref<Map<string, TrackPlaybackState>>(new Map());

  function hydrate(state: AppState): void {
    groups.value = state.groups;
    libraryEntries.value = state.libraryEntries;
    tags.value = state.tags;
  }

  async function command(cmd: string, args?: Record<string, unknown>): Promise<void> {
    try {
      const state = await invoke<AppState>(cmd, args);
      hydrate(state);
    } catch (e) {
      const toast = useToast();
      toast.showError(String(e));
    }
  }

  async function checkMissingFiles(): Promise<void> {
    try {
      const missing = await invoke<string[]>('check_missing_files');
      missingEntryIds.value = new Set(missing);
    } catch (e) {
      console.error('Failed to check missing files:', e);
    }
  }

  function setTrackPlaybackState(trackId: string, state: TrackPlaybackState): void {
    playbackStates.value.set(trackId, state);
  }

  function clearTrackPlaybackState(trackId: string): void {
    playbackStates.value.delete(trackId);
  }

  function getCompStatus(compId: string): CompStatus {
    const comp = findComp(compId);
    if (!comp || comp.tracks.length === 0) return 'inactive';

    const playing = comp.tracks.filter((t) => {
      const state = playbackStates.value.get(t.id);
      return state?.isPlaying;
    });

    if (playing.length === 0) return 'inactive';
    if (playing.length === comp.tracks.length) return 'active';
    return 'partial';
  }

  function findComp(compId: string): Comp | undefined {
    for (const group of groups.value) {
      const comp = group.comps.find((c) => c.id === compId);
      if (comp) return comp;
    }
    return undefined;
  }

  function findTrack(trackId: string): Track | undefined {
    for (const group of groups.value) {
      for (const comp of group.comps) {
        const track = comp.tracks.find((t) => t.id === trackId);
        if (track) return track;
      }
    }
    return undefined;
  }

  function findLibraryEntry(entryId: string): LibraryEntry | undefined {
    return libraryEntries.value.find((e) => e.id === entryId);
  }

  function isEntryMissing(entryId: string): boolean {
    return missingEntryIds.value.has(entryId);
  }

  return {
    groups,
    libraryEntries,
    tags,
    missingEntryIds,
    playbackStates,
    hydrate,
    command,
    checkMissingFiles,
    setTrackPlaybackState,
    clearTrackPlaybackState,
    getCompStatus,
    findComp,
    findTrack,
    findLibraryEntry,
    isEntryMissing,
  };
});
```

- [ ] **Step 3: Call checkMissingFiles on startup**

In `src/App.vue`, in the `onMounted` callback (where `load_data` is called), add after hydration:

```typescript
store.checkMissingFiles();
```

- [ ] **Step 4: Verify the frontend compiles**

Run: `npm run type-check` (or `npx vue-tsc --noEmit`)

Expected: Type errors in components that reference old `Clip.filePath` or `Clip.name` directly. This is expected — Task 11 fixes those.

- [ ] **Step 5: Commit**

```bash
git add generated/ src/stores/appStore.ts src/App.vue
git commit -m "feat: update TypeScript types and Pinia store for library support"
```

---

## Task 11: Update existing Vue components

**Files:**
- Modify: `src/components/TrackClipList.vue`
- Modify: `src/components/ClipRow.vue`
- Modify: `src/components/TrackSettings.vue`
- Modify: `src/components/WaveformEditor.vue`
- Modify: `src/composables/useTrackUpdates.ts`

- [ ] **Step 1: Update TrackClipList — replace file picker with library browser placeholder**

In `src/components/TrackClipList.vue`, the `addClips` function currently opens a file dialog and calls `add_clip` with `name` and `filePath`. Replace it to open a library browser dialog (full implementation in Task 12). For now, use a temporary approach that still works:

Replace the `addClips` function:

```typescript
const emit = defineEmits<{
  'open-library-browser': [trackId: string];
}>();

function addClips(): void {
  emit('open-library-browser', props.track.id);
}
```

Remove the `open` import from `@tauri-apps/plugin-dialog`.

- [ ] **Step 2: Update ClipRow — clip name comes from resolved data**

The `Clip` type now has `libraryEntryId`, `name`, and `filePath` (resolved from library entry). `ClipRow.vue` already reads `clip.name` and that still works since the backend resolves it. No changes needed to ClipRow display logic.

However, update the volume command — clip volume is now per-clip, library volume is separate. The existing `set_clip_volume` command still works for per-clip volume. No changes needed.

- [ ] **Step 3: Update TrackSettings — loop point edits go to library entry**

In `src/composables/useTrackUpdates.ts`, the `updateLoopStart`, `updateLoopEnd`, `updateLoopStartFromWaveform`, `updateLoopEndFromWaveform` methods currently call `updateField` to update the track's mode. They now need to call `update_library_entry_loop_points` instead.

Update these methods:

```typescript
async function updateLoopStart(seconds: number): Promise<void> {
  const entryId = getSeamlessClipEntryId();
  if (!entryId) return;
  await store.command('update_library_entry_loop_points', {
    entryId,
    loopStart: seconds,
    loopEnd: getCurrentLoopEnd(),
  });
}

async function updateLoopEnd(seconds: number): Promise<void> {
  const entryId = getSeamlessClipEntryId();
  if (!entryId) return;
  await store.command('update_library_entry_loop_points', {
    entryId,
    loopStart: getCurrentLoopStart(),
    loopEnd: seconds,
  });
}

// Same for waveform variants
async function updateLoopStartFromWaveform(seconds: number): Promise<void> {
  await updateLoopStart(seconds);
}

async function updateLoopEndFromWaveform(seconds: number): Promise<void> {
  await updateLoopEnd(seconds);
}
```

Add helpers:

```typescript
function getSeamlessClipEntryId(): string | undefined {
  const track = store.findTrack(trackIdFn());
  if (!track || track.clips.length === 0) return undefined;
  return track.clips[0].libraryEntryId;
}

function getCurrentLoopStart(): number {
  const track = store.findTrack(trackIdFn());
  if (!track || track.mode.type !== 'seamless') return 0;
  return track.mode.loopStart;
}

function getCurrentLoopEnd(): number {
  const track = store.findTrack(trackIdFn());
  if (!track || track.mode.type !== 'seamless') return 0;
  return track.mode.loopEnd;
}
```

- [ ] **Step 4: Verify the frontend compiles**

Run: `npx vue-tsc --noEmit`

Expected: Clean compilation, or minor issues to fix.

- [ ] **Step 5: Run the dev server and verify existing features work**

Run: `npm run tauri dev`

Test: Create a group, comp, track. The existing playback functionality should work with the new SQLite backend. Audio should play, stop, pause correctly. Waveforms should render.

- [ ] **Step 6: Commit**

```bash
git add src/components/ src/composables/
git commit -m "feat: update existing components for library-backed clips"
```

---

## Task 12: Library view and browser dialog

**Files:**
- Create: `src/views/LibraryView.vue`
- Create: `src/components/LibraryBrowser.vue`
- Create: `src/components/LibraryEntryRow.vue`
- Create: `src/components/TagFilter.vue`
- Modify: `src/App.vue`

- [ ] **Step 1: Create TagFilter component**

Create `src/components/TagFilter.vue`:

```vue
<script setup lang="ts">
import { computed } from 'vue';
import { useAppStore } from '../stores/appStore';

const props = defineProps<{
  selectedTags: string[];
}>();

const emit = defineEmits<{
  'update:selectedTags': [tags: string[]];
}>();

const store = useAppStore();

function toggleTag(tag: string): void {
  const current = [...props.selectedTags];
  const idx = current.indexOf(tag);
  if (idx >= 0) {
    current.splice(idx, 1);
  } else {
    current.push(tag);
  }
  emit('update:selectedTags', current);
}

function isSelected(tag: string): boolean {
  return props.selectedTags.includes(tag);
}
</script>

<template>
  <div class="d-flex flex-wrap ga-1">
    <VChip
      v-for="tag in store.tags"
      :key="tag"
      :color="isSelected(tag) ? 'primary' : undefined"
      :variant="isSelected(tag) ? 'flat' : 'outlined'"
      size="small"
      @click="toggleTag(tag)"
    >
      {{ tag }}
    </VChip>
    <span v-if="store.tags.length === 0" class="text-caption text-medium-emphasis">
      No tags yet
    </span>
  </div>
</template>
```

- [ ] **Step 2: Create LibraryEntryRow component**

Create `src/components/LibraryEntryRow.vue`:

```vue
<script setup lang="ts">
import type { LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';

const props = defineProps<{
  entry: LibraryEntry;
  selectable?: boolean;
  selected?: boolean;
}>();

const emit = defineEmits<{
  select: [entryId: string];
}>();

const store = useAppStore();
const isMissing = store.isEntryMissing(props.entry.id);

function formatDuration(seconds: number | null): string {
  if (seconds == null) return '--:--';
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}
</script>

<template>
  <div
    class="library-entry-row d-flex align-center pa-2 rounded"
    :class="{ 'bg-surface-light': selected, 'missing-entry': isMissing }"
    @click="selectable && emit('select', entry.id)"
  >
    <VIcon v-if="isMissing" color="warning" size="small" class="mr-2">mdi-alert</VIcon>
    <div class="flex-grow-1">
      <div class="text-body-2">{{ entry.name }}</div>
      <div class="d-flex ga-1 mt-1">
        <VChip v-for="tag in entry.tags" :key="tag" size="x-small" variant="outlined">{{ tag }}</VChip>
      </div>
    </div>
    <span class="text-caption text-medium-emphasis mr-4">{{ formatDuration(entry.duration) }}</span>
    <slot name="actions" />
  </div>
</template>

<style scoped>
.library-entry-row {
  cursor: default;
  border: 1px solid transparent;
}
.library-entry-row:hover {
  background: rgba(255, 255, 255, 0.03);
}
.missing-entry {
  opacity: 0.6;
}
</style>
```

- [ ] **Step 3: Create LibraryView**

Create `src/views/LibraryView.vue`:

```vue
<script setup lang="ts">
import { ref, computed } from 'vue';
import { useAppStore } from '../stores/appStore';
import LibraryEntryRow from '../components/LibraryEntryRow.vue';
import TagFilter from '../components/TagFilter.vue';
import { useLibraryImport } from '../composables/useLibraryImport';

const store = useAppStore();
const { importFiles, importFolder } = useLibraryImport();

const searchQuery = ref('');
const selectedTags = ref<string[]>([]);

const filteredEntries = computed(() => {
  let entries = store.libraryEntries;

  if (searchQuery.value) {
    const q = searchQuery.value.toLowerCase();
    entries = entries.filter((e) => e.name.toLowerCase().includes(q));
  }

  if (selectedTags.value.length > 0) {
    entries = entries.filter((e) =>
      selectedTags.value.every((tag) => e.tags.includes(tag))
    );
  }

  return entries;
});

// Tag editing
const editingEntryId = ref<string | null>(null);
const editingTags = ref<string>('');

function startEditTags(entryId: string, currentTags: string[]): void {
  editingEntryId.value = entryId;
  editingTags.value = currentTags.join(', ');
}

async function saveTags(): Promise<void> {
  if (!editingEntryId.value) return;
  const tags = editingTags.value
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
  await store.command('update_library_entry_tags', {
    entryId: editingEntryId.value,
    tags,
  });
  editingEntryId.value = null;
}

// Delete
async function deleteEntry(entryId: string): Promise<void> {
  await store.command('delete_library_entry', { entryId });
}
</script>

<template>
  <div class="library-view pa-4">
    <div class="d-flex align-center mb-4">
      <h2 class="text-h6">Library</h2>
      <VSpacer />
      <VBtn prepend-icon="mdi-file-music" variant="tonal" class="mr-2" @click="importFiles">
        Import Files
      </VBtn>
      <VBtn prepend-icon="mdi-folder-open" variant="tonal" @click="importFolder">
        Import Folder
      </VBtn>
    </div>

    <VTextField
      v-model="searchQuery"
      prepend-inner-icon="mdi-magnify"
      placeholder="Search library..."
      variant="outlined"
      density="compact"
      hide-details
      class="mb-3"
    />

    <TagFilter v-model:selectedTags="selectedTags" class="mb-4" />

    <div class="text-caption text-medium-emphasis mb-2">
      {{ filteredEntries.length }} entries
    </div>

    <div v-for="entry in filteredEntries" :key="entry.id">
      <LibraryEntryRow :entry="entry">
        <template #actions>
          <VBtn icon="mdi-tag-edit" variant="text" size="small" @click.stop="startEditTags(entry.id, entry.tags)" />
          <VBtn icon="mdi-delete" variant="text" size="small" @click.stop="deleteEntry(entry.id)" />
        </template>
      </LibraryEntryRow>
    </div>

    <div v-if="filteredEntries.length === 0" class="text-center text-medium-emphasis mt-8">
      <VIcon size="48">mdi-music-note-off</VIcon>
      <div class="mt-2">No entries found. Import some audio files to get started.</div>
    </div>

    <!-- Tag editing dialog -->
    <VDialog v-model="editingEntryId" max-width="400">
      <VCard>
        <VCardTitle>Edit Tags</VCardTitle>
        <VCardText>
          <VTextField
            v-model="editingTags"
            label="Tags (comma-separated)"
            hint="e.g. somber, tavern, peaceful"
            persistent-hint
            @keyup.enter="saveTags"
          />
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="editingEntryId = null">Cancel</VBtn>
          <VBtn color="primary" @click="saveTags">Save</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>
```

- [ ] **Step 4: Create LibraryBrowser dialog**

Create `src/components/LibraryBrowser.vue`:

```vue
<script setup lang="ts">
import { ref, computed } from 'vue';
import { useAppStore } from '../stores/appStore';
import LibraryEntryRow from './LibraryEntryRow.vue';
import TagFilter from './TagFilter.vue';

const props = defineProps<{
  modelValue: boolean;
  trackId: string;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const store = useAppStore();

const searchQuery = ref('');
const selectedTags = ref<string[]>([]);
const selectedEntryIds = ref<Set<string>>(new Set());

const filteredEntries = computed(() => {
  let entries = store.libraryEntries;
  if (searchQuery.value) {
    const q = searchQuery.value.toLowerCase();
    entries = entries.filter((e) => e.name.toLowerCase().includes(q));
  }
  if (selectedTags.value.length > 0) {
    entries = entries.filter((e) =>
      selectedTags.value.every((tag) => e.tags.includes(tag))
    );
  }
  return entries;
});

function toggleSelect(entryId: string): void {
  const s = new Set(selectedEntryIds.value);
  if (s.has(entryId)) {
    s.delete(entryId);
  } else {
    s.add(entryId);
  }
  selectedEntryIds.value = s;
}

async function addSelected(): Promise<void> {
  for (const entryId of selectedEntryIds.value) {
    await store.command('add_clip', { trackId: props.trackId, libraryEntryId: entryId });
  }
  selectedEntryIds.value = new Set();
  emit('update:modelValue', false);
}

function close(): void {
  selectedEntryIds.value = new Set();
  emit('update:modelValue', false);
}
</script>

<template>
  <VDialog :model-value="modelValue" max-width="600" @update:model-value="close">
    <VCard>
      <VCardTitle class="d-flex align-center">
        Add Clips from Library
        <VSpacer />
        <VChip v-if="selectedEntryIds.size > 0" color="primary" size="small">
          {{ selectedEntryIds.size }} selected
        </VChip>
      </VCardTitle>
      <VCardText>
        <VTextField
          v-model="searchQuery"
          prepend-inner-icon="mdi-magnify"
          placeholder="Search..."
          variant="outlined"
          density="compact"
          hide-details
          class="mb-3"
        />
        <TagFilter v-model:selectedTags="selectedTags" class="mb-3" />
        <div style="max-height: 400px; overflow-y: auto">
          <LibraryEntryRow
            v-for="entry in filteredEntries"
            :key="entry.id"
            :entry="entry"
            selectable
            :selected="selectedEntryIds.has(entry.id)"
            @select="toggleSelect"
          />
          <div v-if="filteredEntries.length === 0" class="text-center text-medium-emphasis pa-4">
            No entries match your search.
          </div>
        </div>
      </VCardText>
      <VCardActions>
        <VSpacer />
        <VBtn @click="close">Cancel</VBtn>
        <VBtn color="primary" :disabled="selectedEntryIds.size === 0" @click="addSelected">
          Add {{ selectedEntryIds.size }} Clip{{ selectedEntryIds.size === 1 ? '' : 's' }}
        </VBtn>
      </VCardActions>
    </VCard>
  </VDialog>
</template>
```

- [ ] **Step 5: Add Library navigation to App.vue**

In `src/App.vue`, add a tab or navigation item for the Library view. Add a `showLibrary` ref and toggle between PlaybackView and LibraryView:

Add to the template, next to the group tabs:

```vue
<VBtn
  variant="text"
  :color="showLibrary ? 'primary' : undefined"
  @click="showLibrary = !showLibrary"
>
  <VIcon>mdi-bookshelf</VIcon>
  Library
</VBtn>
```

Toggle the main content:

```vue
<LibraryView v-if="showLibrary" />
<PlaybackView v-else :active-tab="activeTab" />
```

- [ ] **Step 6: Wire LibraryBrowser into TrackClipList**

In the parent component that renders `TrackClipList` (likely `TrackControl.vue` or `CompDetails.vue`), add the `LibraryBrowser` dialog and handle the `open-library-browser` event from `TrackClipList`.

- [ ] **Step 7: Run the dev server and test**

Run: `npm run tauri dev`

Test:
- Library view shows (empty initially)
- Navigation between Library and Playback views works
- Library browser dialog opens when clicking "Add Clips" on a track

- [ ] **Step 8: Commit**

```bash
git add src/views/LibraryView.vue src/components/LibraryBrowser.vue src/components/LibraryEntryRow.vue src/components/TagFilter.vue src/App.vue src/components/TrackClipList.vue
git commit -m "feat: library view, browser dialog, and tag filter components"
```

---

## Task 13: Import flow with dedup and tagging

**Files:**
- Create: `src/composables/useLibraryImport.ts`
- Modify: `src/views/LibraryView.vue` (if not already wired)

- [ ] **Step 1: Create the import composable**

Create `src/composables/useLibraryImport.ts`:

```typescript
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type { AppState } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useToast } from './useToast';

const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];

export function useLibraryImport() {
  const store = useAppStore();
  const toast = useToast();

  async function importFiles(): Promise<void> {
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
    });
    if (!paths) return;
    const pathList = Array.isArray(paths) ? paths : [paths];
    await importPaths(pathList);
  }

  async function importFolder(): Promise<void> {
    // Open a file picker pre-set to folder selection — user selects all audio files from a folder
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
    });
    if (!paths) return;
    const pathList = Array.isArray(paths) ? paths : [paths];
    await importPaths(pathList);
  }

  async function importPaths(paths: string[]): Promise<void> {
    let imported = 0;
    let skipped = 0;

    for (const filePath of paths) {
      const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';

      try {
        // Hash the file on the backend side — invoke import which handles dedup
        const state = await invoke<AppState>('import_library_entry', {
          filePath,
          contentHash: null, // Backend will hash it (or we add a hash command)
          name: fileName,
          duration: null,
          sampleRate: null,
          fileSize: null,
        });
        store.hydrate(state);
        imported++;
      } catch (e) {
        const msg = String(e);
        if (msg.includes('UNIQUE constraint failed')) {
          skipped++;
        } else {
          toast.showError(`Failed to import ${fileName}: ${msg}`);
        }
      }
    }

    if (imported > 0) {
      toast.showSuccess(`Imported ${imported} file${imported === 1 ? '' : 's'}${skipped > 0 ? `, ${skipped} already in library` : ''}`);
    } else if (skipped > 0) {
      toast.showSuccess(`All ${skipped} file${skipped === 1 ? '' : 's'} already in library`);
    }
  }

  return { importFiles, importFolder };
}
```

- [ ] **Step 2: Add backend SHA-256 hashing during import**

Update the `import_library_entry` action in `src-tauri/src/actions.rs` to compute the hash if not provided:

```rust
pub fn import_library_entry(
    container: &AppStateContainer,
    file_path: String,
    content_hash: Option<String>,
    name: String,
    duration: Option<f64>,
    sample_rate: Option<u32>,
    file_size: Option<u64>,
) -> Result<AppState, String> {
    use sha2::{Sha256, Digest};
    use std::io::Read;

    // Compute hash if not provided
    let hash = match content_hash {
        Some(h) => Some(h),
        None => {
            let path = std::path::Path::new(&file_path);
            if path.exists() {
                let mut file = std::fs::File::open(path)
                    .map_err(|e| format!("Failed to open file: {}", e))?;
                let mut hasher = Sha256::new();
                let mut buf = [0u8; 8192];
                loop {
                    match file.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => hasher.update(&buf[..n]),
                        Err(e) => return Err(format!("Failed to read file: {}", e)),
                    }
                }
                Some(format!("{:x}", hasher.finalize()))
            } else {
                None
            }
        }
    };

    // Check for duplicate by hash
    if let Some(ref h) = hash {
        let conn = container.db.lock().map_err(|e| e.to_string())?;
        if let Some((_, existing_name)) = library::get_library_entry_by_hash(&conn, h)? {
            return Err(format!("Already in library as '{}'", existing_name));
        }
    }

    with_db(container, |conn| {
        library::insert_library_entry(conn, &file_path, hash.as_deref(), &name, duration, sample_rate, file_size)?;
        database::load_full_state(conn)
    })
}
```

- [ ] **Step 3: Run the dev server and test the full import flow**

Run: `npm run tauri dev`

Test:
- Open Library view, click "Import Files", select audio files
- Files appear in the library list
- Importing the same file again shows a "already in library" message
- Add tags to imported entries
- Use Library Browser from a track to add clips from the library
- Play the track — audio should play from the library entry's file path

- [ ] **Step 4: Commit**

```bash
git add src/composables/useLibraryImport.ts src-tauri/src/actions.rs
git commit -m "feat: import flow with SHA-256 dedup and library browser integration"
```

---

## Task 14: Broken reference detection and relocation

**Files:**
- Modify: `src/App.vue`
- Modify: `src/views/LibraryView.vue`

- [ ] **Step 1: Show missing file indicators in Library view**

The store already has `isEntryMissing()` and `LibraryEntryRow` already shows a warning icon for missing entries. The `checkMissingFiles()` call on startup populates the set.

Add a "Relocate" action to missing entries in `LibraryView.vue`:

```typescript
import { open } from '@tauri-apps/plugin-dialog';

async function relocateEntry(entryId: string): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'] }],
  });
  if (!path || Array.isArray(path)) return;
  await store.command('relocate_library_entry', { entryId, newFilePath: path });
  await store.checkMissingFiles();
}
```

Add a relocate button to missing entries in the template:

```vue
<VBtn
  v-if="store.isEntryMissing(entry.id)"
  icon="mdi-file-find"
  variant="text"
  size="small"
  title="Relocate file"
  @click.stop="relocateEntry(entry.id)"
/>
```

- [ ] **Step 2: Show a startup notification if files are missing**

In `src/App.vue`, after `checkMissingFiles` completes, show a toast if any files are missing:

```typescript
onMounted(async () => {
  await store.command('load_data');
  await store.checkMissingFiles();
  if (store.missingEntryIds.size > 0) {
    toast.showError(`${store.missingEntryIds.size} audio file(s) not found. Check the Library view.`);
  }
});
```

- [ ] **Step 3: Test broken reference flow**

Run: `npm run tauri dev`

Test:
- Import a file, then rename/move it on disk
- Restart the app
- Toast should show "X audio file(s) not found"
- Library view shows warning icon on the missing entry
- Click relocate, select the new file location
- Warning icon disappears

- [ ] **Step 4: Commit**

```bash
git add src/App.vue src/views/LibraryView.vue
git commit -m "feat: broken reference detection with startup check and relocate flow"
```

---

## Task 15: Delete old useTrackImport and clean up

**Files:**
- Delete: `src/composables/useTrackImport.ts`
- Modify: any remaining references

- [ ] **Step 1: Remove useTrackImport**

Delete `src/composables/useTrackImport.ts`. Search for any remaining imports of `useTrackImport` and remove them.

Run: `npx vue-tsc --noEmit` to verify no type errors.

- [ ] **Step 2: Final integration test**

Run: `npm run tauri dev`

Full test checklist:
- Create groups, comps, tracks (existing flow works)
- Import audio files into library
- Tag files with freeform tags
- Filter library by tags
- Add clips from library to tracks using the browser dialog
- Play tracks — audio plays correctly
- Loop points in seamless mode work (set from library entry)
- Volume chain works (library gain * clip * track * master)
- Reorder clips via drag-and-drop
- Delete a clip (library entry stays)
- Delete a library entry that's in use (blocked with error)
- Delete a library entry that's not in use (succeeds)
- Relocate a missing file
- Remote control still works (WebSocket state includes library data)

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "cleanup: remove old useTrackImport, finalize library migration"
```
