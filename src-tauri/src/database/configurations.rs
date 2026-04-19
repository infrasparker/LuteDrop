use rusqlite::{params, Connection};
use uuid::Uuid;

const SORT_GAP: i64 = 1000;

/// Returns the current time as milliseconds since the Unix epoch, as a string.
fn now_millis() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_millis()
        .to_string()
}

pub fn insert_configuration(conn: &Connection, name: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let created_at = now_millis();
    let max_sort: i64 = conn
        .query_row("SELECT COALESCE(MAX(sort_order), 0) FROM configurations", [], |row| row.get(0))
        .map_err(|e| format!("Failed to query max sort_order: {e}"))?;
    conn.execute(
        "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, created_at, max_sort + SORT_GAP],
    )
    .map_err(|e| format!("Failed to insert configuration: {e}"))?;
    Ok(id)
}

pub fn delete_configuration(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM configurations WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete configuration: {e}"))?;
    Ok(())
}

pub fn rename_configuration(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE configurations SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| format!("Failed to rename configuration: {e}"))?;
    Ok(())
}

pub fn set_active_configuration(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
        params![id],
    )
    .map_err(|e| format!("Failed to set active configuration: {e}"))?;
    Ok(())
}

pub fn duplicate_configuration(conn: &Connection, source_id: &str, new_name: &str) -> Result<String, String> {
    let new_config_id = insert_configuration(conn, new_name)?;

    // Load source groups
    let mut group_stmt = conn
        .prepare("SELECT id, name, sort_order FROM groups WHERE configuration_id = ?1 ORDER BY sort_order")
        .map_err(|e| format!("Failed to prepare group query: {e}"))?;
    let source_groups: Vec<(String, String, i64)> = group_stmt
        .query_map(params![source_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| format!("Failed to query source groups: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to collect source groups: {e}"))?;

    for (old_group_id, group_name, group_sort) in &source_groups {
        let new_group_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES (?1, ?2, ?3, ?4)",
            params![new_group_id, group_name, group_sort, new_config_id],
        ).map_err(|e| format!("Failed to copy group: {e}"))?;

        // Copy comps
        let mut comp_stmt = conn
            .prepare("SELECT id, name, sort_order FROM comps WHERE group_id = ?1 ORDER BY sort_order")
            .map_err(|e| format!("Failed to prepare comp query: {e}"))?;
        let source_comps: Vec<(String, String, i64)> = comp_stmt
            .query_map(params![old_group_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| format!("Failed to query source comps: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to collect source comps: {e}"))?;

        for (old_comp_id, comp_name, comp_sort) in &source_comps {
            let new_comp_id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
                params![new_comp_id, new_group_id, comp_name, comp_sort],
            ).map_err(|e| format!("Failed to copy comp: {e}"))?;

            // Copy tracks
            let mut track_stmt = conn
                .prepare(
                    "SELECT id, name, volume, mode_type, repeat, transition_type,
                            transition_gap_min, transition_gap_max, fade_in, fade_out, sort_order
                     FROM tracks WHERE comp_id = ?1 ORDER BY sort_order"
                )
                .map_err(|e| format!("Failed to prepare track query: {e}"))?;
            let source_tracks: Vec<(String, String, f64, String, Option<i64>, Option<String>, Option<f64>, Option<f64>, f64, f64, i64)> = track_stmt
                .query_map(params![old_comp_id], |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?,
                        row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                        row.get(8)?, row.get(9)?, row.get(10)?,
                    ))
                })
                .map_err(|e| format!("Failed to query source tracks: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to collect source tracks: {e}"))?;

            for (old_track_id, track_name, volume, mode_type, repeat, transition_type,
                 gap_min, gap_max, fade_in, fade_out, track_sort) in &source_tracks
            {
                let new_track_id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO tracks (id, comp_id, name, volume, mode_type, repeat,
                         transition_type, transition_gap_min, transition_gap_max,
                         fade_in, fade_out, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        new_track_id, new_comp_id, track_name, volume, mode_type,
                        repeat, transition_type, gap_min, gap_max, fade_in, fade_out, track_sort
                    ],
                ).map_err(|e| format!("Failed to copy track: {e}"))?;

                // Copy clips — keep same library_entry_id (library is shared)
                let mut clip_stmt = conn
                    .prepare(
                        "SELECT library_entry_id, volume, sort_order
                         FROM clips WHERE track_id = ?1 ORDER BY sort_order"
                    )
                    .map_err(|e| format!("Failed to prepare clip query: {e}"))?;
                let source_clips: Vec<(String, f64, i64)> = clip_stmt
                    .query_map(params![old_track_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(|e| format!("Failed to query source clips: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("Failed to collect source clips: {e}"))?;

                for (library_entry_id, clip_volume, clip_sort) in &source_clips {
                    let new_clip_id = Uuid::new_v4().to_string();
                    conn.execute(
                        "INSERT INTO clips (id, track_id, library_entry_id, volume, sort_order)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![new_clip_id, new_track_id, library_entry_id, clip_volume, clip_sort],
                    ).map_err(|e| format!("Failed to copy clip: {e}"))?;
                }
            }
        }
    }

    Ok(new_config_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{open_memory_db, ensure_test_config, hierarchy};

    #[test]
    fn insert_and_rename_configuration() {
        let conn = open_memory_db();
        ensure_test_config(&conn);

        let id = insert_configuration(&conn, "Dungeon Crawl").expect("insert");

        let name: String = conn
            .query_row("SELECT name FROM configurations WHERE id = ?1", params![id], |row| row.get(0))
            .expect("query name");
        assert_eq!(name, "Dungeon Crawl");

        rename_configuration(&conn, &id, "Cave Delve").expect("rename");

        let name: String = conn
            .query_row("SELECT name FROM configurations WHERE id = ?1", params![id], |row| row.get(0))
            .expect("query name");
        assert_eq!(name, "Cave Delve");
    }

    #[test]
    fn delete_configuration_cascades() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);

        hierarchy::insert_group(&conn, "My Group", &config_id).expect("insert group");

        let group_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))
            .expect("count");
        assert_eq!(group_count, 1);

        delete_configuration(&conn, &config_id).expect("delete");

        let group_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))
            .expect("count");
        assert_eq!(group_count, 0);
    }

    #[test]
    fn set_active_configuration_updates_meta() {
        let conn = open_memory_db();
        let _config1 = ensure_test_config(&conn);
        let config2 = insert_configuration(&conn, "Other").expect("insert");

        set_active_configuration(&conn, &config2).expect("set active");

        let active: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'active_configuration_id'",
                [],
                |row| row.get(0),
            )
            .expect("get active");
        assert_eq!(active, config2);
    }

    #[test]
    fn duplicate_configuration_deep_copies_hierarchy() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);

        let entry_id = crate::database::library::insert_library_entry(
            &conn, "/tmp/test.mp3", None, "Test Sound", None, None, None, "music",
        ).expect("insert library entry");

        let group_id = hierarchy::insert_group(&conn, "Battle", &config_id).expect("group");
        let comp_id = hierarchy::insert_comp(&conn, &group_id, "Act 1").expect("comp");
        let track_id = hierarchy::insert_track(&conn, &comp_id, "Theme").expect("track");
        let _clip_id = hierarchy::insert_clip(&conn, &track_id, &entry_id).expect("clip");

        let new_config_id = duplicate_configuration(&conn, &config_id, "Battle Copy").expect("duplicate");

        let new_name: String = conn
            .query_row("SELECT name FROM configurations WHERE id = ?1", params![new_config_id], |row| row.get(0))
            .expect("query name");
        assert_eq!(new_name, "Battle Copy");

        let new_groups: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, name FROM groups WHERE configuration_id = ?1")
                .expect("prepare");
            stmt.query_map(params![new_config_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect")
        };
        assert_eq!(new_groups.len(), 1);
        assert_eq!(new_groups[0].1, "Battle");
        assert_ne!(new_groups[0].0, group_id, "should have a new group ID");

        let clip_entry: String = conn
            .query_row(
                "SELECT c.library_entry_id FROM clips c
                 JOIN tracks t ON c.track_id = t.id
                 JOIN comps co ON t.comp_id = co.id
                 JOIN groups g ON co.group_id = g.id
                 WHERE g.configuration_id = ?1",
                params![new_config_id],
                |row| row.get(0),
            )
            .expect("query clip entry");
        assert_eq!(clip_entry, entry_id, "clip should reference the same library entry");

        let orig_group_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM groups WHERE configuration_id = ?1",
                params![config_id],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(orig_group_count, 1);
    }
}
