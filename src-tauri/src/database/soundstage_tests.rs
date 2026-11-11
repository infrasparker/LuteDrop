use super::*;
use crate::database::{configurations, hierarchy, library};
use crate::models::TrackMode;

/// Opt-in rehearsal against a disposable SQLite backup, never the running app's DB.
#[test]
#[ignore]
fn rehearse_copied_database_upgrade() {
    let path = std::path::PathBuf::from(
        std::env::var("LUTEDROP_MIGRATION_REHEARSAL").expect("supply a disposable database copy"),
    );
    assert!(
        path.file_name().unwrap() == "rehearsal.db",
        "only a named rehearsal copy may be upgraded"
    );
    let before = Connection::open(&path).unwrap();
    let count = |conn: &Connection, table: &str| {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    let expected = ["groups", "comps", "tracks", "clips", "library_entries"]
        .map(|table| count(&before, table));
    let rows = |conn: &Connection, sql: &str| {
        let mut stmt = conn.prepare(sql).unwrap();
        let columns = stmt.column_count();
        stmt.query_map([], |row| {
            (0..columns)
                .map(|i| row.get::<_, rusqlite::types::Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    let track_sql="SELECT id,name,volume,mode_type,repeat,transition_type,transition_gap_min,transition_gap_max,fade_in,fade_out FROM tracks ORDER BY id";
    let clip_sql = "SELECT id,track_id,library_entry_id,volume,sort_order FROM clips ORDER BY id";
    let tracks = rows(&before, track_sql);
    let clips = rows(&before, clip_sql);
    drop(before);
    let after = open_db(&path).unwrap();
    assert_eq!(
        ["boards", "scenes", "tracks", "clips", "library_entries"].map(|t| count(&after, t)),
        expected
    );
    assert_eq!(rows(&after, track_sql), tracks);
    assert_eq!(rows(&after, clip_sql), clips);
    assert_eq!(count(&after, "scene_tracks"), expected[2]);
    assert_eq!(
        after
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    let state = load_full_state(&after).unwrap();
    assert_eq!(state.tracks.len() as i64, expected[2]);
}

fn setup() -> (Connection, String, String, String, String, String) {
    let conn = open_memory_db();
    let config = ensure_test_config(&conn);
    let board = hierarchy::insert_board(&conn, "Wilderness", &config).unwrap();
    let scene = hierarchy::insert_scene(&conn, &board, "Rainy trail").unwrap();
    let track = create_track(&conn, "Weather").unwrap();
    attach_track(&conn, &board, &track).unwrap();
    let a = library::insert_library_entry(
        &conn,
        "/light.wav",
        None,
        "Light rain",
        None,
        None,
        None,
        "ambiance",
    )
    .unwrap();
    let b = library::insert_library_entry(
        &conn,
        "/heavy.wav",
        None,
        "Heavy rain",
        None,
        None,
        None,
        "ambiance",
    )
    .unwrap();
    let clip_a = hierarchy::insert_clip(&conn, &track, &a).unwrap();
    let clip_b = hierarchy::insert_clip(&conn, &track, &b).unwrap();
    (conn, board, scene, track, clip_a, clip_b)
}

fn setting(track: &str, clip: Option<&str>, volume: f64) -> SceneTrack {
    SceneTrack {
        track_id: track.into(),
        enabled: true,
        volume,
        selected_clip_id: clip.map(str::to_string),
    }
}

#[test]
fn changing_manual_tracks_to_playlists_resets_inherited_scene_starts_only() {
    for manual in ["seamless", "oneShot"] {
        for playlist in ["shuffle", "sequence"] {
            let (conn,board,scene,track,a,b)=setup();
            hierarchy::update_track_mode(&conn,&track,manual,None,None,None,None).unwrap();
            let other_scene=hierarchy::insert_scene(&conn,&board,"Other").unwrap();
            save_scene(&conn,&scene,&[setting(&track,Some(&a),0.4)]).unwrap();
            save_scene(&conn,&other_scene,&[setting(&track,Some(&b),0.7)]).unwrap();
            set_mix(&conn,&track,Some(0.5),Some(Some(&a))).unwrap();
            hierarchy::update_track_mode(&conn,&track,playlist,Some(true),Some("gapless"),None,None).unwrap();
            for (id,volume) in [(&scene,0.4),(&other_scene,0.7)] {
                let saved=scene_tracks(&conn,id).unwrap().remove(0);
                assert_eq!(saved.selected_clip_id,None,"{manual} -> {playlist}");
                assert!(saved.enabled);
                assert_eq!(saved.volume,volume);
            }
            assert!(load_mix(&conn).unwrap().iter().find(|m|m.track_id==track).unwrap().selected_clip_id.is_none());
            // An explicit playlist starting clip is intentional. Ordinary
            // settings edits and switching automatic order do not erase it.
            save_scene(&conn,&scene,&[setting(&track,Some(&b),0.4)]).unwrap();
            hierarchy::update_track_mode(&conn,&track,playlist,Some(false),Some("gap"),Some(2.),Some(5.)).unwrap();
            assert_eq!(scene_tracks(&conn,&scene).unwrap()[0].selected_clip_id.as_deref(),Some(b.as_str()));
        }
    }
}

#[test]
fn scene_layout_edits_preserve_other_scenes_and_optional_card_settings() {
    let (conn, board, scene, track, a, _) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.3)]).unwrap();
    let other = hierarchy::insert_scene(&conn, &board, "Other scene").unwrap();
    attach_scene_track(&conn, &other, &track).unwrap();
    assert!(
        !scene_tracks(&conn, &other).unwrap()[0].enabled,
        "new optional card should start disabled"
    );
    let second = create_track(&conn, "Birds").unwrap();
    attach_scene_track(&conn, &other, &second).unwrap();
    set_scene_track_order(&conn, &other, &second, 0).unwrap();
    assert_eq!(scene_tracks(&conn, &other).unwrap()[0].track_id, second);
    assert_eq!(scene_tracks(&conn, &scene).unwrap()[0].volume, 0.3);
    detach_scene_track(&conn, &other, &track).unwrap();
    assert_eq!(scene_tracks(&conn, &scene).unwrap()[0].track_id, track);
    assert_eq!(load_track_by_id(&conn, &track).unwrap().clips[0].id, a);
}

#[test]
fn independent_scene_copy_maps_selected_clip_and_leaves_shared_track_untouched() {
    let (conn, board, scene, track, _, b) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&b), 0.4)]).unwrap();
    let other = hierarchy::insert_scene(&conn, &board, "Other scene").unwrap();
    save_scene(&conn, &other, &[setting(&track, Some(&b), 0.8)]).unwrap();
    let copied = make_scene_track_independent(&conn, &scene, &track).unwrap();
    let ref_copy = scene_tracks(&conn, &scene).unwrap().remove(0);
    assert_eq!(ref_copy.track_id, copied);
    assert_eq!(ref_copy.volume, 0.4);
    let copy = load_track_by_id(&conn, &copied).unwrap();
    assert_eq!(
        ref_copy.selected_clip_id.as_deref(),
        Some(copy.clips[1].id.as_str())
    );
    hierarchy::rename_track(&conn, &copied, "Private weather").unwrap();
    assert_eq!(load_track_by_id(&conn, &track).unwrap().name, "Weather");
    assert_eq!(scene_tracks(&conn, &other).unwrap()[0].track_id, track);
    remove_board_track(&conn, &board, &track).unwrap();
    assert!(scene_tracks(&conn, &other).unwrap().is_empty());
    assert_eq!(scene_tracks(&conn, &scene).unwrap()[0].track_id, copied);
    assert!(
        load_track_by_id(&conn, &track).is_ok(),
        "board removal deleted the inventory track"
    );
}

