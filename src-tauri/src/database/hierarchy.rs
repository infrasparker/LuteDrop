use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

const SORT_GAP: i64 = 1000;

// ─── Parent-existence helpers ────────────────────────────────────────────────

fn require_parent(conn: &Connection, table: &str, id: &str) -> Result<(), String> {
    let sql = format!("SELECT id FROM {table} WHERE id = ?1");
    let exists: Option<String> = conn
        .query_row(&sql, params![id], |row| row.get(0))
        .optional()
        .map_err(|e| format!("Failed to check {table} existence: {e}"))?;
    if exists.is_none() {
        return Err(format!("{table} not found: {id}"));
    }
    Ok(())
}

// ─── Sort Order Utilities ─────────────────────────────────────────────────────

/// Returns MAX(sort_order) + SORT_GAP for siblings that share the same parent.
/// Returns SORT_GAP if the sibling list is empty.
fn next_sort_order(
    conn: &Connection,
    table: &str,
    parent_col: &str,
    parent_id: &str,
) -> Result<i64, String> {
    let sql = format!(
        "SELECT COALESCE(MAX(sort_order), 0) FROM {table} WHERE {parent_col} = ?1"
    );
    let max: i64 = conn
        .query_row(&sql, params![parent_id], |row| row.get(0))
        .map_err(|e| format!("Failed to query max sort_order in {table}: {e}"))?;
    Ok(max + SORT_GAP)
}

/// Moves `item_id` to 0-indexed `new_index` within the sibling list defined by
/// `parent_col = parent_id` (or the whole table when both are `None`).
///
/// The algorithm calculates a midpoint between the neighbours at `new_index-1`
/// and `new_index` (after excluding `item_id`). If no integer midpoint is
/// available the entire list is renumbered via `renumber_list`.
pub fn reorder_item(
    conn: &Connection,
    table: &str,
    parent_col: Option<&str>,
    parent_id: Option<&str>,
    item_id: &str,
    new_index: usize,
) -> Result<(), String> {
    // Fetch sibling sort_orders (excluding the item being moved), ordered.
    let siblings: Vec<(String, i64)> = fetch_siblings(conn, table, parent_col, parent_id, item_id)?;

    let before: Option<i64> = if new_index == 0 {
        None
    } else {
        siblings.get(new_index - 1).map(|(_, so)| *so)
    };
    let after: Option<i64> = siblings.get(new_index).map(|(_, so)| *so);

    let new_sort_order: Option<i64> = match (before, after) {
        (None, None) => Some(SORT_GAP),
        (Some(b), None) => Some(b + SORT_GAP),
        (None, Some(a)) => {
            if a > 1 {
                Some(a / 2)
            } else {
                None // need renumber
            }
        }
        (Some(b), Some(a)) => {
            let mid = b + (a - b) / 2;
            if mid > b {
                Some(mid)
            } else {
                None // no integer gap — renumber
            }
        }
    };

    if let Some(order) = new_sort_order {
        let sql = format!("UPDATE {table} SET sort_order = ?1 WHERE id = ?2");
        conn.execute(&sql, params![order, item_id])
            .map_err(|e| format!("Failed to update sort_order in {table}: {e}"))?;
        Ok(())
    } else {
        renumber_list(conn, table, parent_col, parent_id, item_id, new_index)
    }
}

