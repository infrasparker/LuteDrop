"""Regenerates the UI preview fixtures (ui-preview/fixtures/*.json) from the
live LuteDrop library, read-only.

    python ui-preview/export_fixtures.py            # state, settings, import report, Tune runs
    python ui-preview/export_fixtures.py --no-tune  # skip the analyzer calls

Why a copy: the live database (%APPDATA%\\com.lutedrop\\lutedrop.db) may be open
in the app. It is opened read-only (`mode=ro`) and copied with SQLite's backup
API into a temporary file, which gives a consistent snapshot even mid-write;
everything below reads that copy. Nothing is ever written to the live file.

What it writes:
- state.json: the AppState `load_data` returns (a port of
  `database::load_full_state`), with file paths cut down to
  "<artist>/<album>/<file>" under a fake L: drive.
- settings.json: the analyzer and broadcast settings with the URL, token,
  prefixes and listener key replaced.
- case-import.json: the CaseImportReport `import_case_studies` would give for
  the case-study file, if it exists locally (a port of `plan_import`).
- tune.json: real `/tune` runs on the deployed analyzer, turned into the app's
  TuneResult shape the way src-tauri/src/autotag/tune.rs does, keyed
  "<mood id>:<mode>". The request is built as `build_tune_request` builds it.
  The analyzer URL and token come from the copied meta table and are never
  printed or saved.
"""

import argparse
import json
import math
import os
import re
import sqlite3
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
OUT = HERE / "fixtures"
LIVE_DB = Path(os.path.expandvars(r"%APPDATA%\com.lutedrop\lutedrop.db"))
CASE_FILE = REPO / "docs/superpowers/research/2026-11-05-feel-lab/calibration-case-studies.json"
ENERGY_ID = "energy"

# The Tune runs captured: (mood id, mode, typed input).
TUNE_RUNS = [
    ("sorrowful", "compare", {"versions_extra": [[], ["melancholic"]]}),
    ("sorrowful", "arrange", {"typed": ["grief, funeral, dark sadness, cello, sad piano, memory, lonely, farewell"]}),
    ("sorrowful", "suggest", {}),
    ("grim", "suggest", {}),
]


# --- Snapshot ---

def snapshot(dest: Path) -> sqlite3.Connection:
    src = sqlite3.connect(f"file:{LIVE_DB.as_posix()}?mode=ro", uri=True)
    dst = sqlite3.connect(dest)
    src.backup(dst)
    src.close()
    return dst


def meta(conn) -> dict:
    return dict(conn.execute("SELECT key, value FROM meta"))


# --- Paths ---

def cut_path(path: str) -> str:
    """Keeps the last three parts (artist / album / file, which the UI shows)
    under a fake drive, dropping the rest of the local path."""
    parts = [p for p in re.split(r"[\\/]", path) if p and not re.fullmatch(r"[A-Za-z]:", p)]
    return "L:/" + "/".join(parts[-3:])


def album_of(path: str) -> str:
    parts = [p for p in re.split(r"[\\/]", path) if p and not (len(p) == 2 and p.endswith(":"))]
    n = len(parts)
    return " / ".join(parts[i] for i in (n - 3, n - 2) if i >= 0)


# --- Vocabulary, calibration, flags (database/settings.rs, mood_flags.rs) ---

def vocabulary(m: dict) -> dict:
    return json.loads(m["mood_vocabulary"])


def calibration(m: dict):
    text = m.get("mood_calibration")
    return json.loads(text) if text else None


def vocab_ids(v) -> list:
    return [x["id"] for x in v["moods"]] + [ENERGY_ID]


def prompt_signature(v, mood_id):
    if mood_id == ENERGY_ID:
        return [f"+ {p}" for p in v["energy"]["positive"]] + [f"- {p}" for p in v["energy"]["negative"]]
    return next((x["prompts"] for x in v["moods"] if x["id"] == mood_id), None)


def lines_sig(prompts) -> str:
    return "\n".join(prompts)


def read_mark(mark):
    if not mark or "|" not in mark:
        return None
    a, b = mark.split("|", 1)
    try:
        return float(a), float(b)
    except ValueError:
        return None


def stale_reason(flag, current_sig, current_stat):
    if flag["kind"] in ("higher", "lower") and current_stat is not None:
        then = read_mark(flag["calibrated_at"])
        same = then is not None and abs(then[0] - current_stat["mean"]) <= 1e-9 and abs(then[1] - current_stat["sd"]) <= 1e-9
        if not same:
            return "calibration"
    return "wording" if flag["lines_sig"] != current_sig else None


