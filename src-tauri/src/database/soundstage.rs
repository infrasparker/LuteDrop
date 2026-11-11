//! Shared track inventory, board membership, saved scenes, and the current mix.
use super::*;
use crate::models::{SceneReference, SceneTrack, TrackMix};
use rusqlite::OptionalExtension;
#[cfg(test)]
#[path = "soundstage_tests.rs"]
mod tests;

/// Preserve every old ID and setting. Track definitions become independent of
/// scenes; old compositions become scenes with all their tracks enabled.
pub fn migrate_v17_to_v18(conn: &Connection) -> SqlResult<()> {
    let done: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='board_tracks'",
        [],
        |r| r.get(0),
    )?;
    if done {
        return ensure_indexes(conn);
    }
    let legacy_exists: bool = conn.query_row(
        "SELECT COUNT(*)>0 FROM sqlite_master WHERE type='table' AND name='groups'",
        [],
        |r| r.get(0),
    )?;
    if !legacy_exists {
        super::legacy_schema::init_legacy_schema(conn)?;
    }
    conn.execute_batch("
        ALTER TABLE groups RENAME TO boards;
        ALTER TABLE comps RENAME TO scenes;
        ALTER TABLE scenes RENAME COLUMN group_id TO board_id;
        CREATE TEMP TABLE legacy_scene_tracks AS
          SELECT t.comp_id AS scene_id, t.id AS track_id, t.volume, t.sort_order,
            CASE WHEN t.mode_type='seamless' THEN (SELECT id FROM clips WHERE track_id=t.id ORDER BY sort_order LIMIT 1) END AS selected_clip_id
          FROM tracks t;
        CREATE TABLE tracks_v18 (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, volume REAL NOT NULL DEFAULT 1.0,
          mode_type TEXT NOT NULL DEFAULT 'sequence', repeat INTEGER DEFAULT 1,
          transition_type TEXT DEFAULT 'gapless', transition_gap_min REAL, transition_gap_max REAL,
          fade_in REAL NOT NULL DEFAULT 0.0, fade_out REAL NOT NULL DEFAULT 0.0, sort_order INTEGER NOT NULL
        );
        INSERT INTO tracks_v18 SELECT id,name,volume,mode_type,repeat,transition_type,transition_gap_min,transition_gap_max,fade_in,fade_out,sort_order FROM tracks;
        CREATE TABLE clips_v18 (
          id TEXT PRIMARY KEY, track_id TEXT NOT NULL REFERENCES tracks_v18(id) ON DELETE CASCADE,
          library_entry_id TEXT NOT NULL REFERENCES library_entries(id), volume REAL NOT NULL DEFAULT 1.0, sort_order INTEGER NOT NULL
        );
        INSERT INTO clips_v18 SELECT * FROM clips;
        DROP TABLE clips;
        DROP TABLE tracks;
        ALTER TABLE tracks_v18 RENAME TO tracks;
        ALTER TABLE clips_v18 RENAME TO clips;
        CREATE TABLE board_tracks (
          board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
          track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
          sort_order INTEGER NOT NULL, PRIMARY KEY(board_id,track_id)
        );
        CREATE TABLE scene_tracks (
          scene_id TEXT NOT NULL REFERENCES scenes(id) ON DELETE CASCADE,
          track_id TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
          enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
          volume REAL NOT NULL DEFAULT 1.0 CHECK(volume>=0 AND volume<=1),
          selected_clip_id TEXT REFERENCES clips(id) ON DELETE RESTRICT,
          sort_order INTEGER NOT NULL, PRIMARY KEY(scene_id,track_id)
        );
        INSERT INTO scene_tracks SELECT scene_id,track_id,1,volume,selected_clip_id,sort_order FROM legacy_scene_tracks;
        INSERT INTO board_tracks SELECT s.board_id,st.track_id,
          ROW_NUMBER() OVER (PARTITION BY s.board_id ORDER BY s.sort_order,st.sort_order)*1000
          FROM legacy_scene_tracks st JOIN scenes s ON s.id=st.scene_id;
        CREATE TABLE track_mix (
          track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
          volume REAL NOT NULL CHECK(volume>=0 AND volume<=1),
          selected_clip_id TEXT REFERENCES clips(id) ON DELETE SET NULL
        );
        INSERT INTO track_mix SELECT track_id,volume,selected_clip_id FROM legacy_scene_tracks;
        DROP TABLE legacy_scene_tracks;
        CREATE INDEX idx_scene_tracks_track ON scene_tracks(track_id);
        CREATE INDEX idx_scene_tracks_clip ON scene_tracks(selected_clip_id);
        CREATE INDEX idx_board_tracks_track ON board_tracks(track_id);
        CREATE INDEX idx_clips_track_id ON clips(track_id);
        CREATE INDEX idx_clips_library_entry_id ON clips(library_entry_id);
        DELETE FROM meta WHERE key='active_scene_id';
        DROP INDEX IF EXISTS idx_groups_configuration_id;
        DROP INDEX IF EXISTS idx_comps_group_id;
    ")?;
    ensure_indexes(conn)
}