/// Fetches the sibling list (excluding `item_id`) ordered by sort_order.
/// Returns Vec<(id, sort_order)>.
fn fetch_siblings(
    conn: &Connection,
    table: &str,
    parent_col: Option<&str>,
    parent_id: Option<&str>,
    item_id: &str,
) -> Result<Vec<(String, i64)>, String> {
    let rows: Vec<(String, i64)> = if let (Some(col), Some(pid)) = (parent_col, parent_id) {
        let sql = format!(
            "SELECT id, sort_order FROM {table} WHERE {col} = ?1 AND id != ?2 ORDER BY sort_order ASC"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("Failed to prepare siblings query in {table}: {e}"))?;
        let result: Result<Vec<(String, i64)>, _> = stmt
            .query_map(params![pid, item_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("Failed to query siblings in {table}: {e}"))?
            .collect();
        result.map_err(|e| format!("Failed to collect siblings in {table}: {e}"))?
    } else {
        let sql = format!(
            "SELECT id, sort_order FROM {table} WHERE id != ?1 ORDER BY sort_order ASC"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("Failed to prepare siblings query in {table}: {e}"))?;
        let result: Result<Vec<(String, i64)>, _> = stmt
            .query_map(params![item_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("Failed to query siblings in {table}: {e}"))?
            .collect();
        result.map_err(|e| format!("Failed to collect siblings in {table}: {e}"))?
    };

    Ok(rows)
}

/// Renumbers the entire sibling list with fresh SORT_GAP spacing, inserting
/// `item_id` at `new_index`.
fn renumber_list(
    conn: &Connection,
    table: &str,
    parent_col: Option<&str>,
    parent_id: Option<&str>,
    item_id: &str,
    new_index: usize,
) -> Result<(), String> {
    let siblings = fetch_siblings(conn, table, parent_col, parent_id, item_id)?;

    // Build the ordered list with item_id inserted at new_index.
    let mut ordered: Vec<String> = siblings.into_iter().map(|(id, _)| id).collect();
    let clamped = new_index.min(ordered.len());
    ordered.insert(clamped, item_id.to_string());

    // Assign fresh sort_orders.
    for (i, id) in ordered.iter().enumerate() {
        let order = (i as i64 + 1) * SORT_GAP;
        let sql = format!("UPDATE {table} SET sort_order = ?1 WHERE id = ?2");
        conn.execute(&sql, params![order, id])
            .map_err(|e| format!("Failed to renumber {table}: {e}"))?;
    }

    Ok(())
}

// ─── Groups ───────────────────────────────────────────────────────────────────

pub fn insert_group(conn: &Connection, name: &str, configuration_id: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let sort_order = next_sort_order(conn, "groups", "configuration_id", configuration_id)?;
    conn.execute(
        "INSERT INTO groups (id, name, sort_order, configuration_id) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, sort_order, configuration_id],
    )
    .map_err(|e| format!("Failed to insert group: {e}"))?;
    Ok(id)
}

#[allow(dead_code)]
pub fn reorder_group(conn: &Connection, configuration_id: &str, group_id: &str, new_index: usize) -> Result<(), String> {
    reorder_item(conn, "groups", Some("configuration_id"), Some(configuration_id), group_id, new_index)
}

pub fn delete_group(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM groups WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete group: {e}"))?;
    Ok(())
}

pub fn rename_group(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE groups SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| format!("Failed to rename group: {e}"))?;
    Ok(())
}

// ─── Comps ────────────────────────────────────────────────────────────────────

pub fn insert_comp(conn: &Connection, group_id: &str, name: &str) -> Result<String, String> {
    require_parent(conn, "groups", group_id)?;
    let id = Uuid::new_v4().to_string();
    let sort_order = next_sort_order(conn, "comps", "group_id", group_id)?;
    conn.execute(
        "INSERT INTO comps (id, group_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
        params![id, group_id, name, sort_order],
    )
    .map_err(|e| format!("Failed to insert comp: {e}"))?;
    Ok(id)
}

pub fn delete_comp(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM comps WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete comp: {e}"))?;
    Ok(())
}

pub fn rename_comp(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE comps SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| format!("Failed to rename comp: {e}"))?;
    Ok(())
}

// ─── Tracks ───────────────────────────────────────────────────────────────────

pub fn insert_track(conn: &Connection, comp_id: &str, name: &str) -> Result<String, String> {
    require_parent(conn, "comps", comp_id)?;
    let id = Uuid::new_v4().to_string();
    let sort_order = next_sort_order(conn, "tracks", "comp_id", comp_id)?;
    conn.execute(
        "INSERT INTO tracks
             (id, comp_id, name, volume, mode_type, repeat, transition_type,
              fade_in, fade_out, sort_order)
         VALUES (?1, ?2, ?3, 1.0, 'sequence', 1, 'gapless', 0.0, 0.0, ?4)",
        params![id, comp_id, name, sort_order],
    )
    .map_err(|e| format!("Failed to insert track: {e}"))?;
    Ok(id)
}

