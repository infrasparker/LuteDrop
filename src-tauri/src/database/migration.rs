use rusqlite::{params, Connection};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use crate::database::library;

// ─── Legacy JSON Types ────────────────────────────────────────────────────────

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
    mode: serde_json::Value, // parse mode_type + fields from raw JSON
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

// ─── Public API ───────────────────────────────────────────────────────────────

/// Returns true if `json_path` exists AND the groups table is empty.
pub fn needs_migration(conn: &Connection, json_path: &Path) -> bool {
    if !json_path.exists() {
        return false;
    }

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))
        .unwrap_or(0);

    count == 0
}

/// Migrates data from the legacy JSON file into the SQLite database.
///
/// Returns `(groups_migrated, entries_created, files_missing)` on success.
pub fn migrate_from_json(
    conn: &Connection,
    json_path: &Path,
) -> Result<(usize, usize, usize), String> {
    let raw = fs::read_to_string(json_path)
        .map_err(|e| format!("Failed to read JSON file: {e}"))?;

    let old_state: OldAppState = serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse JSON: {e}"))?;

    // ── Phase 1: Build file_path → entry_id map ───────────────────────────────
    // Walk all clips, compute hashes, dedup by hash, insert library entries.

    // file_path → entry_id
    let mut path_to_entry: HashMap<String, String> = HashMap::new();
    // sha256_hex → entry_id  (for dedup)
    let mut hash_to_entry: HashMap<String, String> = HashMap::new();

    let mut entries_created: usize = 0;
    let mut files_missing: usize = 0;

    for group in &old_state.groups {
        for comp in &group.comps {
            for track in &comp.tracks {
                for clip in &track.clips {
                    // Skip if we've already processed this file_path.
                    if path_to_entry.contains_key(&clip.file_path) {
                        continue;
                    }

                    let (hash_opt, missing) = hash_file(&clip.file_path);

                    if missing {
                        files_missing += 1;
                    }

                    // Dedup: if we already have an entry with this hash, reuse it.
                    let entry_id = if let Some(ref hash) = hash_opt {
                        if let Some(existing_id) = hash_to_entry.get(hash) {
                            existing_id.clone()
                        } else {
                            // Check DB for existing entry with same hash.
                            match library::get_library_entry_by_hash(conn, hash) {
                                Ok(Some((id, _))) => {
                                    hash_to_entry.insert(hash.clone(), id.clone());
                                    id
                                }
                                _ => {
                                    // Insert new entry.
                                    let id = library::insert_library_entry(
                                        conn,
                                        &clip.file_path,
                                        Some(hash.as_str()),
                                        &clip.name,
                                        None,
                                        None,
                                        None,
                                    )
                                    .map_err(|e| format!("Failed to insert library entry: {e}"))?;
                                    hash_to_entry.insert(hash.clone(), id.clone());
                                    entries_created += 1;
                                    id
                                }
                            }
                        }
                    } else {
                        // Missing/unreadable file — insert with null hash.
                        let id = library::insert_library_entry(
                            conn,
                            &clip.file_path,
                            None,
                            &clip.name,
                            None,
                            None,
                            None,
                        )
                        .map_err(|e| format!("Failed to insert missing library entry: {e}"))?;
                        entries_created += 1;
                        id
                    };

                    path_to_entry.insert(clip.file_path.clone(), entry_id);
                }
            }
        }
    }

    // ── Phase 2: Recreate hierarchy ───────────────────────────────────────────

    let groups_migrated = old_state.groups.len();

    for (g_idx, group) in old_state.groups.iter().enumerate() {
        let g_sort = ((g_idx + 1) * 1000) as i64;
        conn.execute(
            "INSERT INTO groups (id, name, sort_order) VALUES (?1, ?2, ?3)",
            params![group.id, group.name, g_sort],
        )
        .map_err(|e| format!("Failed to insert group '{}': {e}", group.name))?;

        for (c_idx, comp) in group.comps.iter().enumerate() {
            let c_sort = ((c_idx + 1) * 1000) as i64;
            conn.execute(
                "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
                params![comp.id, group.id, comp.name, c_sort],
            )
            .map_err(|e| format!("Failed to insert comp '{}': {e}", comp.name))?;

            for (t_idx, track) in comp.tracks.iter().enumerate() {
                let t_sort = ((t_idx + 1) * 1000) as i64;

                let (mode_type, repeat, transition_type, gap_min, gap_max) =
                    parse_old_mode(&track.mode);

                conn.execute(
                    "INSERT INTO tracks
                         (id, comp_id, name, volume, mode_type, repeat,
                          transition_type, transition_gap_min, transition_gap_max,
                          fade_in, fade_out, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        track.id,
                        comp.id,
                        track.name,
                        track.volume,
                        mode_type,
                        repeat,
                        transition_type,
                        gap_min,
                        gap_max,
                        track.fade_in,
                        track.fade_out,
                        t_sort,
                    ],
                )
                .map_err(|e| format!("Failed to insert track '{}': {e}", track.name))?;

                // For seamless tracks, store loop points on the library entry.
                if mode_type == "seamless" {
                    if let Some(clip) = track.clips.first() {
                        if let Some(entry_id) = path_to_entry.get(&clip.file_path) {
                            let loop_start = track.mode.get("loopStart").and_then(|v| v.as_f64());
                            let loop_end = track.mode.get("loopEnd").and_then(|v| v.as_f64());
                            if loop_start.is_some() || loop_end.is_some() {
                                library::update_library_entry_loop(
                                    conn, entry_id, loop_start, loop_end,
                                )
                                .map_err(|e| {
                                    format!("Failed to update loop points for entry: {e}")
                                })?;
                            }
                        }
                    }
                }

                for (cl_idx, clip) in track.clips.iter().enumerate() {
                    let cl_sort = ((cl_idx + 1) * 1000) as i64;

                    let entry_id = path_to_entry
                        .get(&clip.file_path)
                        .ok_or_else(|| {
                            format!(
                                "No library entry found for file path '{}' (clip '{}')",
                                clip.file_path, clip.name
                            )
                        })?
                        .clone();

                    conn.execute(
                        "INSERT INTO clips (id, track_id, library_entry_id, volume, sort_order)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![clip.id, track.id, entry_id, clip.volume, cl_sort],
                    )
                    .map_err(|e| format!("Failed to insert clip '{}': {e}", clip.name))?;
                }
            }
        }
    }

    Ok((groups_migrated, entries_created, files_missing))
}