#[test]
fn inventory_exists_without_scenes_or_boards() {
    let conn = open_memory_db();
    ensure_test_config(&conn);
    let id = create_track(&conn, "Unused track").unwrap();
    let state = load_full_state(&conn).unwrap();
    assert!(state.boards.is_empty());
    assert_eq!(state.tracks[0].id, id);
    assert!(references(&conn, &id, None).unwrap().is_empty());
    delete_track(&conn, &id, false).unwrap();
    assert!(load_inventory(&conn).unwrap().is_empty());
}

#[test]
fn scenes_share_definitions_but_keep_independent_mix_settings() {
    let (conn, board, scene, track, a, b) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.3)]).unwrap();
    let second = hierarchy::insert_scene(&conn, &board, "Storm").unwrap();
    save_scene(&conn, &second, &[setting(&track, Some(&b), 0.8)]).unwrap();
    hierarchy::rename_track(&conn, &track, "Shared weather").unwrap();
    set_mix(&conn, &track, Some(0.1), Some(Some(&b))).unwrap();
    let state = load_full_state(&conn).unwrap();
    assert_eq!(state.tracks.len(), 1);
    assert_eq!(state.tracks[0].name, "Shared weather");
    assert_eq!(scene_tracks(&conn, &scene).unwrap()[0].volume, 0.3);
    assert_eq!(scene_tracks(&conn, &second).unwrap()[0].volume, 0.8);
    assert_eq!(
        scene_tracks(&conn, &scene).unwrap()[0]
            .selected_clip_id
            .as_deref(),
        Some(a.as_str())
    );
    assert_eq!(load_for_playback(&conn, &track).unwrap().volume, 0.1);
    assert_eq!(state.tracks[0].volume, 1.0);
    assert_eq!(references(&conn, &track, None).unwrap().len(), 2);
}

