# DB Auto-Backup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automatically back up the SQLite database on startup and every 30 minutes, keeping the 5 most recent backups.

**Architecture:** A new `database::backup` module provides `perform_backup` (uses rusqlite's Backup API) and `prune_old_backups`. A background thread spawned in `lib.rs` runs backup-on-startup then loops every 30 minutes. `AppStateContainer` gains a `db_path` field so the backup thread knows where to write.

**Tech Stack:** rusqlite (backup module), chrono (timestamp formatting), std::fs, std::thread

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src-tauri/Cargo.toml` | Modify | Add `chrono` dependency |
| `src-tauri/src/state.rs` | Modify | Add `db_path: PathBuf` field to `AppStateContainer` |
| `src-tauri/src/database/mod.rs` | Modify | Add `pub mod backup;` |
| `src-tauri/src/database/backup.rs` | Create | `perform_backup`, `prune_old_backups`, backup loop, tests |
| `src-tauri/src/lib.rs` | Modify | Pass `db_path`, spawn backup thread |

---

### Task 1: Add `db_path` to `AppStateContainer`

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs:51`

- [ ] **Step 1: Add `db_path` field to `AppStateContainer`**

In `src-tauri/src/state.rs`, add `use std::path::PathBuf;` and add the field + constructor parameter:

```rust
use std::path::PathBuf;
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
    pub db_path: PathBuf,
}

impl AppStateContainer {
    pub fn new(db: Connection, audio: AudioEngine, peak_cache_dir: std::path::PathBuf, db_path: PathBuf) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            db: Mutex::new(db),
            audio: Mutex::new(audio),
            broadcast_tx,
            peak_cache: PeakCache::new(peak_cache_dir),
            db_path,
        }
    }
}
```

- [ ] **Step 2: Update `lib.rs` to pass `db_path` to constructor**

In `src-tauri/src/lib.rs`, change line 51 from:

```rust
let container = Arc::new(AppStateContainer::new(conn, audio_engine, peak_cache_dir));
```

to:

```rust
let container = Arc::new(AppStateContainer::new(conn, audio_engine, peak_cache_dir, db_path.clone()));
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10`
Expected: All 35 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs
git commit -m "refactor: add db_path field to AppStateContainer"
```

---

### Task 2: Add `chrono` dependency and create `backup.rs` with `prune_old_backups` (TDD)

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/database/mod.rs:1-2`
- Create: `src-tauri/src/database/backup.rs`

- [ ] **Step 1: Add `chrono` to Cargo.toml**

Add to the `[dependencies]` section of `src-tauri/Cargo.toml`:

```toml
chrono = "0.4"
```

- [ ] **Step 2: Add `pub mod backup;` to `database/mod.rs`**

Change the top of `src-tauri/src/database/mod.rs` from:

```rust
pub mod hierarchy;
pub mod library;
```

to:

```rust
pub mod backup;
pub mod hierarchy;
pub mod library;
```

- [ ] **Step 3: Create `backup.rs` with the test for `prune_old_backups`**

Create `src-tauri/src/database/backup.rs` with the prune function signature and test:

```rust
use std::fs;
use std::path::Path;

const BACKUP_PREFIX: &str = "lutedrop-backup-";
const BACKUP_EXT: &str = ".db";

/// Deletes all but the `keep` most recent backup files in `backup_dir`.
/// Files are sorted by filename (which sorts chronologically due to the
/// timestamp format YYYYMMDD-HHMMSS). Errors on individual deletes are
/// logged but do not stop pruning.
pub fn prune_old_backups(backup_dir: &Path, keep: usize) {
    let mut backups: Vec<_> = match fs::read_dir(backup_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with(BACKUP_PREFIX) && name.ends_with(BACKUP_EXT)
            })
            .collect(),
        Err(e) => {
            log::warn!("Failed to read backup directory: {e}");
            return;
        }
    };

    // Sort by filename ascending (chronological due to timestamp format).
    backups.sort_by_key(|e| e.file_name());

    if backups.len() <= keep {
        return;
    }

    let to_delete = backups.len() - keep;
    for entry in backups.iter().take(to_delete) {
        if let Err(e) = fs::remove_file(entry.path()) {
            log::warn!("Failed to delete old backup {:?}: {e}", entry.path());
        } else {
            log::info!("Pruned old backup: {:?}", entry.file_name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn prune_keeps_newest_backups() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let names = [
            "lutedrop-backup-20260401-100000.db",
            "lutedrop-backup-20260402-100000.db",
            "lutedrop-backup-20260403-100000.db",
            "lutedrop-backup-20260404-100000.db",
            "lutedrop-backup-20260405-100000.db",
            "lutedrop-backup-20260406-100000.db",
            "lutedrop-backup-20260407-100000.db",
        ];
        for name in &names {
            File::create(dir.path().join(name)).expect("create file");
        }

        prune_old_backups(dir.path(), 5);

        let remaining: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        assert_eq!(remaining.len(), 5);
        // The two oldest should be gone.
        assert!(!remaining.contains(&"lutedrop-backup-20260401-100000.db".to_string()));
        assert!(!remaining.contains(&"lutedrop-backup-20260402-100000.db".to_string()));
        // The five newest should remain.
        for name in &names[2..] {
            assert!(remaining.contains(&name.to_string()), "expected {name} to remain");
        }
    }

    #[test]
    fn prune_does_nothing_when_under_limit() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let names = [
            "lutedrop-backup-20260401-100000.db",
            "lutedrop-backup-20260402-100000.db",
        ];
        for name in &names {
            File::create(dir.path().join(name)).expect("create file");
        }

        prune_old_backups(dir.path(), 5);

        let count = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(count, 2);
    }

    #[test]
    fn prune_ignores_non_backup_files() {
        let dir = tempfile::tempdir().expect("create temp dir");
        // 6 backup files + 1 unrelated file
        for i in 1..=6 {
            File::create(dir.path().join(format!("lutedrop-backup-2026040{i}-100000.db")))
                .expect("create file");
        }
        File::create(dir.path().join("other-file.txt")).expect("create file");

        prune_old_backups(dir.path(), 5);

        let remaining: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        // 5 backups + 1 unrelated file
        assert_eq!(remaining.len(), 6);
        assert!(remaining.contains(&"other-file.txt".to_string()));
    }
}
```

- [ ] **Step 4: Add `tempfile` dev-dependency to Cargo.toml**

Add to `src-tauri/Cargo.toml`:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 5: Run the prune tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup -- --nocapture 2>&1 | tail -15`
Expected: 3 tests pass — `prune_keeps_newest_backups`, `prune_does_nothing_when_under_limit`, `prune_ignores_non_backup_files`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/database/mod.rs src-tauri/src/database/backup.rs
git commit -m "feat: add prune_old_backups with tests"
```

---

### Task 3: Add `perform_backup` to `backup.rs` (TDD)

**Files:**
- Modify: `src-tauri/src/database/backup.rs`

- [ ] **Step 1: Write the failing test for `perform_backup`**

Add to the `tests` module in `backup.rs`:

```rust
    #[test]
    fn perform_backup_creates_valid_copy() {
        use rusqlite::Connection;

        // Create an in-memory source DB with the app schema and some data.
        let src = crate::database::open_memory_db();
        crate::database::library::insert_library_entry(
            &src,
            "/tmp/test.mp3",
            Some("hash123"),
            "Test Sound",
            Some(60.0),
            Some(44100),
            Some(1024),
        )
        .expect("insert entry");

        let dir = tempfile::tempdir().expect("create temp dir");

        let backup_path = perform_backup(&src, dir.path()).expect("backup should succeed");

        // Open the backup and verify the data is there.
        let dst = Connection::open(&backup_path).expect("open backup");
        let name: String = dst
            .query_row(
                "SELECT name FROM library_entries WHERE file_path = '/tmp/test.mp3'",
                [],
                |row| row.get(0),
            )
            .expect("query backup");
        assert_eq!(name, "Test Sound");
    }

    #[test]
    fn perform_backup_prunes_to_five() {
        let src = crate::database::open_memory_db();
        let dir = tempfile::tempdir().expect("create temp dir");

        // Create 6 pre-existing backup files.
        for i in 1..=6 {
            File::create(dir.path().join(format!("lutedrop-backup-2026040{i}-100000.db")))
                .expect("create file");
        }

        // This 7th backup should trigger pruning down to 5.
        perform_backup(&src, dir.path()).expect("backup should succeed");

        let count = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with(BACKUP_PREFIX) && name.ends_with(BACKUP_EXT)
            })
            .count();
        assert_eq!(count, 5);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup -- --nocapture 2>&1 | tail -15`
Expected: FAIL — `perform_backup` not found.

- [ ] **Step 3: Implement `perform_backup`**

Add to `backup.rs`, above the `#[cfg(test)]` block:

```rust
use std::path::PathBuf;
use std::time::Duration;

use chrono::Local;
use rusqlite::Connection;

const MAX_BACKUPS: usize = 5;

/// Creates a backup of the given database connection into `backup_dir`.
/// Uses SQLite's Online Backup API for a consistent snapshot.
/// After a successful backup, prunes old backups to keep only `MAX_BACKUPS`.
/// Returns the path to the new backup file.
pub fn perform_backup(src: &Connection, backup_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_dir)
        .map_err(|e| format!("Failed to create backup directory: {e}"))?;

    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("{BACKUP_PREFIX}{timestamp}{BACKUP_EXT}");
    let dest_path = backup_dir.join(&filename);

    let mut dst = Connection::open(&dest_path)
        .map_err(|e| format!("Failed to open backup destination: {e}"))?;

    let backup = rusqlite::backup::Backup::new(src, &mut dst)
        .map_err(|e| format!("Failed to initialise backup: {e}"))?;

    backup
        .run_to_completion(-1, Duration::from_millis(0), None)
        .map_err(|e| format!("Backup failed: {e}"))?;

    log::info!("Database backed up to {:?}", dest_path);

    prune_old_backups(backup_dir, MAX_BACKUPS);

    Ok(dest_path)
}
```

Also add `use` for the new imports at the top of the file. The full top of `backup.rs` should be:

```rust
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Local;
use rusqlite::Connection;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml backup -- --nocapture 2>&1 | tail -15`
Expected: All 5 backup tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/database/backup.rs
git commit -m "feat: add perform_backup using SQLite Online Backup API"
```

---

### Task 4: Wire up the backup loop in `lib.rs`

**Files:**
- Modify: `src-tauri/src/database/backup.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add `start_backup_loop` to `backup.rs`**

Add this function to `backup.rs`, below `perform_backup`:

```rust
use std::sync::Arc;
use crate::state::AppStateContainer;

const BACKUP_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Runs the backup loop: one backup immediately, then every 30 minutes.
/// Intended to be spawned on a dedicated background thread.
/// Never panics — errors are logged and the loop continues.
pub fn start_backup_loop(container: Arc<AppStateContainer>) {
    let backup_dir = container
        .db_path
        .parent()
        .unwrap_or(&container.db_path)
        .join("backups");

    loop {
        let result = {
            let conn = match container.db.lock() {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Backup: failed to acquire DB lock: {e}");
                    std::thread::sleep(BACKUP_INTERVAL);
                    continue;
                }
            };
            perform_backup(&conn, &backup_dir)
        };

        match result {
            Ok(path) => log::info!("Backup complete: {:?}", path),
            Err(e) => log::warn!("Backup failed: {e}"),
        }

        std::thread::sleep(BACKUP_INTERVAL);
    }
}
```

- [ ] **Step 2: Spawn the backup thread in `lib.rs`**

In `src-tauri/src/lib.rs`, add the backup thread spawn after the peak preload spawn (after line 63) and before the server thread spawn:

```rust
            // Start automatic database backups (on startup + every 30 min)
            let backup_container = container.clone();
            std::thread::spawn(move || {
                database::backup::start_backup_loop(backup_container);
            });
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | tail -5`
Expected: `Finished` with no errors.

- [ ] **Step 4: Run all tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10`
Expected: All tests pass (35 existing + 5 new = 40).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/database/backup.rs src-tauri/src/lib.rs
git commit -m "feat: wire up automatic DB backup loop on startup + 30min interval"
```

---

### Task 5: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1`
Expected: All tests pass, no warnings.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings 2>&1 | tail -10`
Expected: No warnings or errors.

- [ ] **Step 3: Fix any clippy issues found**

Address any clippy warnings, re-run to confirm clean.

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "cleanup: address clippy warnings in backup module"
```

(Skip this step if clippy was clean.)