/// Renames the JSON file to `.backup.json`.
pub fn backup_json(json_path: &Path) -> Result<(), String> {
    let backup_path = json_path.with_extension("backup.json");
    fs::rename(json_path, &backup_path)
        .map_err(|e| format!("Failed to rename JSON to backup: {e}"))?;
    Ok(())
}

// ─── Private Helpers ──────────────────────────────────────────────────────────

/// Computes the SHA-256 hex digest of a file.
///
/// Returns `(Some(hex), false)` on success, `(None, true)` if the file is
/// missing or unreadable.
fn hash_file(path: &str) -> (Option<String>, bool) {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, true),
    };

    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];

    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return (None, true),
        }
    }

    let hex = format!("{:x}", hasher.finalize());
    (Some(hex), false)
}

/// Parses the legacy mode JSON into flat DB columns.
///
/// Returns `(mode_type, repeat, transition_type, gap_min, gap_max)`.
///
/// Expected JSON shapes:
/// - `{"type": "seamless", "loopStart": 0.0, "loopEnd": 120.0}`
/// - `{"type": "sequence", "repeat": true, "transition": {"type": "gapless"}}`
/// - `{"type": "sequence", "repeat": true, "transition": {"type": "gap", "min": 1.0, "max": 3.0}}`
/// - `{"type": "shuffle", "repeat": false, "transition": {"type": "crossfade"}}`
fn parse_old_mode(
    mode_json: &serde_json::Value,
) -> (String, Option<i32>, Option<String>, Option<f64>, Option<f64>) {
    let mode_type = mode_json
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("sequence")
        .to_string();

    if mode_type == "seamless" {
        return (mode_type, None, None, None, None);
    }

    let repeat: Option<i32> = mode_json
        .get("repeat")
        .and_then(|v| v.as_bool())
        .map(|b| if b { 1 } else { 0 });

    let transition = mode_json.get("transition");

    let transition_type: Option<String> = transition
        .and_then(|t| t.get("type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let gap_min: Option<f64> = transition
        .and_then(|t| t.get("min"))
        .and_then(|v| v.as_f64());

    let gap_max: Option<f64> = transition
        .and_then(|t| t.get("max"))
        .and_then(|v| v.as_f64());

    (mode_type, repeat, transition_type, gap_min, gap_max)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_memory_db;
    use std::io::Write;

    const SAMPLE_JSON: &str = r#"{
        "groups": [{
            "id": "g1", "name": "Exploration",
            "comps": [{"id": "c1", "name": "Tavern",
                "tracks": [{"id": "t1", "name": "Ambience",
                    "clips": [{"id": "cl1", "name": "Tavern Music", "filePath": "/nonexistent/tavern.mp3", "volume": 0.8}],
                    "volume": 1.0,
                    "mode": {"type": "sequence", "repeat": true, "transition": {"type": "gapless"}},
                    "fadeIn": 2.0, "fadeOut": 1.5}]}]}],
        "lastSavedAt": "1234567890"
    }"#;

    // ── 1. parse_old_json ─────────────────────────────────────────────────────

    #[test]
    fn parse_old_json() {
        let state: OldAppState =
            serde_json::from_str(SAMPLE_JSON).expect("should parse sample JSON");

        assert_eq!(state.groups.len(), 1);
        let group = &state.groups[0];
        assert_eq!(group.id, "g1");
        assert_eq!(group.name, "Exploration");

        assert_eq!(group.comps.len(), 1);
        let comp = &group.comps[0];
        assert_eq!(comp.id, "c1");
        assert_eq!(comp.name, "Tavern");

        assert_eq!(comp.tracks.len(), 1);
        let track = &comp.tracks[0];
        assert_eq!(track.id, "t1");
        assert_eq!(track.name, "Ambience");
        assert!((track.volume - 1.0).abs() < 1e-9);
        assert!((track.fade_in - 2.0).abs() < 1e-9);
        assert!((track.fade_out - 1.5).abs() < 1e-9);

        assert_eq!(track.clips.len(), 1);
        let clip = &track.clips[0];
        assert_eq!(clip.id, "cl1");
        assert_eq!(clip.name, "Tavern Music");
        assert_eq!(clip.file_path, "/nonexistent/tavern.mp3");
        assert!((clip.volume - 0.8).abs() < 1e-9);

        assert_eq!(state.last_saved_at, Some("1234567890".to_string()));
    }

    // ── 2. migrate_with_missing_files ─────────────────────────────────────────

    #[test]
    fn migrate_with_missing_files() {
        let conn = open_memory_db();

        // Write JSON to a temp file.
        let dir = std::env::temp_dir().join(format!(
            "lutedrop_migration_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let json_path = dir.join("lutedrop-data.json");
        {
            let mut f = std::fs::File::create(&json_path).expect("create json");
            f.write_all(SAMPLE_JSON.as_bytes()).expect("write json");
        }

        // Verify needs_migration returns true before migration.
        assert!(
            needs_migration(&conn, &json_path),
            "needs_migration should be true before migration"
        );

        // Run migration (file /nonexistent/tavern.mp3 won't exist).
        let (groups, entries, missing) =
            migrate_from_json(&conn, &json_path).expect("migration should succeed");

        assert_eq!(groups, 1, "one group migrated");
        assert_eq!(entries, 1, "one library entry created");
        assert_eq!(missing, 1, "one missing file");

        // Verify groups table has one row.
        let group_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))
            .expect("count groups");
        assert_eq!(group_count, 1);

        // Verify library entry has null content_hash.
        let hash: Option<String> = conn
            .query_row(
                "SELECT content_hash FROM library_entries WHERE file_path = ?1",
                ["/nonexistent/tavern.mp3"],
                |row| row.get(0),
            )
            .expect("query content_hash");
        assert!(hash.is_none(), "content_hash should be null for missing file");

        // Verify clips table has one row referencing the entry.
        let clip_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM clips", [], |row| row.get(0))
            .expect("count clips");
        assert_eq!(clip_count, 1);

        // Verify needs_migration returns false after migration.
        assert!(
            !needs_migration(&conn, &json_path),
            "needs_migration should be false after migration"
        );

        // Cleanup temp directory.
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 3. parse_old_mode_variants ────────────────────────────────────────────

    #[test]
    fn parse_old_mode_variants() {
        // Seamless mode.
        let seamless = serde_json::json!({
            "type": "seamless",
            "loopStart": 1.0,
            "loopEnd": 30.0
        });
        let (mt, repeat, trans, gmin, gmax) = parse_old_mode(&seamless);
        assert_eq!(mt, "seamless");
        assert!(repeat.is_none());
        assert!(trans.is_none());
        assert!(gmin.is_none());
        assert!(gmax.is_none());

        // Sequence + gapless transition.
        let seq_gapless = serde_json::json!({
            "type": "sequence",
            "repeat": true,
            "transition": {"type": "gapless"}
        });
        let (mt, repeat, trans, gmin, gmax) = parse_old_mode(&seq_gapless);
        assert_eq!(mt, "sequence");
        assert_eq!(repeat, Some(1));
        assert_eq!(trans.as_deref(), Some("gapless"));
        assert!(gmin.is_none());
        assert!(gmax.is_none());

        // Sequence + gap transition.
        let seq_gap = serde_json::json!({
            "type": "sequence",
            "repeat": true,
            "transition": {"type": "gap", "min": 1.0, "max": 3.0}
        });
        let (mt, repeat, trans, gmin, gmax) = parse_old_mode(&seq_gap);
        assert_eq!(mt, "sequence");
        assert_eq!(repeat, Some(1));
        assert_eq!(trans.as_deref(), Some("gap"));
        assert!((gmin.unwrap() - 1.0).abs() < 1e-9);
        assert!((gmax.unwrap() - 3.0).abs() < 1e-9);

        // Shuffle + no repeat.
        let shuffle = serde_json::json!({
            "type": "shuffle",
            "repeat": false,
            "transition": {"type": "crossfade"}
        });
        let (mt, repeat, trans, gmin, gmax) = parse_old_mode(&shuffle);
        assert_eq!(mt, "shuffle");
        assert_eq!(repeat, Some(0));
        assert_eq!(trans.as_deref(), Some("crossfade"));
        assert!(gmin.is_none());
        assert!(gmax.is_none());
    }
}