#[test]
fn selected_clip_reordering_does_not_change_scene_defaults() {
    let (conn, _, scene, track, a, b) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.4)]).unwrap();
    hierarchy::set_clip_order(&conn, &track, &[b.clone(), a.clone()]).unwrap();
    assert_eq!(load_inventory(&conn).unwrap()[0].clips[0].id, b);
    assert_eq!(
        scene_tracks(&conn, &scene).unwrap()[0]
            .selected_clip_id
            .as_deref(),
        Some(a.as_str())
    );
    assert!(hierarchy::set_clip_order(&conn, &track, &[a.clone(), a]).is_err());
}

#[test]
fn removing_referenced_clip_requires_resolution_and_preserves_unaffected_scenes() {
    let (conn, board, scene, track, a, b) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.4)]).unwrap();
    let other = hierarchy::insert_scene(&conn, &board, "Heavy").unwrap();
    save_scene(&conn, &other, &[setting(&track, Some(&b), 0.9)]).unwrap();
    assert!(remove_clip(&conn, &track, &a, None, false).is_err());
    assert!(remove_clip(&conn, &track, &a, Some(&a), false).is_err());
    assert!(remove_clip(&conn, &track, &a, Some("foreign clip"), false).is_err());
    assert_eq!(load_inventory(&conn).unwrap()[0].clips.len(), 2);
    remove_clip(&conn, &track, &a, Some(&b), false).unwrap();
    let resolved = scene_tracks(&conn, &scene).unwrap();
    assert!(resolved[0].enabled);
    assert_eq!(resolved[0].selected_clip_id.as_deref(), Some(b.as_str()));
    assert_eq!(resolved[0].volume, 0.4);
    assert_eq!(scene_tracks(&conn, &other).unwrap()[0].volume, 0.9);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM library_entries", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn disabling_references_is_explicit_and_removal_rolls_back_on_failure() {
    let (conn, _, scene, track, a, _) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.4)]).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_remove BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT,'simulated failure'); END;").unwrap();
    assert!(remove_clip(&conn, &track, &a, None, true).is_err());
    assert!(scene_tracks(&conn, &scene).unwrap()[0].enabled);
    conn.execute_batch("DROP TRIGGER fail_remove").unwrap();
    remove_clip(&conn, &track, &a, None, true).unwrap();
    let state = scene_tracks(&conn, &scene).unwrap();
    assert!(!state[0].enabled);
    assert!(state[0].selected_clip_id.is_none());
}

#[test]
fn deleting_board_and_scene_retains_inventory_while_deleting_track_requires_consent() {
    let (conn, board, scene, track, a, _) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.4)]).unwrap();
    assert!(delete_track(&conn, &track, false).is_err());
    assert!(detach_track(&conn, &board, &track).is_err());
    hierarchy::delete_board(&conn, &board).unwrap();
    assert_eq!(load_inventory(&conn).unwrap()[0].clips.len(), 2);
    assert!(references(&conn, &track, None).unwrap().is_empty());
    delete_track(&conn, &track, false).unwrap();
    assert!(load_inventory(&conn).unwrap().is_empty());
}

#[test]
fn confirmed_track_deletion_removes_all_references_in_other_configurations() {
    let (conn, board, scene, track, a, _) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.4)]).unwrap();
    let config = configurations::insert_configuration(&conn, "Other campaign").unwrap();
    let other_board = hierarchy::insert_board(&conn, "City", &config).unwrap();
    let other_scene = hierarchy::copy_scene(&conn, &scene, &other_board, "Rainy city").unwrap();
    assert_eq!(references(&conn, &track, Some(&a)).unwrap().len(), 2);
    delete_track(&conn, &track, true).unwrap();
    assert!(scene_tracks(&conn, &scene).unwrap().is_empty());
    assert!(scene_tracks(&conn, &other_scene).unwrap().is_empty());
    assert!(
        load_boards(&conn, &get_active_configuration_id(&conn).unwrap()).unwrap()[0]
            .track_ids
            .is_empty()
    );
    assert!(detach_track(&conn, &board, &track).is_ok());
}

