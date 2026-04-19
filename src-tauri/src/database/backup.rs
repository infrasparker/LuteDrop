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
        .run_to_completion(256, Duration::from_millis(0), None)
        .map_err(|e| format!("Backup failed: {e}"))?;

    prune_old_backups(backup_dir, MAX_BACKUPS);

    Ok(dest_path)
}

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
}
