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
