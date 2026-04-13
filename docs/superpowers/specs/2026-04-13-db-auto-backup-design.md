# DB Auto-Backup Design

## Overview

Automatic, invisible SQLite database backups using the SQLite Online Backup API. Backups run on app startup and every 30 minutes, retaining the 5 most recent. No UI — fully background.

## Motivation

The app's entire state lives in a single SQLite file (`lutedrop.db`). If it becomes corrupted or accidentally deleted, all library entries, configurations, groups, comps, tracks, and clips are lost. Automatic backups provide a safety net with zero user effort.

## Architecture

### New module: `src-tauri/src/database/backup.rs`

Two public functions:

**`perform_backup(conn: &Connection, backup_dir: &Path) -> Result<PathBuf, String>`**

- Creates the `backup_dir` directory if it doesn't exist.
- Opens a new SQLite connection to the destination file: `lutedrop-backup-{YYYYMMDD-HHMMSS}.db`.
- Uses `rusqlite`'s `backup()` method (wraps SQLite's Online Backup API) to produce a consistent, self-contained snapshot. This does not block the source connection for writes.
- On success, calls `prune_old_backups()` and returns the path to the new backup file.
- On failure, logs a warning and returns the error. Never panics.

**`prune_old_backups(backup_dir: &Path, keep: usize)`**

- Lists all files in `backup_dir` matching the `lutedrop-backup-*.db` pattern.
- Sorts by filename (which sorts chronologically due to the timestamp format).
- Deletes all but the `keep` most recent files.
- Logs a warning for any file that fails to delete, but continues.

### Background thread: startup + 30-minute loop

A new background thread spawned in `lib.rs` during `setup`:

1. Immediately performs one backup (captures state at launch).
2. Sleeps for 30 minutes.
3. Repeats step 1-2 indefinitely.
4. Errors are logged and never propagate — the loop always continues.

This follows the same pattern as the existing peak preload thread and playback polling thread.

### Changes to existing files

**`src-tauri/src/state.rs`** — Add a `db_path: PathBuf` field to `AppStateContainer` so the backup thread can derive the backup directory without passing extra state.

**`src-tauri/src/lib.rs`** — Spawn the backup thread in the `setup` hook, after creating the `AppStateContainer`. Pass `container.clone()`. ~5 new lines, same pattern as the peak preload spawn.

**`src-tauri/src/database/mod.rs`** — Add `pub mod backup;` to expose the new module.

## Backup location

`{APP_DATA_DIR}/backups/`

On Windows: `C:\Users\{Username}\AppData\Local\com.lutedrop\backups\`

## File naming

`lutedrop-backup-{YYYYMMDD-HHMMSS}.db`

Example: `lutedrop-backup-20260413-143022.db`

The timestamp format ensures lexicographic sorting equals chronological sorting.

## Retention

Keep the 5 most recent backups. Pruning happens after every successful backup.

## What this does NOT include

- No UI for viewing, managing, or restoring backups.
- No schema changes or migration.
- No configuration table for backup settings.
- No impact on audio playback, remote control, or other threads.
- Restore is manual: user copies a backup file over `lutedrop.db` in AppData.

## Testing

- Unit test for `prune_old_backups`: create 7 dummy files in a temp dir, call prune with keep=5, assert 2 oldest are deleted.
- Unit test for `perform_backup`: use an in-memory source DB with some data, back up to a temp dir, open the backup and verify data is present.