#[test]
fn duplicate_track_is_independent_and_preserves_all_playback_settings() {
    let (conn, _, _, track, a, b) = setup();
    hierarchy::update_track_mode(
        &conn,
        &track,
        "shuffle",
        Some(true),
        Some("gap"),
        Some(15.0),
        Some(80.0),
    )
    .unwrap();
    hierarchy::update_track_fades(&conn, &track, 1.5, 2.5).unwrap();
    hierarchy::update_clip_volume(&conn, &a, 0.25).unwrap();
    hierarchy::set_clip_order(&conn, &track, &[b, a]).unwrap();
    let copied = duplicate_track(&conn, &track, "Independent weather").unwrap();
    let original = load_track_by_id(&conn, &track).unwrap();
    let copy = load_track_by_id(&conn, &copied).unwrap();
    assert_eq!(
        serde_json::to_value(&original.mode).unwrap(),
        serde_json::to_value(&copy.mode).unwrap()
    );
    assert_eq!((copy.fade_in, copy.fade_out), (1.5, 2.5));
    for (source, copied) in original.clips.iter().zip(copy.clips.iter()) {
        assert_ne!(source.id, copied.id);
        assert_eq!(source.library_entry_id, copied.library_entry_id);
        assert_eq!(source.volume, copied.volume);
    }
    assert!(references(&conn, &copied, None).unwrap().is_empty());
}

#[test]
fn scene_and_board_ordering_and_copies_keep_shared_references() {
    let (conn, board, scene, track, a, _) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.4)]).unwrap();
    let other_track = create_track(&conn, "Music").unwrap();
    attach_track(&conn, &board, &other_track).unwrap();
    set_board_track_order(&conn, &board, &other_track, 0).unwrap();
    let config = get_active_configuration_id(&conn).unwrap();
    assert_eq!(
        load_boards(&conn, &config).unwrap()[0].track_ids,
        vec![other_track, track.clone()]
    );
    let copy = hierarchy::copy_scene(&conn, &scene, &board, "Copy").unwrap();
    assert_eq!(load_inventory(&conn).unwrap().len(), 2);
    hierarchy::reorder_scene(&conn, &copy, 0).unwrap();
    assert_eq!(load_boards(&conn, &config).unwrap()[0].scenes[0].id, copy);
    let other = hierarchy::insert_board(&conn, "City", &config).unwrap();
    hierarchy::move_scene(&conn, &copy, &other).unwrap();
    let boards = load_boards(&conn, &config).unwrap();
    assert_eq!(boards[1].track_ids, vec![track]);
    assert_eq!(boards[1].scenes[0].tracks[0].volume, 0.4);
}

#[test]
fn scene_validation_is_atomic_and_rejects_foreign_selection_or_track() {
    let (conn, _, scene, track, a, _) = setup();
    save_scene(&conn, &scene, &[setting(&track, Some(&a), 0.4)]).unwrap();
    let unused = create_track(&conn, "Unattached").unwrap();
    for invalid in [
        vec![setting(&track, Some("foreign"), 0.2)],
        vec![setting(&unused, None, 0.1)],
        vec![
            setting(&track, Some(&a), 0.2),
            setting(&track, Some(&a), 0.3),
        ],
        vec![setting(&track, None, f64::NAN)],
    ] {
        assert!(save_scene(&conn, &scene, &invalid).is_err());
        assert_eq!(scene_tracks(&conn, &scene).unwrap()[0].volume, 0.4);
    }
}