FLAG_COLS = "library_entry_id, mood, kind, band, bound, bar_at_flag, calibrated_at, lines_sig"


def flag_row(r):
    keys = ["entry_id", "mood", "kind", "band", "bound", "bar_at_flag", "calibrated_at", "lines_sig"]
    return dict(zip(keys, r))


def flags_for_mood(conn, mood):
    return [flag_row(r) for r in conn.execute(
        f"SELECT {FLAG_COLS} FROM mood_flags WHERE mood = ? ORDER BY created_at, library_entry_id", (mood,))]


def flags_newest_first(conn, mood):
    return [r[0] for r in conn.execute(
        "SELECT library_entry_id FROM mood_flags WHERE mood = ? ORDER BY created_at DESC, rowid DESC", (mood,))]


def needs_recalibration(cal, v) -> bool:
    if cal is None:
        return False
    ok = [i for i in vocab_ids(v) if i in cal["stats"] and cal["prompts"].get(i) == prompt_signature(v, i)]
    return len(ok) < len(v["moods"]) + 1


# --- AppState (database::load_full_state) ---

ROLE = {"combat": "combat", "non_combat": "nonCombat"}
ANALYSIS = {"embedded": "embedded", "failed": "failed", "unreachable_path": "unreachablePath"}


def library_entries(conn, v, cal):
    tags = {}
    for eid, name in conn.execute(
        "SELECT let.library_entry_id, t.name FROM library_entry_tags let "
        "JOIN tags t ON t.id = let.tag_id ORDER BY t.name ASC"):
        tags.setdefault(eid, []).append(name)

    analysis = {
        eid: {"status": ANALYSIS.get(status, "pending"), "scoredAt": at, "error": err}
        for eid, status, at, err in conn.execute(
            "SELECT library_entry_id, status, analyzed_at, error FROM entry_analysis")
    }

    ids = vocab_ids(v)
    moods = {}
    for eid, mood, auto, user in conn.execute(
        "SELECT library_entry_id, mood, auto_score, user_level FROM library_entry_moods"):
        if mood in ids:
            moods.setdefault(eid, {})[mood] = (auto, user)

    stats = cal["stats"] if cal else {}
    flags = {}
    for r in conn.execute(f"SELECT {FLAG_COLS} FROM mood_flags"):
        f = flag_row(r)
        current = prompt_signature(v, f["mood"])
        if current is None:
            continue
        reason = stale_reason(f, lines_sig(current), stats.get(f["mood"]))
        flags[(f["entry_id"], f["mood"])] = {
            "kind": f["kind"], "band": f["band"], "bound": f["bound"], "barAtFlag": f["bar_at_flag"],
            "calibratedAt": f["calibrated_at"], "stale": reason is not None, "staleReason": reason,
        }

    out, real_paths = [], {}
    for row in conn.execute(
        "SELECT id, file_path, content_hash, name, duration, sample_rate, file_size, volume, trim_start, trim_end, "
        "loop_start, loop_end, created_at, entry_type, role_user, mood_reviewed_at "
        "FROM library_entries ORDER BY name ASC"):
        (eid, path, chash, name, dur, sr, size, vol, ts, te, ls, le, created, etype, role_user, reviewed) = row
        etype = "ambiance" if etype == "ambiance" else "music"
        role_user = ROLE.get(role_user)
        stored = moods.get(eid, {})
        real_paths[eid] = path
        out.append({
            "id": eid, "filePath": cut_path(path), "contentHash": chash, "name": name, "duration": dur,
            "sampleRate": sr, "fileSize": size, "volume": vol, "trimStart": ts, "trimEnd": te,
            "loopStart": ls, "loopEnd": le, "createdAt": created, "tags": tags.get(eid, []), "entryType": etype,
            "analysis": analysis.get(eid),
            "role": (role_user or "nonCombat") if etype == "music" else None, "roleUser": role_user,
            "moods": [
                {"mood": m, "auto": stored.get(m, (None, None))[0], "user": stored.get(m, (None, None))[1],
                 "flag": flags.get((eid, m))}
                for m in ids
            ],
            "moodReviewedAt": reviewed,
        })
    return out, real_paths


def transition(ttype, gmin, gmax):
    if ttype == "gap":
        return {"type": "gap", "min": gmin or 0.0, "max": gmax or 0.0}
    if ttype == "crossfade":
        return {"type": "crossfade"}
    return {"type": "gapless"}