pub fn delete_track(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM tracks WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete track: {e}"))?;
    Ok(())
}

pub fn rename_track(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE tracks SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| format!("Failed to rename track: {e}"))?;
    Ok(())
}

pub fn update_track_volume(conn: &Connection, id: &str, volume: f64) -> Result<(), String> {
    conn.execute(
        "UPDATE tracks SET volume = ?1 WHERE id = ?2",
        params![volume, id],
    )
    .map_err(|e| format!("Failed to update track volume: {e}"))?;
    Ok(())
}

pub fn update_track_mode(
    conn: &Connection,
    id: &str,
    mode_type: &str,
    repeat: Option<bool>,
    transition_type: Option<&str>,
    gap_min: Option<f64>,
    gap_max: Option<f64>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE tracks SET
             mode_type          = ?1,
             repeat             = COALESCE(?2, repeat),
             transition_type    = COALESCE(?3, transition_type),
             transition_gap_min = COALESCE(?4, transition_gap_min),
             transition_gap_max = COALESCE(?5, transition_gap_max)
         WHERE id = ?6",
        params![
            mode_type,
            repeat.map(|b| if b { 1i64 } else { 0i64 }),
            transition_type,
            gap_min,
            gap_max,
            id
        ],
    )
    .map_err(|e| format!("Failed to update track mode: {e}"))?;
    Ok(())
}

pub fn update_track_fades(
    conn: &Connection,
    id: &str,
    fade_in: f64,
    fade_out: f64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE tracks SET fade_in = ?1, fade_out = ?2 WHERE id = ?3",
        params![fade_in, fade_out, id],
    )
    .map_err(|e| format!("Failed to update track fades: {e}"))?;
    Ok(())
}

// ─── Clips ────────────────────────────────────────────────────────────────────

pub fn insert_clip(
    conn: &Connection,
    track_id: &str,
    library_entry_id: &str,
) -> Result<String, String> {
    require_parent(conn, "tracks", track_id)?;
    require_parent(conn, "library_entries", library_entry_id)?;
    let id = Uuid::new_v4().to_string();
    let sort_order = next_sort_order(conn, "clips", "track_id", track_id)?;
    conn.execute(
        "INSERT INTO clips (id, track_id, library_entry_id, volume, sort_order)
         VALUES (?1, ?2, ?3, 1.0, ?4)",
        params![id, track_id, library_entry_id, sort_order],
    )
    .map_err(|e| format!("Failed to insert clip: {e}"))?;
    Ok(id)
}

pub fn delete_clip(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM clips WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete clip: {e}"))?;
    Ok(())
}

pub fn update_clip_volume(conn: &Connection, id: &str, volume: f64) -> Result<(), String> {
    conn.execute(
        "UPDATE clips SET volume = ?1 WHERE id = ?2",
        params![volume, id],
    )
    .map_err(|e| format!("Failed to update clip volume: {e}"))?;
    Ok(())
}

#[allow(dead_code)]
pub fn reorder_clip(
    conn: &Connection,
    track_id: &str,
    clip_id: &str,
    new_index: usize,
) -> Result<(), String> {
    reorder_item(conn, "clips", Some("track_id"), Some(track_id), clip_id, new_index)
}