pub fn ensure_indexes(conn: &Connection) -> SqlResult<()> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_boards_configuration_id ON boards(configuration_id);
      CREATE INDEX IF NOT EXISTS idx_scenes_board_id ON scenes(board_id);
      CREATE INDEX IF NOT EXISTS idx_board_tracks_track ON board_tracks(track_id);
      CREATE INDEX IF NOT EXISTS idx_scene_tracks_track ON scene_tracks(track_id);
      CREATE INDEX IF NOT EXISTS idx_scene_tracks_clip ON scene_tracks(selected_clip_id);
      CREATE INDEX IF NOT EXISTS idx_clips_track_id ON clips(track_id);
      CREATE INDEX IF NOT EXISTS idx_clips_library_entry_id ON clips(library_entry_id);
      CREATE INDEX IF NOT EXISTS idx_library_entries_content_hash ON library_entries(content_hash);
      CREATE INDEX IF NOT EXISTS idx_library_entry_tags_tag_id ON library_entry_tags(tag_id);",
    )?;
    conn.execute_batch(include_str!("scene_playback_rules.sql"))
}

fn check_exists(conn: &Connection, table: &str, id: &str) -> Result<(), String> {
    if conn
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE id=?1"),
            [id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())?
        == 0
    {
        let label = match table {
            "scenes" => "Scene",
            "boards" => "Board",
            "tracks" => "Track",
            _ => "Item",
        };
        return Err(format!("{label} not found: {id}"));
    }
    Ok(())
}

pub fn load_inventory(conn: &Connection) -> Result<Vec<Track>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {TRACK_COLUMNS} FROM tracks ORDER BY name COLLATE NOCASE, id"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut clips: HashMap<String, Vec<ClipWithLoops>> = HashMap::new();
    for (id, clip) in load_clips_resolved(conn, "?1 IS NOT NULL", "")? {
        clips.entry(id).or_default().push(clip);
    }
    Ok(rows
        .into_iter()
        .map(
            |row: (
                String,
                String,
                f64,
                String,
                Option<i64>,
                Option<String>,
                Option<f64>,
                Option<f64>,
                f64,
                f64,
            )| {
                let own = clips.remove(&row.0).unwrap_or_default();
                assemble_track(row, own)
            },
        )
        .collect())
}