#[test]
fn selected_loop_uses_its_own_library_markers_and_inventory_stays_in_order() {
    let (conn, _, _, track, a, b) = setup();
    let original = load_track_by_id(&conn, &track).unwrap();
    library::update_library_entry_loop(
        &conn,
        &original.clips[0].library_entry_id,
        Some(1.0),
        Some(4.0),
    )
    .unwrap();
    library::update_library_entry_loop(
        &conn,
        &original.clips[1].library_entry_id,
        Some(2.0),
        Some(7.0),
    )
    .unwrap();
    set_mix(&conn, &track, None, Some(Some(&b))).unwrap();
    match load_for_playback(&conn, &track).unwrap().mode {
        TrackMode::Seamless {
            loop_start,
            loop_end,
        } => assert_eq!((loop_start, loop_end), (2.0, 7.0)),
        _ => panic!("loop mode"),
    }
    assert_eq!(load_inventory(&conn).unwrap()[0].clips[0].id, a);
    for kind in ["sequence", "shuffle", "oneShot"] {
        hierarchy::update_track_mode(
            &conn,
            &track,
            kind,
            Some(true),
            Some("gap"),
            Some(20.0),
            Some(90.0),
        )
        .unwrap();
        let mode = load_track_by_id(&conn, &track).unwrap().mode;
        assert_eq!(serde_json::to_value(mode).unwrap()["type"], kind);
    }
}

fn legacy(conn: &Connection) {
    super::super::legacy_schema::init_legacy_schema(conn).unwrap();
    conn.execute_batch("INSERT INTO meta VALUES('schema_version','17');
      INSERT INTO configurations VALUES('config','Campaign','0',1000);
      INSERT INTO meta VALUES('active_configuration_id','config');
      INSERT INTO groups VALUES('board','Village',1000,'config');
      INSERT INTO comps VALUES('scene','board','Tavern',1000);
      INSERT INTO tracks(id,comp_id,name,volume,mode_type,repeat,transition_type,transition_gap_min,transition_gap_max,fade_in,fade_out,sort_order)
        VALUES('track','scene','Music',.4,'shuffle',1,'gap',20,90,1.5,3,1000);
      INSERT INTO tracks(id,comp_id,name,volume,mode_type,repeat,sort_order) VALUES('loop','scene','Music',.6,'seamless',1,2000);").unwrap();
    let entry =
        library::insert_library_entry(conn, "/audio.wav", None, "Audio", None, None, None, "music")
            .unwrap();
    conn.execute(
        "INSERT INTO clips VALUES('clip','track',?1,.7,1000)",
        [&entry],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO clips VALUES('loop-clip','loop',?1,.8,1000)",
        [&entry],
    )
    .unwrap();
}

#[test]
fn v17_migration_preserves_ids_clips_modes_order_and_creates_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    legacy(&conn);
    drop(conn);
    let conn = open_db(&path).unwrap();
    assert_eq!(read_schema_version(&conn), 18);
    let state = load_full_state(&conn).unwrap();
    assert_eq!(state.boards[0].id, "board");
    assert_eq!(state.boards[0].scenes[0].id, "scene");
    assert_eq!(state.boards[0].track_ids, vec!["track", "loop"]);
    assert_eq!(
        state.tracks.len(),
        2,
        "same-named definitions stay distinct"
    );
    let source = state.tracks.iter().find(|t| t.id == "track").unwrap();
    assert_eq!(source.clips[0].id, "clip");
    assert_eq!(source.clips[0].volume, 0.7);
    assert_eq!((source.fade_in, source.fade_out), (1.5, 3.0));
    match source.mode {
        TrackMode::Shuffle {
            repeat,
            transition: crate::models::TrackTransition::Gap { min, max },
        } => {
            assert!(repeat);
            assert_eq!((min, max), (20., 90.));
        }
        _ => panic!("lost shuffle mode"),
    }
    let settings = scene_tracks(&conn, "scene").unwrap();
    assert_eq!(settings[0].volume, 0.4);
    assert_eq!(settings[1].selected_clip_id.as_deref(), Some("loop-clip"));
    assert!(!std::fs::read_dir(backup::backup_dir_for(&path))
        .unwrap()
        .collect::<Vec<_>>()
        .is_empty());
    let issues: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(issues, 0);
    drop(conn);
    let reopened = open_db(&path).unwrap();
    assert_eq!(load_inventory(&reopened).unwrap().len(), 2);
}

#[test]
fn v18_migration_failure_restores_legacy_tables_and_version() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    legacy(&conn);
    conn.execute_batch("CREATE TABLE scene_tracks(block_migration TEXT)")
        .unwrap();
    assert!(run_migrations(&conn).is_err());
    assert_eq!(read_schema_version(&conn), 17);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM tracks WHERE comp_id='scene'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM clips", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(conn.prepare("SELECT * FROM groups").is_ok());
}
