# Media Library & Tagging System

## Problem

LuteDrop stores audio file paths directly on Clips. When the same audio file is used in multiple playlists, the user must maintain duplicate copies on disk, each producing its own Clip, peak cache entry, and ID. There is no way to tag or filter audio files by mood, setting, or other metadata when building playlists.

## Solution

Introduce a **media library** layer between raw audio files on disk and the playlist hierarchy. The library is the single source of truth for all imported audio. Clips become thin references from Tracks to library entries. Audio files are never copied — the library stores paths to their original locations on disk.

Additionally, **migrate all persistence from JSON to SQLite**. The playlist hierarchy (Groups, Comps, Tracks, Clips) and the new library data all live in one embedded database.

## SQLite Schema

### Library tables

```sql
CREATE TABLE library_entries (
    id            TEXT PRIMARY KEY,
    file_path     TEXT NOT NULL UNIQUE,
    content_hash  TEXT,
    name          TEXT NOT NULL,
    duration      REAL,
    sample_rate   INTEGER,
    file_size     INTEGER,
    volume        REAL NOT NULL DEFAULT 1.0,
    trim_start    REAL,
    trim_end      REAL,
    loop_start    REAL,
    loop_end      REAL,
    created_at    TEXT NOT NULL
);

CREATE TABLE tags (
    id    TEXT PRIMARY KEY,
    name  TEXT NOT NULL UNIQUE
);

CREATE TABLE library_entry_tags (
    library_entry_id  TEXT REFERENCES library_entries(id) ON DELETE CASCADE,
    tag_id            TEXT REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (library_entry_id, tag_id)
);
```

### Playlist hierarchy tables

```sql
CREATE TABLE groups (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    sort_order  INTEGER NOT NULL
);

CREATE TABLE comps (
    id          TEXT PRIMARY KEY,
    group_id    TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    sort_order  INTEGER NOT NULL
);

CREATE TABLE tracks (
    id                  TEXT PRIMARY KEY,
    comp_id             TEXT NOT NULL REFERENCES comps(id) ON DELETE CASCADE,
    name                TEXT NOT NULL,
    volume              REAL NOT NULL DEFAULT 1.0,
    mode_type           TEXT NOT NULL,
    repeat              INTEGER,
    transition_type     TEXT,
    transition_gap_min  REAL,
    transition_gap_max  REAL,
    fade_in             REAL NOT NULL DEFAULT 0.0,
    fade_out            REAL NOT NULL DEFAULT 0.0,
    sort_order          INTEGER NOT NULL
);

CREATE TABLE clips (
    id                TEXT PRIMARY KEY,
    track_id          TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    library_entry_id  TEXT NOT NULL REFERENCES library_entries(id) ON DELETE RESTRICT,
    volume            REAL NOT NULL DEFAULT 1.0,
    sort_order        INTEGER NOT NULL
);
```

### Metadata table

```sql
CREATE TABLE meta (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

-- Initial row:
-- INSERT INTO meta (key, value) VALUES ('schema_version', '1');
```

### Key schema decisions

- **`ON DELETE RESTRICT` on clips -> library_entries.** Deleting a library entry that is still referenced by clips is blocked. The user must remove it from all tracks first, or the UI prompts them.
- **`ON DELETE CASCADE` on hierarchy tables.** Deleting a group cascades to its comps, tracks, and clips. Same for comps and tracks.
- **`sort_order` uses gapped integers.** Items are spaced by 1000 (first = 1000, second = 2000, etc.). Reordering inserts at the midpoint between neighbors. When a gap is exhausted, the entire list is renumbered.
- **TrackMode is flattened into columns** (`mode_type`, `repeat`, `transition_type`, `transition_gap_min`, `transition_gap_max`). `loop_start`/`loop_end` no longer live on the track — Seamless mode uses the library entry's loop points. Remaining columns are nullable where they do not apply to the current mode type. No JSON blobs.
- **Clips have no name column.** The display name is inherited from the referenced library entry. This avoids stale names if the user renames an entry in the library.
- **Tags are freeform.** The `tags` table stores normalized lowercase strings. No predefined categories.

### Volume chain

```
final_volume = library_entry.volume * clip.volume * track.volume * master_volume
```

- **Library entry volume (gain):** Normalizes the source file. Set once on import or audition.
- **Clip volume (mix level):** Per-usage contextual adjustment. Defaults to 1.0.
- **Track volume (fader):** Balances this track against other tracks in the comp.
- **Master volume:** Global output level.

### Trim and loop points

Trim and loop points live on the library entry. They are a property of the audio file, not of any particular usage. Set once in the Library view, inherited by every clip that references the entry.

If a per-clip override is needed in the future, nullable override columns can be added to the `clips` table.

## Rust dependencies