def soundstage(conn, config_id, entries):
    """Export v18 shared definitions and scene presets by identity."""
    by_id = {e["id"]: e for e in entries}
    tracks = []
    for tid, name, volume, mode, repeat, ttype, gmin, gmax, fin, fout in conn.execute(
            "SELECT id,name,volume,mode_type,repeat,transition_type,transition_gap_min,"
            "transition_gap_max,fade_in,fade_out FROM tracks ORDER BY sort_order,id"):
        clips = []
        for cid, eid, cvol in conn.execute(
                "SELECT id,library_entry_id,volume FROM clips WHERE track_id=? ORDER BY sort_order,id", (tid,)):
            entry = by_id.get(eid)
            clips.append({
                "id": cid, "name": entry["name"] if entry else "[missing]",
                "filePath": entry["filePath"] if entry else "", "volume": cvol,
                "libraryEntryId": eid, "libraryVolume": entry["volume"] if entry else 1.0,
                "loudnessFactor": 1.0,
                "trimStart": entry["trimStart"] if entry else None,
                "trimEnd": entry["trimEnd"] if entry else None,
            })
        if mode == "seamless":
            first = by_id.get(clips[0]["libraryEntryId"]) if clips else None
            tmode = {"type": "seamless", "loopStart": (first["loopStart"] or 0) if first else 0,
                     "loopEnd": (first["loopEnd"] or 0) if first else 0}
        elif mode == "oneShot":
            tmode = {"type": "oneShot"}
        else:
            tmode = {"type": mode, "repeat": bool(repeat), "transition": transition(ttype, gmin, gmax)}
        tracks.append({"id": tid, "name": name, "clips": clips, "volume": volume,
                       "mode": tmode, "fadeIn": fin, "fadeOut": fout})
    boards = []
    for bid, name in conn.execute(
            "SELECT id,name FROM boards WHERE configuration_id=? ORDER BY sort_order,id", (config_id,)):
        scenes = []
        for sid, scene_name in conn.execute(
                "SELECT id,name FROM scenes WHERE board_id=? ORDER BY sort_order,id", (bid,)):
            refs = [{"trackId": tid, "enabled": bool(enabled), "volume": vol, "selectedClipId": clip}
                    for tid, enabled, vol, clip in conn.execute(
                        "SELECT track_id,enabled,volume,selected_clip_id FROM scene_tracks "
                        "WHERE scene_id=? ORDER BY sort_order,track_id", (sid,))]
            scenes.append({"id": sid, "name": scene_name, "tracks": refs})
        ids = [r[0] for r in conn.execute(
            "SELECT track_id FROM board_tracks WHERE board_id=? ORDER BY sort_order,track_id", (bid,))]
        boards.append({"id": bid, "name": name, "scenes": scenes, "trackIds": ids})
    mix = [{"trackId": tid, "volume": vol, "selectedClipId": clip}
           for tid, vol, clip in conn.execute("SELECT track_id,volume,selected_clip_id FROM track_mix")]
    return {"boards": boards, "tracks": tracks, "mix": mix}


def app_state(conn, m):
    v = vocabulary(m)
    cal = calibration(m)
    entries, real_paths = library_entries(conn, v, cal)
    config_id = m.get("active_configuration_id", "")
    tag_names = [r[0] for r in conn.execute(
        "SELECT DISTINCT t.name FROM tags t JOIN library_entry_tags lt ON lt.tag_id = t.id ORDER BY t.name ASC")]
    configs = [{"id": i, "name": n, "createdAt": c} for i, n, c in conn.execute(
        "SELECT id, name, created_at FROM configurations ORDER BY sort_order ASC, created_at ASC")]
    # Missing files: checked against the real paths now, like the app's startup scan.
    missing = [e["id"] for e in entries if not os.path.exists(real_paths[e["id"]])]
    return {
        **soundstage(conn, config_id, entries),
        "activeSceneId": m.get("active_scene_id"),
        "libraryEntries": entries,
        "tags": tag_names,
        "configurations": configs,
        "activeConfigurationId": config_id,
        "missingEntryIds": missing,
        "moodVocabulary": v,
        "moodCalibration": None if cal is None else {
            "calibratedAt": cal["calibratedAt"], "tracks": cal["tracks"], "source": cal.get("source", "library"),
            "referenceSet": cal.get("referenceSet"),
        },
        "moodsNeedRecalibration": needs_recalibration(cal, v),
        "moodVocabularyUnreadable": False,
    }


