use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use rusqlite::Connection;

use crate::state::AppStateContainer;

const BACKUP_PREFIX: &str = "lutedrop-backup-";
const BACKUP_EXT: &str = ".db";
const MAX_BACKUPS: usize = 5;
/// Copies taken before a schema migration. A different prefix from the
/// rolling backups, so `prune_old_backups` never deletes them.
const PRE_MIGRATION_PREFIX: &str = "pre-migration-v";

/// The folder backups of the database at `db_path` go in: `backups` beside it.
pub fn backup_dir_for(db_path: &Path) -> PathBuf {
    db_path.parent().unwrap_or(db_path).join("backups")
}

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

/// Creates a backup of the given database connection into `backup_dir`.
/// Uses SQLite's Online Backup API for a consistent snapshot.
/// After a successful backup, prunes old backups to keep only `MAX_BACKUPS`.
/// Returns the path to the new backup file.
pub fn perform_backup(src: &Connection, backup_dir: &Path) -> Result<PathBuf, String> {
    let dest_path = backup_to(src, backup_dir, BACKUP_PREFIX)?;
    prune_old_backups(backup_dir, MAX_BACKUPS);
    Ok(dest_path)
}

/// Copies the database into `backup_dir` as
/// `pre-migration-v<from_version>-<timestamp>.db` before a schema migration.
/// These copies are kept until the user deletes them.
pub fn perform_pre_migration_backup(
    src: &Connection,
    backup_dir: &Path,
    from_version: i64,
) -> Result<PathBuf, String> {
    backup_to(src, backup_dir, &format!("{PRE_MIGRATION_PREFIX}{from_version}-"))
}

/// Writes a consistent snapshot of `src` to `<backup_dir>/<prefix><timestamp>.db`
/// with SQLite's Online Backup API and returns its path.
fn backup_to(src: &Connection, backup_dir: &Path, prefix: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_dir)
        .map_err(|e| format!("Failed to create backup directory: {e}"))?;

    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let filename = format!("{prefix}{timestamp}{BACKUP_EXT}");
    let dest_path = backup_dir.join(&filename);

    let mut dst = Connection::open(&dest_path)
        .map_err(|e| format!("Failed to open backup destination: {e}"))?;

    let backup = rusqlite::backup::Backup::new(src, &mut dst)
        .map_err(|e| format!("Failed to initialise backup: {e}"))?;

    backup
        .run_to_completion(256, Duration::from_millis(0), None)
        .map_err(|e| format!("Backup failed: {e}"))?;

    Ok(dest_path)
}

const BACKUP_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Performs one backup by opening a dedicated read-only connection to the
/// database file. WAL mode allows this concurrent reader, so the app's main
/// connection (and every UI command behind its mutex) stays unblocked for
/// the duration — previously the mutex was held for the whole backup.
pub fn backup_once(db_path: &Path, backup_dir: &Path) -> Result<PathBuf, String> {
    use rusqlite::OpenFlags;
    let src = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open backup source connection: {e}"))?;
    perform_backup(&src, backup_dir)
}

/// Runs the backup loop: one backup immediately, then every 30 minutes.
/// Intended to be spawned on a dedicated background thread.
/// Never panics — errors are logged and the loop continues.
pub fn start_backup_loop(container: Arc<AppStateContainer>) {
    let backup_dir = backup_dir_for(&container.db_path);

    loop {
        match backup_once(&container.db_path, &backup_dir) {
            Ok(path) => log::info!("Backup complete: {:?}", path),
            Err(e) => log::warn!("Backup failed: {e}"),
        }
        std::thread::sleep(BACKUP_INTERVAL);
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
            "music",
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

    #[test]
    fn pre_migration_backup_is_named_and_never_pruned() {
        let src = crate::database::open_memory_db();
        let dir = tempfile::tempdir().expect("create temp dir");

        let path = perform_pre_migration_backup(&src, dir.path(), 6).expect("backup");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("pre-migration-v6-") && name.ends_with(".db"), "{name}");

        // Rolling backups prune only their own files.
        for i in 1..=6 {
            File::create(dir.path().join(format!("lutedrop-backup-2026040{i}-100000.db")))
                .expect("create file");
        }
        perform_backup(&src, dir.path()).expect("rolling backup");
        assert!(path.exists(), "pre-migration copy must survive pruning");
    }

    #[test]
    fn backup_once_reads_via_second_connection() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("app.db");
        // A real file-backed DB, opened the way the app opens it:
        let conn = crate::database::open_db(&db_path).expect("open db");
        crate::database::library::insert_library_entry(
            &conn, "/tmp/second-conn.mp3", Some("h2c"), "Second Conn", None, None, None, "music",
        )
        .expect("insert");

        // The app connection stays open (and un-locked by us) while the
        // backup runs from its own read-only connection:
        let backup_dir = dir.path().join("backups");
        let path = backup_once(&db_path, &backup_dir).expect("backup");

        let dst = Connection::open(&path).expect("open backup");
        let name: String = dst
            .query_row("SELECT name FROM library_entries WHERE content_hash = 'h2c'", [], |r| r.get(0))
            .expect("query");
        assert_eq!(name, "Second Conn");
        drop(conn);
    }
}