pub fn load_mix(conn: &Connection) -> Result<Vec<TrackMix>, String> {
    let mut stmt=conn.prepare("SELECT t.id,COALESCE(m.volume,t.volume),m.selected_clip_id FROM tracks t LEFT JOIN track_mix m ON m.track_id=t.id ORDER BY t.name").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TrackMix {
                track_id: r.get(0)?,
                volume: r.get(1)?,
                selected_clip_id: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn scene_tracks(conn: &Connection, scene_id: &str) -> Result<Vec<SceneTrack>, String> {
    check_exists(conn, "scenes", scene_id)?;
    let mut stmt=conn.prepare("SELECT track_id,enabled,volume,selected_clip_id FROM scene_tracks WHERE scene_id=?1 ORDER BY sort_order").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map([scene_id], |r| {
            Ok(SceneTrack {
                track_id: r.get(0)?,
                enabled: r.get(1)?,
                volume: r.get(2)?,
                selected_clip_id: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn load_boards(conn: &Connection, configuration_id: &str) -> Result<Vec<Board>, String> {
    let mut stmt = conn
        .prepare("SELECT id,name FROM boards WHERE configuration_id=?1 ORDER BY sort_order")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([configuration_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|(id, name)| {
            let mut scene_stmt = conn
                .prepare("SELECT id,name FROM scenes WHERE board_id=?1 ORDER BY sort_order")
                .map_err(|e| e.to_string())?;
            let scene_rows = scene_stmt
                .query_map([&id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<SqlResult<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            let scenes = scene_rows
                .into_iter()
                .map(|(id, name)| {
                    Ok(Scene {
                        id: id.clone(),
                        name,
                        tracks: scene_tracks(conn, &id)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut track_stmt = conn
                .prepare("SELECT track_id FROM board_tracks WHERE board_id=?1 ORDER BY sort_order")
                .map_err(|e| e.to_string())?;
            let track_ids = track_stmt
                .query_map([&id], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<SqlResult<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            Ok(Board {
                id,
                name,
                scenes,
                track_ids,
            })
        })
        .collect()
}

pub fn create_track(conn: &Connection, name: &str) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    conn.execute("INSERT INTO tracks(id,name,volume,mode_type,repeat,transition_type,fade_in,fade_out,sort_order) VALUES(?1,?2,1,'seamless',1,'gapless',0,0,1000)",params![id,name]).map_err(|e|e.to_string())?;
    Ok(id)
}

pub fn attach_track(conn: &Connection, board_id: &str, track_id: &str) -> Result<(), String> {
    check_exists(conn, "boards", board_id)?;
    check_exists(conn, "tracks", track_id)?;
    conn.execute("INSERT OR IGNORE INTO board_tracks(board_id,track_id,sort_order) VALUES(?1,?2,(SELECT COALESCE(MAX(sort_order),0)+1000 FROM board_tracks WHERE board_id=?1))",params![board_id,track_id]).map_err(|e|e.to_string())?;
    Ok(())
}

/// Layout membership is independent of whether this track starts playing.
/// Caller owns the transaction, so track creation and scene attachment are atomic.
pub fn attach_scene_track_inner(
    conn: &Connection,
    scene_id: &str,
    track_id: &str,
) -> Result<(), String> {
    check_exists(conn, "scenes", scene_id)?;
    check_exists(conn, "tracks", track_id)?;
    let board: String = conn
        .query_row("SELECT board_id FROM scenes WHERE id=?1", [scene_id], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    attach_track(conn, &board, track_id)?;
    conn.execute("INSERT OR IGNORE INTO scene_tracks(scene_id,track_id,enabled,volume,selected_clip_id,sort_order) SELECT ?1,t.id,0,COALESCE(m.volume,t.volume),CASE WHEN t.mode_type IN ('seamless','oneShot') THEN m.selected_clip_id END,(SELECT COALESCE(MAX(sort_order),0)+1000 FROM scene_tracks WHERE scene_id=?1) FROM tracks t LEFT JOIN track_mix m ON m.track_id=t.id WHERE t.id=?2",params![scene_id,track_id]).map_err(|e|e.to_string())?;
    Ok(())
}

pub fn attach_scene_track(conn: &Connection, scene_id: &str, track_id: &str) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    attach_scene_track_inner(conn, scene_id, track_id)?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn detach_scene_track(conn: &Connection, scene_id: &str, track_id: &str) -> Result<(), String> {
    check_exists(conn, "scenes", scene_id)?;
    conn.execute(
        "DELETE FROM scene_tracks WHERE scene_id=?1 AND track_id=?2",
        params![scene_id, track_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_scene_track_order(
    conn: &Connection,
    scene_id: &str,
    track_id: &str,
    new_index: usize,
) -> Result<(), String> {
    let mut settings = scene_tracks(conn, scene_id)?;
    let old = settings
        .iter()
        .position(|state| state.track_id == track_id)
        .ok_or("Track is not in this scene")?;
    let state = settings.remove(old);
    settings.insert(new_index.min(settings.len()), state);
    save_scene(conn, scene_id, &settings)
}

pub fn detach_track(conn: &Connection, board_id: &str, track_id: &str) -> Result<(), String> {
    let count:i64=conn.query_row("SELECT COUNT(*) FROM scene_tracks st JOIN scenes s ON s.id=st.scene_id WHERE s.board_id=?1 AND st.track_id=?2",params![board_id,track_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count > 0 {
        return Err(
            "This track is used by scenes on this board. Remove it from those scenes first.".into(),
        );
    }
    conn.execute(
        "DELETE FROM board_tracks WHERE board_id=?1 AND track_id=?2",
        params![board_id, track_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_board_track(conn: &Connection, board_id: &str, track_id: &str) -> Result<(), String> {
    check_exists(conn, "boards", board_id)?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scene_tracks WHERE track_id=?1 AND scene_id IN (SELECT id FROM scenes WHERE board_id=?2)",params![track_id,board_id]).map_err(|e|e.to_string())?;
    detach_track(conn, board_id, track_id)?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn set_board_track_order(
    conn: &Connection,
    board_id: &str,
    track_id: &str,
    new_index: usize,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare("SELECT track_id FROM board_tracks WHERE board_id=?1 ORDER BY sort_order")
        .map_err(|e| e.to_string())?;
    let mut ids = stmt
        .query_map([board_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let old = ids
        .iter()
        .position(|id| id == track_id)
        .ok_or("Track is not on this board")?;
    let id = ids.remove(old);
    ids.insert(new_index.min(ids.len()), id);
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE board_tracks SET sort_order=?1 WHERE board_id=?2 AND track_id=?3",
            params![(index + 1) * 1000, board_id, id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn validate_clip(conn: &Connection, track_id: &str, clip_id: Option<&str>) -> Result<(), String> {
    if let Some(id) = clip_id {
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*)>0 FROM clips WHERE id=?1 AND track_id=?2",
                params![id, track_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exists {
            return Err("The selected clip no longer belongs to this track".into());
        }
    }
    Ok(())
}

pub fn save_scene_inner(
    conn: &Connection,
    scene_id: &str,
    tracks: &[SceneTrack],
) -> Result<(), String> {
    check_exists(conn, "scenes", scene_id)?;
    let board_id: String = conn
        .query_row("SELECT board_id FROM scenes WHERE id=?1", [scene_id], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    for state in tracks {
        check_exists(conn, "tracks", &state.track_id)?;
        if !seen.insert(&state.track_id) {
            return Err("A scene cannot contain a track twice".into());
        }
        if !state.volume.is_finite() || !(0.0..=1.0).contains(&state.volume) {
            return Err("Scene volume must be between 0 and 1".into());
        }
        validate_clip(conn, &state.track_id, state.selected_clip_id.as_deref())?;
        let member: bool = conn
            .query_row(
                "SELECT COUNT(*)>0 FROM board_tracks WHERE board_id=?1 AND track_id=?2",
                params![board_id, state.track_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !member {
            return Err("Add the track to this board before saving it in a scene".into());
        }
    }
    conn.execute("DELETE FROM scene_tracks WHERE scene_id=?1", [scene_id])
        .map_err(|e| e.to_string())?;
    for (index, state) in tracks.iter().enumerate() {
        conn.execute("INSERT INTO scene_tracks(scene_id,track_id,enabled,volume,selected_clip_id,sort_order) VALUES(?1,?2,?3,?4,?5,?6)",params![scene_id,state.track_id,state.enabled,state.volume,state.selected_clip_id,(index+1)*1000]).map_err(|e|e.to_string())?;
    }
    Ok(())
}

pub fn save_scene(conn: &Connection, scene_id: &str, tracks: &[SceneTrack]) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    save_scene_inner(conn, scene_id, tracks)?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn load_for_playback(conn: &Connection, track_id: &str) -> Result<Track, String> {
    let mut track = load_track_by_id(conn, track_id)?;
    let mix: Option<(f64, Option<String>)> = conn
        .query_row(
            "SELECT volume,selected_clip_id FROM track_mix WHERE track_id=?1",
            [track_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((volume, selected)) = mix {
        track.volume = volume;
        if let (crate::models::TrackMode::Seamless { .. }, Some(clip_id)) = (&track.mode, selected)
        {
            let markers:(Option<f64>,Option<f64>)=conn.query_row("SELECT e.loop_start,e.loop_end FROM clips c JOIN library_entries e ON e.id=c.library_entry_id WHERE c.id=?1 AND c.track_id=?2",params![clip_id,track_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
            track.mode = crate::models::TrackMode::Seamless {
                loop_start: markers.0.unwrap_or(0.0),
                loop_end: markers.1.unwrap_or(0.0),
            };
        }
    }
    Ok(track)
}

pub fn set_mix(
    conn: &Connection,
    track_id: &str,
    volume: Option<f64>,
    selected: Option<Option<&str>>,
) -> Result<(), String> {
    check_exists(conn, "tracks", track_id)?;
    if let Some(v) = volume {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err("Volume must be between 0 and 1".into());
        }
    }
    if let Some(clip) = selected {
        validate_clip(conn, track_id, clip)?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO track_mix(track_id,volume) SELECT id,volume FROM tracks WHERE id=?1",
        [track_id],
    )
    .map_err(|e| e.to_string())?;
    if let Some(v) = volume {
        conn.execute(
            "UPDATE track_mix SET volume=?1 WHERE track_id=?2",
            params![v, track_id],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(clip) = selected {
        conn.execute(
            "UPDATE track_mix SET selected_clip_id=?1 WHERE track_id=?2",
            params![clip, track_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn references(
    conn: &Connection,
    track_id: &str,
    clip_id: Option<&str>,
) -> Result<Vec<SceneReference>, String> {
    let mut stmt=conn.prepare("SELECT s.id,s.name,b.name,c.name,st.selected_clip_id FROM scene_tracks st JOIN scenes s ON s.id=st.scene_id JOIN boards b ON b.id=s.board_id JOIN configurations c ON c.id=b.configuration_id WHERE st.track_id=?1 AND (?2 IS NULL OR st.selected_clip_id=?2) ORDER BY c.name,b.name,s.name").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map(params![track_id, clip_id], |r| {
            Ok(SceneReference {
                scene_id: r.get(0)?,
                scene_name: r.get(1)?,
                board_name: r.get(2)?,
                configuration_name: r.get(3)?,
                selected_clip_id: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn remove_clip(
    conn: &Connection,
    track_id: &str,
    clip_id: &str,
    replacement: Option<&str>,
    disable: bool,
) -> Result<(), String> {
    validate_clip(conn, track_id, Some(clip_id))?;
    if replacement == Some(clip_id) {
        return Err("Choose a different replacement clip".into());
    }
    validate_clip(conn, track_id, replacement)?;
    let refs = references(conn, track_id, Some(clip_id))?;
    if !refs.is_empty() && replacement.is_none() && !disable {
        return Err(format!("This clip is selected by {} scene(s). Choose a replacement or disable their track before removing it.",refs.len()));
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    conn.execute("UPDATE scene_tracks SET selected_clip_id=?1, enabled=CASE WHEN ?1 IS NULL THEN 0 ELSE enabled END WHERE track_id=?2 AND selected_clip_id=?3",params![replacement,track_id,clip_id]).map_err(|e|e.to_string())?;
    conn.execute(
        "UPDATE track_mix SET selected_clip_id=?1 WHERE track_id=?2 AND selected_clip_id=?3",
        params![replacement, track_id, clip_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM clips WHERE id=?1 AND track_id=?2",
        params![clip_id, track_id],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn delete_track(
    conn: &Connection,
    track_id: &str,
    remove_from_scenes: bool,
) -> Result<(), String> {
    if !remove_from_scenes && !references(conn, track_id, None)?.is_empty() {
        return Err(
            "This shared track is used by saved scenes. Confirm removing those references first."
                .into(),
        );
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scene_tracks WHERE track_id=?1", [track_id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM tracks WHERE id=?1", [track_id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

pub fn duplicate_track(conn: &Connection, track_id: &str, name: &str) -> Result<String, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let id = duplicate_track_inner(conn, track_id, name)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

fn duplicate_track_inner(conn: &Connection, track_id: &str, name: &str) -> Result<String, String> {
    check_exists(conn, "tracks", track_id)?;
    let id = Uuid::new_v4().to_string();
    conn.execute("INSERT INTO tracks(id,name,volume,mode_type,repeat,transition_type,transition_gap_min,transition_gap_max,fade_in,fade_out,sort_order) SELECT ?1,?2,volume,mode_type,repeat,transition_type,transition_gap_min,transition_gap_max,fade_in,fade_out,sort_order FROM tracks WHERE id=?3",params![id,name,track_id]).map_err(|e|e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id FROM clips WHERE track_id=?1 ORDER BY sort_order")
        .map_err(|e| e.to_string())?;
    let clips = stmt
        .query_map([track_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<SqlResult<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for clip in clips {
        conn.execute("INSERT INTO clips(id,track_id,library_entry_id,volume,sort_order) SELECT ?1,?2,library_entry_id,volume,sort_order FROM clips WHERE id=?3",params![Uuid::new_v4().to_string(),id,clip]).map_err(|e|e.to_string())?;
    }
    Ok(id)
}

pub fn make_scene_track_independent(
    conn: &Connection,
    scene_id: &str,
    track_id: &str,
) -> Result<String, String> {
    let source = load_track_by_id(conn, track_id)?;
    let states = scene_tracks(conn, scene_id)?;
    let state = states
        .iter()
        .find(|state| state.track_id == track_id)
        .ok_or("Track is not in this scene")?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let id = duplicate_track_inner(conn, track_id, &format!("{} (copy)", source.name))?;
    let copied = load_track_by_id(conn, &id)?;
    let selected = state
        .selected_clip_id
        .as_ref()
        .and_then(|selected| source.clips.iter().position(|clip| &clip.id == selected))
        .map(|index| copied.clips[index].id.clone());
    let board: String = conn
        .query_row("SELECT board_id FROM scenes WHERE id=?1", [scene_id], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    attach_track(conn, &board, &id)?;
    conn.execute(
        "UPDATE scene_tracks SET track_id=?1,selected_clip_id=?2 WHERE scene_id=?3 AND track_id=?4",
        params![id, selected, scene_id, track_id],
    )
    .map_err(|e| e.to_string())?;
    set_mix(conn, &id, Some(state.volume), Some(selected.as_deref()))?;
    // The live engine still has the old identity until the revised scene loads.
    conn.execute("DELETE FROM meta WHERE key='active_scene_id' AND value=?1",[scene_id]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}