def settings(m):
    """Shapes the settings dialogs ask for, with every secret replaced."""
    return {
        "autotag": {"enabled": m.get("autotag_enabled") == "true", "analyzerUrl": "http://analyzer.example:8765",
                    "token": "", "localPrefix": "L:/", "remotePrefix": "/music/"},
        "broadcast": {"enabled": m.get("broadcast_enabled") == "true", "port": int(m.get("broadcast_port") or 5883),
                      "key": "preview-key", "publicBase": "", "lanUrl": "http://192.168.0.10:5883/listen/preview-key",
                      "publicUrl": None, "listenerCount": 0, "error": None},
    }


# --- Case-study import (autotag/case_import.rs plan_import) ---

def normalize(name):
    return " ".join(name.replace("\xa0", " ").split()).lower()


def find_entry(track, names):
    want = normalize(track)
    if not want:
        return None
    norm = [normalize(n) for _, n in names]
    if want in norm:
        return names[norm.index(want)][0]
    hits = [i for i, n in enumerate(norm) if want in n]
    return names[hits[0]][0] if len(hits) == 1 else None


def case_import_report(conn, v):
    if not CASE_FILE.exists():
        return None
    doc = json.loads(CASE_FILE.read_text(encoding="utf-8"))
    names = conn.execute("SELECT id, name FROM library_entries WHERE entry_type = 'music' ORDER BY name, id").fetchall()
    ids = [x["id"] for x in v["moods"]]
    imported, unmatched, skipped, settled_skipped = 0, [], set(), 0
    for case in doc.get("cases", []):
        track = case.get("track")
        if not isinstance(track, str):
            continue
        lists = {k: [x for x in case.get(k, []) if isinstance(x, str)] for k in ("high", "maybe", "stretch", "low", "settled")}
        moods = sorted(set(lists["high"] + lists["maybe"] + lists["stretch"] + lists["low"]))
        before = len(moods)
        moods = [x for x in moods if x not in lists["settled"]]
        settled_skipped += before - len(moods)
        if not moods:
            continue
        if find_entry(track, names) is None:
            unmatched.append(track)
            continue
        for mood in moods:
            if mood not in ids:
                skipped.add(mood)
            else:
                imported += 1
    return {"imported": imported, "unmatched": unmatched, "skippedMoods": sorted(skipped), "settledSkipped": settled_skipped}


# --- Tune (autotag/tune.rs) ---

BAND_FLOOR = {"definitely": 0.9, "understandable": 0.75, "stretch": 0.5, "no": None}
BAND_CEIL = {"definitely": None, "understandable": 0.9, "stretch": 0.75, "no": 0.5}
APP_BOUND = {"definitely": "at_least", "understandable": "at_least", "stretch": "at_most", "no": "in"}
DIRECTION_MARGIN, STALE_WEIGHT, MAX_BANK = 0.1, 0.5, 1000
REFERENCE_MIDPOINT_Z, MOOD_MIDPOINT_Z, MOOD_SLOPE = 0.674, 0.5, 1.5


def flag_target(f, bar):
    if f["kind"] == "higher":
        return None if bar is None else (min(bar + DIRECTION_MARGIN, 1.0), None, "high")
    if f["kind"] == "lower":
        return None if bar is None else (None, max(bar - DIRECTION_MARGIN, 0.0), "low")
    band = f["band"]
    if band is None:
        return None
    if f["bound"] == "at_least":
        return (BAND_FLOOR[band], None, "high")
    if f["bound"] == "at_most":
        return (None, BAND_CEIL[band], "low")
    side = {"no": "low", "definitely": "high"}.get(band, "mid")
    return (BAND_FLOOR[band], BAND_CEIL[band], side)


def level_band(level):
    return "definitely" if level >= 1.0 else "understandable" if level >= 0.75 else "stretch" if level >= 0.5 else "no"


def user_levels(conn, mood):
    return conn.execute(
        "SELECT library_entry_id, user_level FROM library_entry_moods WHERE mood = ? AND user_level IS NOT NULL "
        "ORDER BY library_entry_id", (mood,)).fetchall()


def music_hashes(conn):
    seen, out = set(), []
    for _id, h in conn.execute(
        "SELECT id, content_hash FROM library_entries WHERE entry_type = 'music' AND content_hash IS NOT NULL "
        "ORDER BY name, id"):
        if h not in seen:
            seen.add(h)
            out.append(h)
    return out