/// Writes a complete clip ordering for a track in one transaction:
/// `ordered_ids[i]` gets sort_order (i+1)*SORT_GAP. Replaces the O(n²)
/// per-clip reorder loop for drag-and-drop.
pub fn set_clip_order(conn: &Connection, track_id: &str, ordered_ids: &[String]) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin clip-order transaction: {e}"))?;
    for (i, clip_id) in ordered_ids.iter().enumerate() {
        conn.execute(
            "UPDATE clips SET sort_order = ?1 WHERE id = ?2 AND track_id = ?3",
            params![(i as i64 + 1) * SORT_GAP, clip_id, track_id],
        )
        .map_err(|e| format!("Failed to renumber clip: {e}"))?;
    }
    tx.commit().map_err(|e| format!("Failed to commit clip order: {e}"))?;
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::open_memory_db;
    use crate::database::library::insert_library_entry;

    fn setup_test_config(conn: &Connection) -> String {
        let config_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO configurations (id, name, created_at, sort_order) VALUES (?1, 'Test', '0', 1000)",
            rusqlite::params![config_id],
        ).expect("insert test config");
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('active_configuration_id', ?1)",
            rusqlite::params![config_id],
        ).expect("set active config");
        config_id
    }

    // ── 1. group_crud ─────────────────────────────────────────────────────────

    #[test]
    fn group_crud() {
        let conn = open_memory_db();
        let config_id = setup_test_config(&conn);

        let id = insert_group(&conn, "Adventure", &config_id).expect("insert_group");

        rename_group(&conn, &id, "Epic Adventure").expect("rename_group");

        let name: String = conn
            .query_row(
                "SELECT name FROM groups WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("SELECT name");
        assert_eq!(name, "Epic Adventure");

        delete_group(&conn, &id).expect("delete_group");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))
            .expect("COUNT");
        assert_eq!(count, 0);
    }

    // ── 2. sort_order_increments ──────────────────────────────────────────────

    #[test]
    fn sort_order_increments() {
        let conn = open_memory_db();
        let config_id = setup_test_config(&conn);

        let id_a = insert_group(&conn, "Alpha", &config_id).expect("insert A");
        let id_b = insert_group(&conn, "Beta", &config_id).expect("insert B");
        let id_c = insert_group(&conn, "Gamma", &config_id).expect("insert C");

        let orders: Vec<i64> = {
            let mut stmt = conn
                .prepare(
                    "SELECT sort_order FROM groups WHERE id IN (?1, ?2, ?3) ORDER BY sort_order ASC",
                )
                .expect("prepare");
            stmt.query_map(params![id_a, id_b, id_c], |row| row.get(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect")
        };

        assert_eq!(orders.len(), 3);
        assert!(orders[0] < orders[1], "A sort_order < B sort_order");
        assert!(orders[1] < orders[2], "B sort_order < C sort_order");
    }

    // ── 3. reorder_moves_to_middle ────────────────────────────────────────────

    #[test]
    fn reorder_moves_to_middle() {
        let conn = open_memory_db();
        let config_id = setup_test_config(&conn);

        let id_a = insert_group(&conn, "A", &config_id).expect("insert A");
        let id_b = insert_group(&conn, "B", &config_id).expect("insert B");
        let id_c = insert_group(&conn, "C", &config_id).expect("insert C");

        // Move C (index 2) to position 1 → expected order: A, C, B
        reorder_item(&conn, "groups", None, None, &id_c, 1).expect("reorder_item");

        let ordered: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM groups ORDER BY sort_order ASC")
                .expect("prepare");
            stmt.query_map([], |row| row.get(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect")
        };

        assert_eq!(ordered, vec![id_a, id_c, id_b]);
    }

    // ── 4. cascade_deletes ────────────────────────────────────────────────────

    #[test]
    fn cascade_deletes() {
        let conn = open_memory_db();
        let config_id = setup_test_config(&conn);

        let entry_id = insert_library_entry(
            &conn,
            "/tmp/cascade.mp3",
            None,
            "Cascade Track",
            None,
            None,
            None,
            "music",
        )
        .expect("insert library entry");

        let group_id = insert_group(&conn, "CascadeGroup", &config_id).expect("insert group");
        let comp_id = insert_comp(&conn, &group_id, "CascadeComp").expect("insert comp");
        let track_id = insert_track(&conn, &comp_id, "CascadeTrack").expect("insert track");
        let clip_id = insert_clip(&conn, &track_id, &entry_id).expect("insert clip");

        // Deleting the group should cascade-delete comp, track, clip.
        delete_group(&conn, &group_id).expect("delete group");

        let comp_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM comps WHERE id = ?1",
                params![comp_id],
                |row| row.get(0),
            )
            .expect("count comps");
        assert_eq!(comp_count, 0, "comp should be gone");

        let track_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tracks WHERE id = ?1",
                params![track_id],
                |row| row.get(0),
            )
            .expect("count tracks");
        assert_eq!(track_count, 0, "track should be gone");

        let clip_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM clips WHERE id = ?1",
                params![clip_id],
                |row| row.get(0),
            )
            .expect("count clips");
        assert_eq!(clip_count, 0, "clip should be gone");

        // Library entry must survive.
        let entry_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM library_entries WHERE id = ?1",
                params![entry_id],
                |row| row.get(0),
            )
            .expect("count library entries");
        assert_eq!(entry_count, 1, "library entry should survive");
    }

    // ── 5. clip_insert_and_reorder ────────────────────────────────────────────

    #[test]
    fn clip_insert_and_reorder() {
        let conn = open_memory_db();
        let config_id = setup_test_config(&conn);

        // Create the minimum required hierarchy.
        let entry1 = insert_library_entry(&conn, "/tmp/c1.mp3", None, "C1", None, None, None, "music")
            .expect("entry1");
        let entry2 = insert_library_entry(&conn, "/tmp/c2.mp3", None, "C2", None, None, None, "music")
            .expect("entry2");
        let entry3 = insert_library_entry(&conn, "/tmp/c3.mp3", None, "C3", None, None, None, "music")
            .expect("entry3");

        let group_id = insert_group(&conn, "G", &config_id).expect("group");
        let comp_id = insert_comp(&conn, &group_id, "Co").expect("comp");
        let track_id = insert_track(&conn, &comp_id, "T").expect("track");

        let clip1 = insert_clip(&conn, &track_id, &entry1).expect("clip1");
        let clip2 = insert_clip(&conn, &track_id, &entry2).expect("clip2");
        let clip3 = insert_clip(&conn, &track_id, &entry3).expect("clip3");

        // Move clip3 (position 2) to position 0.
        reorder_clip(&conn, &track_id, &clip3, 0).expect("reorder clip3 to 0");

        let ordered_entry_ids: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT library_entry_id FROM clips WHERE track_id = ?1 ORDER BY sort_order ASC",
                )
                .expect("prepare");
            stmt.query_map(params![track_id], |row| row.get(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect")
        };

        assert_eq!(
            ordered_entry_ids,
            vec![entry3, entry1, entry2],
            "clip3 should be first, then clip1, then clip2"
        );

        // Verify we're not leaking rows.
        let _ = (clip1, clip2);
    }

    // ── 6. set_clip_order_applies_full_order_in_one_pass ──────────────────────

    #[test]
    fn set_clip_order_applies_full_order_in_one_pass() {
        let conn = open_memory_db();
        let config_id = setup_test_config(&conn);
        let e1 = insert_library_entry(&conn, "/o1.mp3", None, "O1", None, None, None, "music").unwrap();
        let e2 = insert_library_entry(&conn, "/o2.mp3", None, "O2", None, None, None, "music").unwrap();
        let e3 = insert_library_entry(&conn, "/o3.mp3", None, "O3", None, None, None, "music").unwrap();
        let g = insert_group(&conn, "G", &config_id).unwrap();
        let c = insert_comp(&conn, &g, "C").unwrap();
        let t = insert_track(&conn, &c, "T").unwrap();
        let c1 = insert_clip(&conn, &t, &e1).unwrap();
        let c2 = insert_clip(&conn, &t, &e2).unwrap();
        let c3 = insert_clip(&conn, &t, &e3).unwrap();

        set_clip_order(&conn, &t, &[c3.clone(), c1.clone(), c2.clone()]).unwrap();

        let ordered: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM clips WHERE track_id = ?1 ORDER BY sort_order ASC")
                .unwrap();
            stmt.query_map(params![t], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(ordered, vec![c3, c1, c2]);
    }
}
