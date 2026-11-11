use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::database::{now_millis, SORT_GAP};

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

pub fn duplicate_configuration(conn:&Connection,source_id:&str,new_name:&str)->Result<String,String> {
    let tx=conn.unchecked_transaction().map_err(|e|e.to_string())?;
    let new_id=insert_configuration(conn,new_name)?;
    let boards=super::soundstage::load_boards(conn,source_id)?;
    for board in boards {
        let id=super::hierarchy::insert_board(conn,&board.name,&new_id)?;
        for track_id in board.track_ids {super::soundstage::attach_track(conn,&id,&track_id)?;}
        for scene in board.scenes {super::hierarchy::copy_scene_inner(conn,&scene.id,&id,&scene.name)?;}
    }
    tx.commit().map_err(|e|e.to_string())?;Ok(new_id)
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

        hierarchy::insert_board(&conn, "My Board", &config_id).expect("insert board");

        let board_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM boards", [], |row| row.get(0))
            .expect("count");
        assert_eq!(board_count, 1);

        delete_configuration(&conn, &config_id).expect("delete");

        let board_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM boards", [], |row| row.get(0))
            .expect("count");
        assert_eq!(board_count, 0);
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
    fn duplicate_configuration_copies_scenes_and_shares_tracks() {
        let conn = open_memory_db();
        let config_id = ensure_test_config(&conn);

        let entry_id = crate::database::library::insert_library_entry(
            &conn, "/tmp/test.mp3", None, "Test Sound", None, None, None, "music",
        ).expect("insert library entry");

        let board_id = hierarchy::insert_board(&conn, "Battle", &config_id).expect("board");
        let scene_id = hierarchy::insert_scene(&conn, &board_id, "Act 1").expect("scene");
        let track_id = hierarchy::insert_track(&conn, &scene_id, "Theme").expect("track");
        let _clip_id = hierarchy::insert_clip(&conn, &track_id, &entry_id).expect("clip");

        let new_config_id = duplicate_configuration(&conn, &config_id, "Battle Copy").expect("duplicate");

        let new_name: String = conn
            .query_row("SELECT name FROM configurations WHERE id = ?1", params![new_config_id], |row| row.get(0))
            .expect("query name");
        assert_eq!(new_name, "Battle Copy");

        let new_boards: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, name FROM boards WHERE configuration_id = ?1")
                .expect("prepare");
            stmt.query_map(params![new_config_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect")
        };
        assert_eq!(new_boards.len(), 1);
        assert_eq!(new_boards[0].1, "Battle");
        assert_ne!(new_boards[0].0, board_id, "should have a new board ID");

        let clip_entry: String = conn
            .query_row(
                "SELECT c.library_entry_id FROM clips c
                 JOIN tracks t ON c.track_id = t.id
                 JOIN scene_tracks st ON st.track_id=t.id
                 JOIN scenes co ON st.scene_id = co.id
                 JOIN boards g ON co.board_id = g.id
                 WHERE g.configuration_id = ?1",
                params![new_config_id],
                |row| row.get(0),
            )
            .expect("query clip entry");
        assert_eq!(clip_entry, entry_id, "clip should reference the same library entry");

        let orig_board_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM boards WHERE configuration_id = ?1",
                params![config_id],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(orig_board_count, 1);
    }
}