def dedupe(words):
    seen, out = set(), []
    for w in (w.strip() for w in words):
        if w and w.lower() not in seen:
            seen.add(w.lower())
            out.append(w)
    return out


def build_request(conn, m, mood_id, mode, extra_words, bank, candidates, focus=None, exclude=()):
    v, cal = vocabulary(m), calibration(m)
    mood = next(x for x in v["moods"] if x["id"] == mood_id)
    sig = lines_sig(mood["prompts"])
    stat = cal["stats"].get(mood_id)
    if mode == "compare":
        extra_words, bank = [], []
        candidates = [[l.strip() for l in c if l.strip()] for c in candidates]
        candidates = [c for c in candidates if c][:8]
    else:
        candidates = []
        if mode == "arrange":
            bank = []

    flags = flags_for_mood(conn, mood_id)
    flagged = {f["entry_id"] for f in flags}
    for eid, level in user_levels(conn, mood_id):
        if eid not in flagged:
            band = level_band(level)
            flags.append({"entry_id": eid, "mood": mood_id, "kind": "band", "band": band, "bound": APP_BOUND[band],
                          "bar_at_flag": None, "calibrated_at": None, "lines_sig": sig})

    out, out_ids, skipped = [], [], 0
    for f in flags:
        row = conn.execute(
            "SELECT e.content_hash, m.auto_score FROM library_entries e LEFT JOIN library_entry_moods m "
            "ON m.library_entry_id = e.id AND m.mood = ? WHERE e.id = ?", (mood_id, f["entry_id"])).fetchone()
        h, bar_now = row if row else (None, None)
        if h is None:
            skipped += 1
            continue
        reason = stale_reason(f, sig, stat)
        bar = (bar_now if bar_now is not None else f["bar_at_flag"]) if reason == "calibration" else f["bar_at_flag"]
        t = flag_target(f, bar)
        if t is None or (t[0] is None and t[1] is None):
            skipped += 1
            continue
        out_ids.append(f["entry_id"])
        out.append({"hash": h, "min": t[0], "max": t[1], "weight": STALE_WEIGHT if reason else 1.0,
                    "fresh": reason is None, "side": t[2], "focus": False})

    if focus == "":
        at = None
    elif focus is not None:
        at = out_ids.index(focus) if focus in out_ids else None
    else:
        at = next((out_ids.index(i) for i in flags_newest_first(conn, mood_id) if i in out_ids), None)
    focus_used = None
    if at is not None:
        out[at]["focus"] = True
        focus_used = out_ids[at]

    typed_in_bank = extra_words if mode == "suggest" else []
    source = cal.get("source", "library")
    ref = source == "reference"
    request = {
        "mood_id": mood_id, "name": mood["name"], "current_lines": mood["prompts"], "flags": out,
        "targets": music_hashes(conn), "bank": dedupe(list(bank) + list(typed_in_bank))[:MAX_BANK],
        "extra_words": dedupe(extra_words),
        "other_moods": {x["id"]: {"name": x["name"], "lines": x["prompts"]} for x in v["moods"] if x["id"] != mood_id},
        "exclude": dedupe(exclude),
        "calibration": {"source": source, "midpoint_z": REFERENCE_MIDPOINT_Z if ref else MOOD_MIDPOINT_Z,
                        "slope": math.log(3) / (1.2816 - REFERENCE_MIDPOINT_Z) if ref else MOOD_SLOPE},
        "mode": mode, "candidates": candidates,
    }
    return request, skipped, focus_used


def songs_index(conn, mood_id):
    """Content hash -> TuneSong, as tune.rs `Songs::read` resolves it."""
    entries = {eid: (h, {"entryId": eid, "name": name, "album": album_of(path)}) for eid, name, path, h in conn.execute(
        "SELECT id, name, file_path, content_hash FROM library_entries WHERE content_hash IS NOT NULL")}
    first_music, seen = [], set()
    for eid, h in conn.execute(
        "SELECT id, content_hash FROM library_entries WHERE entry_type = 'music' AND content_hash IS NOT NULL "
        "ORDER BY name, id"):
        if h not in seen:
            seen.add(h)
            first_music.append(eid)
    order = first_music + [r[0] for r in user_levels(conn, mood_id)] + [f["entry_id"] for f in flags_for_mood(conn, mood_id)]
    songs = {}
    for eid in order:
        if eid in entries:
            h, song = entries[eid]
            songs[h] = song
    return songs


