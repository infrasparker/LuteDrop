use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::database::SORT_GAP;

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
    let new_index = new_index.min(siblings.len());

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

// ─── Boards ───────────────────────────────────────────────────────────────────

pub fn insert_board(conn: &Connection, name: &str, configuration_id: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let sort_order = next_sort_order(conn, "boards", "configuration_id", configuration_id)?;
    conn.execute(
        "INSERT INTO boards (id, name, sort_order, configuration_id) VALUES (?1, ?2, ?3, ?4)",
        params![id, name, sort_order, configuration_id],
    )
    .map_err(|e| format!("Failed to insert board: {e}"))?;
    Ok(id)
}

#[allow(dead_code)]
pub fn reorder_board(conn: &Connection, configuration_id: &str, board_id: &str, new_index: usize) -> Result<(), String> {
    reorder_item(conn, "boards", Some("configuration_id"), Some(configuration_id), board_id, new_index)
}

pub fn delete_board(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM boards WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete board: {e}"))?;
    Ok(())
}

pub fn rename_board(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE boards SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| format!("Failed to rename board: {e}"))?;
    Ok(())
}

// ─── Scenes ────────────────────────────────────────────────────────────────────

pub fn insert_scene(conn: &Connection, board_id: &str, name: &str) -> Result<String, String> {
    require_parent(conn, "boards", board_id)?;
    let id = Uuid::new_v4().to_string();
    let sort_order = next_sort_order(conn, "scenes", "board_id", board_id)?;
    conn.execute(
        "INSERT INTO scenes (id, board_id, name, sort_order) VALUES (?1, ?2, ?3, ?4)",
        params![id, board_id, name, sort_order],
    )
    .map_err(|e| format!("Failed to insert scene: {e}"))?;
    Ok(id)
}

pub fn delete_scene(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM scenes WHERE id = ?1", params![id])
        .map_err(|e| format!("Failed to delete scene: {e}"))?;
    Ok(())
}

pub fn rename_scene(conn: &Connection, id: &str, name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE scenes SET name = ?1 WHERE id = ?2",
        params![name, id],
    )
    .map_err(|e| format!("Failed to rename scene: {e}"))?;
    Ok(())
}

// ─── Tracks ───────────────────────────────────────────────────────────────────

/// Fixture helper: production creates inventory tracks independently.
#[cfg(test)]
pub fn insert_track(conn: &Connection, scene_id: &str, name: &str) -> Result<String,String> {
    let id = super::soundstage::create_track(conn,name)?;
    update_track_mode(conn,&id,"sequence",Some(true),Some("gapless"),None,None)?;
    let board: String=conn.query_row("SELECT board_id FROM scenes WHERE id=?1",[scene_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    super::soundstage::attach_track(conn,&board,&id)?;
    conn.execute("INSERT INTO scene_tracks(scene_id,track_id,enabled,volume,sort_order) VALUES(?1,?2,1,1,(SELECT COALESCE(MAX(sort_order),0)+1000 FROM scene_tracks WHERE scene_id=?1))",params![scene_id,id]).map_err(|e|e.to_string())?;
    Ok(id)
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

// ─── Hierarchy organization ──────────────────────────────────────────────────

/// Reordering resolves the parent from the item itself, so a stale caller cannot
/// accidentally place it among another parent's siblings.
pub fn reorder_scene(conn:&Connection,scene_id:&str,new_index:usize)->Result<(),String> {
    let board:String=conn.query_row("SELECT board_id FROM scenes WHERE id=?1",[scene_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let tx=conn.unchecked_transaction().map_err(|e|e.to_string())?;
    reorder_item(conn,"scenes",Some("board_id"),Some(&board),scene_id,new_index)?;
    tx.commit().map_err(|e|e.to_string())
}

pub fn move_scene(conn:&Connection,scene_id:&str,destination_board_id:&str)->Result<(),String> {
    require_parent(conn,"scenes",scene_id)?;require_parent(conn,"boards",destination_board_id)?;
    let tx=conn.unchecked_transaction().map_err(|e|e.to_string())?;
    let sort=next_sort_order(conn,"scenes","board_id",destination_board_id)?;
    conn.execute("UPDATE scenes SET board_id=?1,sort_order=?2 WHERE id=?3",params![destination_board_id,sort,scene_id]).map_err(|e|e.to_string())?;
    for state in super::soundstage::scene_tracks(conn,scene_id)? {super::soundstage::attach_track(conn,destination_board_id,&state.track_id)?;}
    tx.commit().map_err(|e|e.to_string())
}

pub(crate) fn copy_scene_inner(conn:&Connection,scene_id:&str,destination_board_id:&str,name:&str)->Result<String,String> {
    let states=super::soundstage::scene_tracks(conn,scene_id)?;
    let id=insert_scene(conn,destination_board_id,name)?;
    for (index,state) in states.iter().enumerate() {
        super::soundstage::attach_track(conn,destination_board_id,&state.track_id)?;
        conn.execute("INSERT INTO scene_tracks(scene_id,track_id,enabled,volume,selected_clip_id,sort_order) VALUES(?1,?2,?3,?4,?5,?6)",params![id,state.track_id,state.enabled,state.volume,state.selected_clip_id,(index+1)*1000]).map_err(|e|e.to_string())?;
    }
    let siblings=fetch_siblings(conn,"scenes",Some("board_id"),Some(destination_board_id),&id)?;
    if let Some(index)=siblings.iter().position(|(id,_)|id==scene_id){reorder_item(conn,"scenes",Some("board_id"),Some(destination_board_id),&id,index+1)?;}
    Ok(id)
}

pub fn copy_scene(conn:&Connection,scene_id:&str,destination_board_id:&str,name:&str)->Result<String,String> {
    let tx=conn.unchecked_transaction().map_err(|e|e.to_string())?;
    let id=copy_scene_inner(conn,scene_id,destination_board_id,name)?;
    tx.commit().map_err(|e|e.to_string())?;Ok(id)
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

pub fn update_clip_volume(conn: &Connection, id: &str, volume: f64) -> Result<(), String> {
    conn.execute(
        "UPDATE clips SET volume = ?1 WHERE id = ?2",
        params![volume, id],
    )
    .map_err(|e| format!("Failed to update clip volume: {e}"))?;
    Ok(())
}

/// Writes a complete clip ordering for a track in one transaction:
/// `ordered_ids[i]` gets sort_order (i+1)*SORT_GAP. Replaces the O(n²)
/// per-clip reorder loop for drag-and-drop.
pub fn set_clip_order(conn: &Connection, track_id: &str, ordered_ids: &[String]) -> Result<(), String> {
    let mut stmt=conn.prepare("SELECT id FROM clips WHERE track_id=?1").map_err(|e|e.to_string())?;
    let existing=stmt.query_map([track_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<std::collections::HashSet<_>,_>>().map_err(|e|e.to_string())?;
    let proposed:std::collections::HashSet<_>=ordered_ids.iter().cloned().collect();
    if proposed.len()!=ordered_ids.len()||existing!=proposed{return Err("Clip order must include every clip exactly once".into());}
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
