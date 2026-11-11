-- Manual clip choices are not automatic-playlist starting instructions.
-- Explicit starting clips chosen AFTER switching to Shuffle/Sequence remain
-- valid, including when only repeat, gaps, or transitions are edited.
CREATE TRIGGER IF NOT EXISTS reset_manual_scene_starts_on_playlist_mode
AFTER UPDATE OF mode_type ON tracks
WHEN OLD.mode_type IN ('seamless', 'oneShot')
 AND NEW.mode_type IN ('shuffle', 'sequence')
BEGIN
    UPDATE scene_tracks SET selected_clip_id = NULL WHERE track_id = NEW.id;
    UPDATE track_mix SET selected_clip_id = NULL WHERE track_id = NEW.id;
END;
