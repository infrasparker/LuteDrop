"""Repair an explicitly selected board using a pre-v18 backup as layout evidence.

Dry-run by default. --apply takes a consistent SQLite backup before one atomic
repair. Never run with LuteDrop open. No Library files or Library rows change.
"""
import argparse
import datetime
import json
from pathlib import Path
import sqlite3


def repair(conn, legacy, board_id, duplicate=None, canonical=None):
    conn.row_factory = sqlite3.Row
    legacy.row_factory = sqlite3.Row
    report = {"merged_tracks": 0, "removed_layout_references": 0, "restored_layout_orders": 0}
    aliases = {}
    if duplicate:
        assert canonical and duplicate != canonical
        fields = "name,volume,mode_type,repeat,transition_type,transition_gap_min,transition_gap_max,fade_in,fade_out"
        source = conn.execute(f"SELECT {fields} FROM tracks WHERE id=?", (duplicate,)).fetchone()
        target = conn.execute(f"SELECT {fields} FROM tracks WHERE id=?", (canonical,)).fetchone()
        if not source or not target or tuple(source) != tuple(target):
            raise ValueError("Tracks differ; refusing to merge their playback settings")
        clips = lambda tid: conn.execute("SELECT id,library_entry_id,volume FROM clips WHERE track_id=? ORDER BY sort_order", (tid,)).fetchall()
        old, new = clips(duplicate), clips(canonical)
        if [tuple(row)[1:] for row in old] != [tuple(row)[1:] for row in new]:
            raise ValueError("Tracks differ; refusing to merge their clips")
        clip_map = {a["id"]: b["id"] for a, b in zip(old, new)}
        for ref in conn.execute("SELECT * FROM scene_tracks WHERE track_id=?", (duplicate,)).fetchall():
            selected = clip_map.get(ref["selected_clip_id"])
            existing = conn.execute("SELECT * FROM scene_tracks WHERE scene_id=? AND track_id=?", (ref["scene_id"], canonical)).fetchone()
            if existing:
                if ref["enabled"] and existing["enabled"] and (ref["volume"], selected) != (existing["volume"], existing["selected_clip_id"]):
                    raise ValueError("Both duplicate layers are enabled with different mix settings")
                if ref["enabled"] and not existing["enabled"]:
                    conn.execute("UPDATE scene_tracks SET enabled=1,volume=?,selected_clip_id=? WHERE scene_id=? AND track_id=?", (ref["volume"], selected, ref["scene_id"], canonical))
                conn.execute("DELETE FROM scene_tracks WHERE scene_id=? AND track_id=?", (ref["scene_id"], duplicate))
            else:
                conn.execute("UPDATE scene_tracks SET track_id=?,selected_clip_id=? WHERE scene_id=? AND track_id=?", (canonical, selected, ref["scene_id"], duplicate))
        for member in conn.execute("SELECT * FROM board_tracks WHERE track_id=?", (duplicate,)).fetchall():
            conn.execute("INSERT OR IGNORE INTO board_tracks VALUES(?,?,?)", (member["board_id"], canonical, member["sort_order"]))
        conn.execute("DELETE FROM tracks WHERE id=?", (duplicate,))
        aliases[duplicate] = canonical
        report["merged_tracks"] = 1
    all_ids = {r[0] for r in conn.execute("SELECT track_id FROM board_tracks WHERE board_id=?", (board_id,))}
    for scene in conn.execute("SELECT id,name FROM scenes WHERE board_id=?", (board_id,)).fetchall():
        original_order = list(dict.fromkeys(aliases.get(r[0], r[0]) for r in legacy.execute("SELECT id FROM tracks WHERE comp_id=? ORDER BY sort_order", (scene["id"],))))
        original = set(original_order)
        current = conn.execute("SELECT track_id,enabled FROM scene_tracks WHERE scene_id=?", (scene["id"],)).fetchall()
        # The first build's snapshot bug added the complete board, switched off.
        # Repair only that recognizable pattern in scenes evidenced by the backup.
        if original and {r["track_id"] for r in current} == all_ids:
            for ref in current:
                if not ref["enabled"] and ref["track_id"] not in original:
                    conn.execute("DELETE FROM scene_tracks WHERE scene_id=? AND track_id=?", (scene["id"], ref["track_id"]))
                    report["removed_layout_references"] += 1
            retained = [r[0] for r in conn.execute("SELECT track_id FROM scene_tracks WHERE scene_id=? ORDER BY sort_order", (scene["id"],))]
            ordered = [tid for tid in original_order if tid in retained] + [tid for tid in retained if tid not in original]
            for index, tid in enumerate(ordered):
                conn.execute("UPDATE scene_tracks SET sort_order=? WHERE scene_id=? AND track_id=?", ((index+1)*1000, scene["id"], tid))
            report["restored_layout_orders"] += 1
    if conn.execute("PRAGMA foreign_key_check").fetchall():
        raise ValueError("Repair introduced a foreign key error")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--legacy-backup", type=Path, required=True)
    parser.add_argument("--board-id", required=True)
    parser.add_argument("--duplicate")
    parser.add_argument("--canonical")
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    with sqlite3.connect(args.database.resolve().as_uri() + "?mode=ro", uri=True) as source:
        if args.apply:
            backup = args.database.parent / "backups" / ("before-layout-repair-" + datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f") + ".db")
            backup.parent.mkdir(parents=True, exist_ok=True)
            with sqlite3.connect(backup) as copy:
                source.backup(copy)
            print("Backup:", backup)
            conn = sqlite3.connect(args.database)
        else:
            conn = sqlite3.connect(":memory:")
            source.backup(conn)
    conn.execute("PRAGMA foreign_keys=ON")
    with conn, sqlite3.connect(args.legacy_backup.resolve().as_uri() + "?mode=ro", uri=True) as legacy:
        print(json.dumps(repair(conn, legacy, args.board_id, args.duplicate, args.canonical)))
    conn.close()


if __name__ == "__main__":
    main()
