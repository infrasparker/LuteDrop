# Configurations (Project Save/Load)

## Problem

LuteDrop currently has a single soundstage — all groups, comps, tracks, and clips live in one flat workspace. There's no way to save a soundstage setup for one session (e.g., "Dungeon Crawl") and switch to another (e.g., "Tavern Night") without manually rebuilding the hierarchy. The library of audio assets should remain global and shared across all configurations.

## Decisions

- **Storage approach**: Add a `configuration_id` foreign key on the `groups` table. The rest of the hierarchy (comps, tracks, clips) is already scoped through existing FKs. Library tables are untouched.
- **Autosave + Save As**: Current autosave behavior is preserved — every mutation writes to SQLite immediately under the active configuration. "Save As" deep-copies the entire hierarchy with new UUIDs into a new configuration.
- **Full replace on load**: Switching configurations stops all playback, changes the active config pointer, and reloads state. No merging.
- **Migration**: Existing soundstage data gets assigned to an auto-created "Default" configuration. Users see no change until they create a second config.
- **Legacy cleanup**: Remove the JSON-to-SQLite migration module (`database/migration.rs`) and its startup callsite in `lib.rs`.

## Schema Changes

### New table: `configurations`

```sql
CREATE TABLE IF NOT EXISTS configurations (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    sort_order INTEGER NOT NULL
);
```

### Modified table: `groups`

Add `configuration_id` column with a foreign key to `configurations(id)` and cascade delete.

```sql
-- groups gains:
configuration_id TEXT NOT NULL REFERENCES configurations(id) ON DELETE CASCADE
```

### Active config tracking

Stored in the existing `meta` table:

```sql
INSERT INTO meta (key, value) VALUES ('active_configuration_id', '<uuid>');
```

### v1 → v2 migration

Runs when `schema_version = '1'`. Wrapped in a transaction:

1. Create the `configurations` table.
2. Insert a "Default" configuration row (new UUID, current timestamp, sort_order 1000).
3. Recreate `groups` table with the new `configuration_id` FK column (SQLite doesn't support `ALTER TABLE ADD COLUMN ... REFERENCES`, so: create `groups_new`, copy data with Default config ID, drop `groups`, rename `groups_new`).
4. Set `active_configuration_id` in `meta` to the Default config's ID.
5. Update `schema_version` to `'2'`.

### Legacy migration removal

Delete `src-tauri/src/database/migration.rs` and remove:
- `pub mod migration;` from `database/mod.rs`
- The migration callsite in `lib.rs` (~lines 45-54) that calls `needs_migration`, `migrate_from_json`, and `backup_json`

## Rust Model Changes

### New struct: `Configuration`

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub id: String,
    pub name: String,
    pub created_at: String,
}
```

### Modified: `AppState`

```rust
pub struct AppState {
    pub groups: Vec<Group>,                      // scoped to active config
    pub library_entries: Vec<LibraryEntry>,       // unchanged
    pub tags: Vec<String>,                       // unchanged
    pub configurations: Vec<Configuration>,       // all configs (for picker UI)
    pub active_configuration_id: String,          // which config is loaded
}
```

No changes to `Group`, `Comp`, `Track`, `Clip`, `TrackMode`, `TrackTransition`, or `TrackPlaybackState`.

## Backend Changes

### Database layer

**`database/mod.rs`**:
- `load_full_state` reads the active config ID from `meta`, loads all configurations, and filters `load_groups` by the active config's ID.
- `load_groups` query changes from `SELECT id, name FROM groups ORDER BY sort_order` to `SELECT id, name FROM groups WHERE configuration_id = ?1 ORDER BY sort_order`.
- New helper: `get_active_configuration_id(conn)` — reads from `meta` table.
- New helper: `load_configurations(conn)` — returns all configs ordered by `sort_order`.
- `init_schema` adds the `configurations` table creation (with `IF NOT EXISTS`).
- Schema migration function: `migrate_v1_to_v2(conn)` — runs the v1→v2 steps described above.

**New module: `database/configurations.rs`**:
- `insert_configuration(conn, name)` — creates a new empty configuration.
- `delete_configuration(conn, id)` — cascade-deletes all its groups/comps/tracks/clips.
- `rename_configuration(conn, id, name)` — updates the name.
- `set_active_configuration(conn, id)` — updates the `meta` row.
- `duplicate_configuration(conn, source_id, new_name)` — deep-copies all hierarchy rows (groups, comps, tracks, clips) with new UUIDs under a new configuration. Clips retain their `library_entry_id` references. This is the "Save As" operation.

**`database/hierarchy.rs`**:
- `insert_group(conn, name, configuration_id)` — signature gains `configuration_id` parameter so new groups are created under the active config.
- `next_sort_order_top` for groups changes to scope by `configuration_id`.

### Actions layer (`actions.rs`)

New actions:
- `create_configuration(container, name)` — inserts config, sets it active, returns AppState.
- `duplicate_configuration(container, name)` — deep-copies active config into new config with given name, sets new config active, returns AppState.
- `load_configuration(container, config_id)` — stops all audio, sets config active, returns AppState.
- `delete_configuration(container, config_id)` — validates it's not the only config (error if so), if it's the active config switches to the first remaining config by sort order, deletes it, returns AppState.
- `rename_configuration(container, config_id, name)` — renames, returns AppState.

Modified actions:
- `add_group` — passes the active config ID through to `hierarchy::insert_group`.

All other existing actions are unchanged — they operate on rows that already belong to a configuration through the hierarchy FKs.

### Commands layer (`commands.rs`)

New Tauri commands (thin wrappers following existing pattern):
- `create_configuration(name: String) -> AppState`
- `duplicate_configuration(name: String) -> AppState`
- `load_configuration(config_id: String) -> AppState`
- `delete_configuration(config_id: String) -> AppState`
- `rename_configuration(config_id: String, name: String) -> AppState`

## Frontend Changes

### Store (`appStore.ts`)

New refs:
```typescript
const configurations = ref<Configuration[]>([]);
const activeConfigurationId = ref<string>('');
```

`hydrate()` picks up the two new `AppState` fields:
```typescript
function hydrate(state: AppState): void {
    groups.value = state.groups;
    libraryEntries.value = state.libraryEntries;
    tags.value = state.tags;
    configurations.value = state.configurations;
    activeConfigurationId.value = state.activeConfigurationId;
}
```

No other store changes — the existing `command()` helper handles the new config commands the same way.

### UI: Configuration Picker

A small component in the app's top bar or sidebar header. Shows the active configuration name and provides:
- Dropdown to switch between configs (calls `load_configuration`)
- "New" action (calls `create_configuration` with a name prompt)
- "Save As" action (calls `duplicate_configuration` with a name prompt)
- Rename (inline edit or dialog)
- Delete (with confirmation, disabled if only one config exists)

Self-contained component — no interaction with soundstage or library views.

### Generated types

`ts-rs` auto-generates `Configuration.ts` and updates `AppState.ts` on compile. No manual TypeScript changes needed.

### Remote

The WebSocket server already broadcasts full `AppState` on mutations. Since `AppState` will include `configurations` and `activeConfigurationId`, remotes receive config info automatically. Exposing config switching on remotes is deferred — the data is available when wanted.

## What Does NOT Change

- `Group`, `Comp`, `Track`, `Clip` structs
- `TrackMode`, `TrackTransition`, `TrackPlaybackState`
- Library tables and library CRUD
- Audio engine (`audio/` module)
- Waveform/peaks system
- All existing hierarchy mutation commands (other than `add_group` gaining config awareness)
- Remote control WebSocket protocol (it just gets richer `AppState` payloads)