def review(songs, r):
    def top(bars):
        return [{"song": songs[b["hash"]], "bar": b["bar"]} for b in bars if b["hash"] in songs]

    def moved(ms):
        return [{"song": songs[x["hash"]], "before": x["before"], "after": x["after"]} for x in ms if x["hash"] in songs]

    return {
        "lines": r["lines"], "lineSources": r.get("line_sources", []), "loss": r["loss"],
        "stability": r.get("stability", 0.0),
        "flags": [{"song": songs[f["hash"]], "before": f["before"], "after": f["after"], "holds": f["holds"],
                   "fresh": f["fresh"], "focus": f.get("focus", False)} for f in r["flags"] if f["hash"] in songs],
        "top": top(r["top"]), "topBefore": top(r["top_before"]),
        "risers": moved(r["risers"]), "fallers": moved(r["fallers"]),
        "firesBefore": r["fires_before"], "firesAfter": r["fires_after"],
        "overlap": [{"moodId": o["mood_id"], "before": o["before"], "after": o["after"]} for o in r["overlap"]],
        "loo": r["loo"], "names": r["names"],
        "unsatisfied": [songs[h] for h in r["unsatisfied"] if h in songs],
    }


def post(url, token, path, body, timeout):
    req = urllib.request.Request(url + path, json.dumps(body).encode() if body is not None else None,
                                 {"Content-Type": "application/json", "X-LuteDrop-Token": token})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.load(resp)


def word_bank():
    text = (REPO / "src/utils/moodWords.ts").read_text(encoding="utf-8")
    body = re.search(r"export const WORD_BANK = \[(.*?)\];", text, re.S).group(1)
    return re.findall(r"'([^']+)'", body)


def tune_runs(conn, m):
    url, token = m["autotag_url"].strip().rstrip("/"), m["autotag_token"]
    for _ in range(60):
        try:
            h = post(url, token, "/health", None, 10)
            if h.get("ready", True) and not h.get("loading"):
                break
        except Exception as e:  # noqa: BLE001 - waiting for the analyzer to load
            print("  analyzer not ready:", type(e).__name__)
        time.sleep(5)
    bank = word_bank()
    v = vocabulary(m)
    out = {}
    for mood_id, mode, inputs in TUNE_RUNS:
        current = next(x["prompts"] for x in v["moods"] if x["id"] == mood_id)
        candidates = [current + extra for extra in inputs.get("versions_extra", [])]
        extra = inputs.get("typed", [])
        request, skipped_flags, focus = build_request(conn, m, mood_id, mode, extra, bank, candidates)
        t = time.time()
        resp = post(url, token, "/tune", request, 600)
        songs = songs_index(conn, mood_id)
        s = resp.get("split")
        out[f"{mood_id}:{mode}"] = {
            "baseline": review(songs, resp["baseline"]),
            "proposals": [review(songs, p) for p in resp["proposals"]],
            "split": None if not s else {
                "keep": review(songs, s["keep"]), "new": review(songs, s["new"]),
                "keepSongs": [songs[h] for h in s["keep_hashes"] if h in songs],
                "newSongs": [songs[h] for h in s["new_hashes"] if h in songs], "rho": s["rho"],
            },
            "skipped": [songs[h] for h in resp["skipped"] if h in songs],
            "linesTunedFrom": request["current_lines"],
            "skippedFlags": skipped_flags,
            "focusEntryId": focus,
            "mode": mode,
        }
        print(f"  {mood_id}:{mode} {time.time() - t:.0f}s, {len(resp['proposals'])} proposals, "
              f"split={'yes' if s else 'no'}")
    return out


def write(name, data):
    OUT.mkdir(exist_ok=True)
    (OUT / name).write_text(json.dumps(data, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    print("wrote", (OUT / name).relative_to(REPO))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--no-tune", action="store_true", help="skip the /tune calls (keeps tune.json)")
    args = ap.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        conn = snapshot(Path(tmp) / "snapshot.db")
        try:
            m = meta(conn)
            write("state.json", app_state(conn, m))
            write("settings.json", settings(m))
            report = case_import_report(conn, vocabulary(m))
            if report is not None:
                write("case-import.json", report)
            if not args.no_tune:
                write("tune.json", tune_runs(conn, m))
        finally:
            conn.close()


if __name__ == "__main__":
    sys.exit(main())