**`rusqlite`** for SQLite access. Direct bindings, raw SQL, manual struct mapping. Synchronous (SQLite is in-process, queries take microseconds). Minimal complexity, well-documented, smallest dependency footprint.

## Import flow

### New file import

1. User opens file picker from the Library view (or inline when adding clips to a Track). Selects one or more audio files.
2. For each file:
   - Compute SHA-256 hash of file contents.
   - Check `library_entries` for a matching `content_hash`.
   - **Duplicate found:** Skip, show toast: "Already in library as '{name}'".
   - **New file:** Insert `library_entry` row with file path, hash, name (from filename), duration, sample rate, file size. Volume defaults to 1.0. No trim/loop points.
   - Open tagging prompt. User can add freeform tags before confirming.
3. Peak cache generation happens on first waveform render, unchanged.

### Bulk import

Selecting a folder imports every audio file recursively. Same dedup logic per file. After import, a bulk tagging step where the user applies tags to everything just imported.

### Adding clips to a Track

1. User clicks "add clip" on a Track.
2. A library browser opens — search by name, filter by tags.
3. User selects one or more library entries and confirms.
4. Each becomes a Clip row: new ID, `track_id`, `library_entry_id`, volume 1.0, `sort_order` appended to end.

The same library entry can appear as clips in any number of tracks.

## Migration from JSON

On first launch after the update, if `lutedrop-data.json` exists and the SQLite database has no data:

1. **Parse old JSON** into existing `AppState` struct.
2. **Deduplicate audio files.** Walk every Clip, hash each file. Clips at different paths with the same hash collapse into one library entry.
3. **Create library entries.** One per unique audio file. Name from filename, volume 1.0, no tags or trim/loop points.
4. **Recreate hierarchy.** Insert groups, comps, tracks, clips into their tables. Preserve ordering via `sort_order` (first = 1000, second = 2000, etc.). Wire clips to `library_entry_id` instead of raw file paths. Flatten track mode into columns.
5. **Handle missing files.** If a clip's file does not exist on disk, still create the library entry with a NULL `content_hash`. Flag it for the broken reference flow.
6. **Show migration summary.** Toast or dialog: "Migrated X groups, Y tracks, Z library entries. N files could not be found."
7. **Backup old file.** Rename `lutedrop-data.json` to `lutedrop-data.backup.json`.

Future schema changes use the `meta` table's `schema_version` value. Startup code checks the version and runs sequential migrations as needed.

## Broken reference detection

### On startup

After loading the database, check `Path::exists()` for every `library_entry.file_path`. Not a hash check — just existence. Fast even for hundreds of entries.

### When a file is missing

- Flag in memory (not a permanent DB column — the file may come back).
- UI shows a warning indicator on affected library entries and any clips referencing them.
- Playback skips missing clips. If a Track has 3 clips and one is missing, it plays the other 2.
- Waveforms show an empty/error state.

### Relocating a missing file

- Right-click a missing entry -> "Relocate file".
- File picker opens. User selects the new location.
- `file_path` updated in the database.
- For bulk relocation: "Search this folder for all missing files" — match by filename + content hash.

## Library UI

### Library view (new top-level view)

- List of all library entries.
- Search bar filters by name.
- Tag filter: click tags to narrow results, or type them.
- Each entry shows: name, duration, tags, waveform thumbnail, missing-file indicator if applicable.
- Actions per entry: edit name, edit tags, set trim/loop points (opens waveform editor), set gain volume, delete from library (blocked if clips reference it — prompt shows which tracks use it), relocate file.
- Bulk actions: select multiple entries, apply tags, delete.

### Inline library browser (adding clips to a Track)

- Same search and tag filtering, in a dialog or panel.
- Select one or more entries, confirm. They become clips in the Track.

## Frontend data flow

### Startup

Backend loads the full hierarchy and all library entries/tags from SQLite. Sends one payload to the frontend. Frontend hydrates the Pinia store.

### Mutations

Same IPC command pattern. Frontend calls `invoke('add_track', { compId, name })`. Backend mutates SQLite, returns updated state. Frontend rehydrates.

Library mutations (add entry, edit tags, delete entry) follow the same pattern.

### Pinia store changes

- `AppState` grows `libraryEntries` and `tags` arrays alongside the existing `groups` array.
- Clips lose `filePath`, gain `libraryEntryId`.
- Composables that need file paths resolve through library entries.
- `useTrackImport` replaced with `useLibraryBrowser` composable.

### Rust model changes

- Existing structs (`AppState`, `Group`, `Comp`, `Track`, `Clip`) are built from SQL queries instead of JSON deserialization.
- New `LibraryEntry` and `Tag` structs.
- `ts-rs` generates TypeScript types for all models.

### No changes to

- Playback polling loop (50ms).
- Audio engine (Kira). File paths resolved through library entries instead of directly from clips.
- WebSocket server / remote control protocol.
- Peak cache system (keyed by file path, unchanged).
