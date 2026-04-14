# Missing File Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the backend the single source of truth for missing-file state, detect missing files at launch and runtime, surface broken state on library entries and clips, and provide relocation from the waveform editor and clips.

**Architecture:** `AppStateContainer` holds a `Mutex<HashSet<String>>` for missing entry IDs with two mutation methods. All detection paths (launch scan, command errors, poll loop errors) feed this single set. The frontend syncs via `AppState` hydration (for commands) and a `missing-entries-changed` Tauri event (for the poll loop). UI surfaces broken state on library rows, the waveform editor dialog, and clip rows.

**Tech Stack:** Rust (Tauri backend), Vue 3 + Vuetify 4 (frontend), Pinia (state), TypeScript, ts-rs (type generation)

---

## File Map

**Modify:**
- `src-tauri/src/state.rs` — Add `missing_entry_ids` field and mutation methods
- `src-tauri/src/models.rs` — Add `missing_entry_ids` to `AppState`
- `src-tauri/src/actions.rs` — Add `with_missing` enrichment, launch scan, command-level detection, remove old `check_missing_files`
- `src-tauri/src/commands.rs` — Remove `check_missing_files` command, add `entry_id` to `play_library_sound` and `get_waveform_data`
- `src-tauri/src/lib.rs` — Run launch scan in setup, handle poll loop missing detection, remove `check_missing_files` from handler list
- `src-tauri/src/audio/mod.rs` — Change `poll()` return type to include missing entry IDs
- `src-tauri/src/audio/orchestrator.rs` — Change `poll_track` to collect missing entry IDs
- `src/stores/appStore.ts` — Update `hydrate()` to sync missing set, remove `checkMissingFiles()`
- `src/composables/useAudio.ts` — Add `missing-entries-changed` event listener, add `entry_id` to `playLibrarySound`
- `src/App.vue` — Remove `checkMissingFiles()` call, simplify `onMounted`
- `src/components/WaveformEditorDialog.vue` — Add file path display and relocate button
- `src/components/ClipRow.vue` — Add missing indicator and direct relocate
- `src/views/LibraryView.vue` — Update `relocateEntry` to drop `checkMissingFiles` call

---

### Task 1: Add missing_entry_ids to AppStateContainer

**Files:**
- Modify: `src-tauri/src/state.rs:1-27`

- [ ] **Step 1: Add the field and methods to AppStateContainer**

```rust
// state.rs — full replacement
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use rusqlite::Connection;
use tokio::sync::broadcast;
use crate::audio::AudioEngine;
use crate::waveform::PeakCache;

pub struct AppStateContainer {
    pub db: Mutex<Connection>,
    pub audio: Mutex<AudioEngine>,
    pub broadcast_tx: broadcast::Sender<String>,
    pub peak_cache: PeakCache,
    pub db_path: PathBuf,
    pub missing_entry_ids: Mutex<HashSet<String>>,
}

impl AppStateContainer {
    pub fn new(db: Connection, audio: AudioEngine, peak_cache_dir: PathBuf, db_path: PathBuf) -> Self {
        let (broadcast_tx, _) = broadcast::channel(64);
        Self {
            db: Mutex::new(db),
            audio: Mutex::new(audio),
            broadcast_tx,
            peak_cache: PeakCache::new(peak_cache_dir),
            db_path,
            missing_entry_ids: Mutex::new(HashSet::new()),
        }
    }

    /// Mark an entry as missing. Returns true if it was newly added.
    pub fn mark_entry_missing(&self, id: &str) -> bool {
        if let Ok(mut set) = self.missing_entry_ids.lock() {
            set.insert(id.to_string())
        } else {
            false
        }
    }

    /// Mark an entry as found (no longer missing). Returns true if it was removed.
    pub fn mark_entry_found(&self, id: &str) -> bool {
        if let Ok(mut set) = self.missing_entry_ids.lock() {
            set.remove(id)
        } else {
            false
        }
    }

    /// Snapshot the current missing entry IDs as a Vec.
    pub fn missing_snapshot(&self) -> Vec<String> {
        self.missing_entry_ids
            .lock()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo check 2>&1 | head -20`
Expected: compiles with no errors (new fields/methods are unused warnings only)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/state.rs
git commit -m "feat: add missing_entry_ids to AppStateContainer with mutation methods"
```

---

### Task 2: Add missing_entry_ids to AppState model

**Files:**
- Modify: `src-tauri/src/models.rs:105-126`

- [ ] **Step 1: Add the field to the AppState struct**

In `src-tauri/src/models.rs`, add `missing_entry_ids` to the `AppState` struct and its `Default` impl:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub groups: Vec<Group>,
    pub library_entries: Vec<LibraryEntry>,
    pub tags: Vec<String>,
    pub configurations: Vec<Configuration>,
    pub active_configuration_id: String,
    pub missing_entry_ids: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            library_entries: Vec::new(),
            tags: Vec::new(),
            configurations: Vec::new(),
            active_configuration_id: String::new(),
            missing_entry_ids: Vec::new(),
        }
    }
}
```

- [ ] **Step 2: Regenerate TypeScript types**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo test 2>&1 | tail -5`

This triggers ts-rs export. Verify `generated/AppState.ts` now includes `missingEntryIds: Array<string>`.

- [ ] **Step 3: Verify it compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo check 2>&1 | head -20`
Expected: compiles (database/mod.rs `load_full_state` will need updating — it won't include the new field yet, which causes a compile error. Fix in next step.)

- [ ] **Step 4: Update load_full_state to include the new field**

In `src-tauri/src/database/mod.rs`, update the `load_full_state` function at line 210 to include the new field with an empty default (it gets filled in by actions later):

```rust
    Ok(AppState {
        groups,
        library_entries,
        tags,
        configurations,
        active_configuration_id,
        missing_entry_ids: Vec::new(),
    })
```

- [ ] **Step 5: Verify it compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo check 2>&1 | head -20`
Expected: compiles clean

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/models.rs src-tauri/src/database/mod.rs generated/AppState.ts
git commit -m "feat: add missing_entry_ids field to AppState model"
```

---

### Task 3: Enrich AppState with missing set in actions

**Files:**
- Modify: `src-tauri/src/actions.rs`

- [ ] **Step 1: Add a load_state helper that enriches AppState with missing IDs**

Add this function near the top of `actions.rs`, after the existing `with_db` helper (around line 15):

```rust
/// Loads full state from DB and enriches it with the missing-entry-ids snapshot.
fn load_state(container: &AppStateContainer) -> Result<AppState, String> {
    let mut state = with_db(container, |conn| database::load_full_state(conn))?;
    state.missing_entry_ids = container.missing_snapshot();
    Ok(state)
}
```

- [ ] **Step 2: Update load_data to use load_state**

Replace the `load_data` function body:

```rust
pub fn load_data(container: &AppStateContainer) -> Result<AppState, String> {
    load_state(container)
}
```

- [ ] **Step 3: Update all mutation actions to enrich with missing set**

For every action that returns `Result<AppState, String>` and currently ends with `database::load_full_state(conn)` inside a `with_db` closure, restructure to: do the mutation inside `with_db`, then call `load_state`. This applies to all of these functions:

- `add_group` — change to:
```rust
pub fn add_group(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        let config_id = database::get_active_configuration_id(conn)?;
        hierarchy::insert_group(conn, &name, &config_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `delete_group`:
```rust
pub fn delete_group(container: &AppStateContainer, group_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_group(conn, group_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `rename_group`:
```rust
pub fn rename_group(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_group(conn, group_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `add_comp`:
```rust
pub fn add_comp(container: &AppStateContainer, group_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_comp(conn, group_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `delete_comp`:
```rust
pub fn delete_comp(container: &AppStateContainer, _group_id: &str, comp_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_comp(conn, comp_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `rename_comp`:
```rust
pub fn rename_comp(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::rename_comp(conn, comp_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `add_track`:
```rust
pub fn add_track(container: &AppStateContainer, comp_id: &str, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_track(conn, comp_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `delete_track`:
```rust
pub fn delete_track(container: &AppStateContainer, _comp_id: &str, track_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_track(conn, track_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `update_track` — this one has more complex logic inside `with_db`. Keep the mutation inside `with_db` but remove the `load_full_state` call at the end:
```rust
pub fn update_track(container: &AppStateContainer, _comp_id: &str, track_id: &str, updates: serde_json::Value) -> Result<AppState, String> {
    with_db(container, |conn| {
        // ... all the existing mutation logic stays the same ...
        // Remove the final: database::load_full_state(conn)
        // Replace with:
        Ok(())
    })?;
    load_state(container)
}
```

- `add_clip`:
```rust
pub fn add_clip(container: &AppStateContainer, track_id: &str, library_entry_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::insert_clip(conn, track_id, library_entry_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `delete_clip`:
```rust
pub fn delete_clip(container: &AppStateContainer, _track_id: &str, clip_id: &str) -> Result<AppState, String> {
    with_db(container, |conn| {
        hierarchy::delete_clip(conn, clip_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `create_configuration`:
```rust
pub fn create_configuration(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        let id = configurations::insert_configuration(conn, &name)?;
        configurations::set_active_configuration(conn, &id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `duplicate_configuration`:
```rust
pub fn duplicate_configuration(container: &AppStateContainer, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        let active_id = database::get_active_configuration_id(conn)?;
        let new_id = configurations::duplicate_configuration(conn, &active_id, &name)?;
        configurations::set_active_configuration(conn, &new_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `load_configuration` (note: stop audio first, then mutation, then load):
```rust
pub fn load_configuration(container: &AppStateContainer, config_id: String) -> Result<AppState, String> {
    {
        let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
        audio.stop_all();
    }
    with_db(container, |conn| {
        configurations::set_active_configuration(conn, &config_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `delete_configuration`:
```rust
pub fn delete_configuration(container: &AppStateContainer, config_id: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM configurations", [], |row| row.get(0))
            .map_err(|e| format!("Failed to count configurations: {e}"))?;
        if count <= 1 {
            return Err("Cannot delete the only configuration".to_string());
        }
        let active_id = database::get_active_configuration_id(conn)?;
        if active_id == config_id {
            let next_id: String = conn
                .query_row(
                    "SELECT id FROM configurations WHERE id != ?1 ORDER BY sort_order ASC LIMIT 1",
                    params![config_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("Failed to find next configuration: {e}"))?;
            configurations::set_active_configuration(conn, &next_id)?;
        }
        configurations::delete_configuration(conn, &config_id)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `rename_configuration`:
```rust
pub fn rename_configuration(container: &AppStateContainer, config_id: String, name: String) -> Result<AppState, String> {
    with_db(container, |conn| {
        configurations::rename_configuration(conn, &config_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `import_library_entry` — keep all DB work inside `with_db`, just replace `load_full_state` at the end:
```rust
pub fn import_library_entry(
    container: &AppStateContainer,
    file_path: String,
    content_hash: Option<String>,
    name: String,
    duration: Option<f64>,
    sample_rate: Option<i64>,
    file_size: Option<i64>,
) -> Result<AppState, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let hash = if let Some(h) = content_hash {
        Some(h)
    } else {
        let mut file = std::fs::File::open(&file_path)
            .map_err(|e| format!("Failed to open file for hashing: {e}"))?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(e) => return Err(format!("Failed to read file for hashing: {e}")),
            }
        }
        Some(format!("{:x}", hasher.finalize()))
    };

    with_db(container, |conn| {
        if let Some(ref h) = hash {
            if let Some((_id, existing_name)) = library::get_library_entry_by_hash(conn, h)? {
                return Err(format!("Already in library as '{}'", existing_name));
            }
        }
        let entry_id = library::insert_library_entry(
            conn, &file_path, hash.as_deref(), &name, duration, sample_rate, file_size,
        )?;
        if duration.is_none() {
            if let Some(probed_duration) = crate::audio::util::probe_duration(&file_path) {
                let _ = library::update_library_entry_duration(conn, &entry_id, probed_duration, None);
            }
        }
        Ok(())
    })?;
    load_state(container)
}
```

- `update_library_entry_tags`:
```rust
pub fn update_library_entry_tags(
    container: &AppStateContainer,
    entry_id: &str,
    tags: Vec<String>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        let existing_tags = library::get_tags_for_entry(conn, entry_id)?;
        for tag_name in &existing_tags {
            library::remove_tag_from_entry(conn, entry_id, tag_name)?;
        }
        for tag_name in &tags {
            library::add_tag_to_entry(conn, entry_id, tag_name)?;
        }
        Ok(())
    })?;
    load_state(container)
}
```

- `update_library_entry_markers_action`:
```rust
pub fn update_library_entry_markers_action(
    container: &AppStateContainer,
    entry_id: &str,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_markers(conn, entry_id, trim_start, trim_end, loop_start, loop_end)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `update_library_entry_volume_action`:
```rust
pub fn update_library_entry_volume_action(
    container: &AppStateContainer,
    entry_id: &str,
    volume: f64,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_volume(conn, entry_id, volume)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `rename_library_entry_action`:
```rust
pub fn rename_library_entry_action(
    container: &AppStateContainer,
    entry_id: &str,
    name: String,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_name(conn, entry_id, &name)?;
        Ok(())
    })?;
    load_state(container)
}
```

- `delete_library_entry_action`:
```rust
pub fn delete_library_entry_action(
    container: &AppStateContainer,
    entry_id: &str,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::delete_library_entry(conn, entry_id)?;
        Ok(())
    })?;
    // Also remove from missing set if it was there
    container.mark_entry_found(entry_id);
    load_state(container)
}
```

- `relocate_library_entry`:
```rust
pub fn relocate_library_entry(
    container: &AppStateContainer,
    entry_id: &str,
    new_file_path: String,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_file_path(conn, entry_id, &new_file_path)?;
        Ok(())
    })?;
    // Verify file exists and mark as found
    if std::path::Path::new(&new_file_path).exists() {
        container.mark_entry_found(entry_id);
    }
    load_state(container)
}
```

- [ ] **Step 4: Remove old check_missing_files function**

Delete the `check_missing_files` function (lines 412-423 of actions.rs).

- [ ] **Step 5: Add scan_missing_files function for launch**

Add this new function to actions.rs (replaces the old `check_missing_files`):

```rust
/// Scans all library entries for missing files and populates the backend set.
/// Called once at launch before any frontend state is sent.
pub fn scan_missing_files(container: &AppStateContainer) {
    let entries = match with_db(container, |conn| database::load_full_state(conn)) {
        Ok(state) => state.library_entries,
        Err(_) => return,
    };
    for entry in &entries {
        if !std::path::Path::new(&entry.file_path).exists() {
            container.mark_entry_missing(&entry.id);
        }
    }
}
```

- [ ] **Step 6: Verify it compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo check 2>&1 | head -30`
Expected: compiles (commands.rs will error on `check_missing_files` — that's fixed in Task 4)

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/actions.rs
git commit -m "feat: enrich all AppState responses with missing_entry_ids snapshot"
```

---

### Task 4: Update commands and lib.rs

**Files:**
- Modify: `src-tauri/src/commands.rs:275-280`
- Modify: `src-tauri/src/lib.rs:49-89, 128`

- [ ] **Step 1: Remove check_missing_files command**

In `src-tauri/src/commands.rs`, delete the `check_missing_files` function (lines 275-280):

```rust
// DELETE THIS:
#[tauri::command]
pub fn check_missing_files(
    state: State<'_, Arc<AppStateContainer>>,
) -> Result<Vec<String>, String> {
    actions::check_missing_files(&state)
}
```

- [ ] **Step 2: Remove check_missing_files from handler list in lib.rs**

In `src-tauri/src/lib.rs`, remove `commands::check_missing_files,` from the `invoke_handler` list (line 128).

- [ ] **Step 3: Add launch scan to lib.rs setup**

In `src-tauri/src/lib.rs`, add the missing file scan right after the container is created (after line 49, before `app.manage`):

```rust
            // Scan for missing files before sharing state with frontend
            actions::scan_missing_files(&container);
```

- [ ] **Step 4: Verify it compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo check 2>&1 | head -20`
Expected: compiles clean

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat: run missing file scan at launch, remove check_missing_files command"
```

---

### Task 5: Frontend hydration and cleanup

**Files:**
- Modify: `src/stores/appStore.ts`
- Modify: `src/App.vue`
- Modify: `src/views/LibraryView.vue:56`

- [ ] **Step 1: Update hydrate() to sync missingEntryIds from AppState**

In `src/stores/appStore.ts`, update the `hydrate` function (line 25-31):

```typescript
  function hydrate(state: AppState): void {
    groups.value = state.groups;
    libraryEntries.value = state.libraryEntries;
    tags.value = state.tags;
    configurations.value = state.configurations;
    activeConfigurationId.value = state.activeConfigurationId;
    missingEntryIds.value = new Set(state.missingEntryIds);
  }
```

- [ ] **Step 2: Remove checkMissingFiles function**

Delete the `checkMissingFiles` function (lines 134-142) and remove it from the return object (line 165).

- [ ] **Step 3: Simplify App.vue onMounted**

Replace the `onMounted` block in `src/App.vue` (lines 39-47):

```typescript
onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);

  if (store.missingEntryIds.size > 0) {
    toast.showError(`${store.missingEntryIds.size} audio file(s) not found. Check the Library view.`);
  }
});
```

- [ ] **Step 4: Remove checkMissingFiles call from relocateEntry in LibraryView.vue**

In `src/views/LibraryView.vue`, update `relocateEntry` (lines 49-57). Remove the `await store.checkMissingFiles()` line — hydration handles it:

```typescript
async function relocateEntry(entryId: string): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
  });
  if (!path || Array.isArray(path)) return;
  await store.command('relocate_library_entry', { entryId, newFilePath: path });
}
```

- [ ] **Step 5: Verify frontend compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop && npx vue-tsc --noEmit 2>&1 | tail -10`
Expected: no type errors

- [ ] **Step 6: Commit**

```bash
git add src/stores/appStore.ts src/App.vue src/views/LibraryView.vue
git commit -m "feat: sync missingEntryIds from AppState hydration, remove checkMissingFiles"
```

---

### Task 6: Command-level missing file detection

**Files:**
- Modify: `src-tauri/src/actions.rs` (play_library_sound)
- Modify: `src-tauri/src/commands.rs` (play_library_sound, get_waveform_data)
- Modify: `src/composables/useAudio.ts` (playLibrarySound)
- Modify: `src/components/WaveformEditorDialog.vue` (togglePreview)

- [ ] **Step 1: Add entry_id to play_library_sound action**

In `src-tauri/src/actions.rs`, update `play_library_sound` to accept `entry_id` and detect missing files:

```rust
pub fn play_library_sound(
    container: &AppStateContainer,
    entry_id: &str,
    file_path: &str,
    position: f64,
    volume: f64,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    // Check file exists before attempting playback
    if !std::path::Path::new(file_path).exists() {
        container.mark_entry_missing(entry_id);
        return Err(format!("Audio file not found: {}", file_path));
    }
    let mut audio = container.audio.lock().map_err(|e| e.to_string())?;
    let vol = crate::audio::util::amplitude_to_decibels(audio.get_master_volume() * volume);
    let loop_region = match (loop_start, loop_end) {
        (Some(ls), Some(le)) if le > ls => Some((ls, le)),
        (None, Some(le)) if le > 0.0 => Some((0.0, le)),
        _ => None,
    };
    audio.play_library_sound(crate::audio::sound::PlayParams {
        file_path: file_path.to_string(),
        start_position: position,
        volume: vol,
        trim_start,
        trim_end,
        loop_region,
        fade_in: None,
        fade_out: None,
    })
}
```

- [ ] **Step 2: Update play_library_sound command to pass entry_id**

In `src-tauri/src/commands.rs`, update the `play_library_sound` command:

```rust
#[tauri::command]
pub fn play_library_sound(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    file_path: String,
    position: f64,
    volume: f64,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    actions::play_library_sound(&state, &entry_id, &file_path, position, volume, trim_start, trim_end, loop_start, loop_end)
}
```

- [ ] **Step 3: Update get_waveform_data to detect missing files**

In `src-tauri/src/commands.rs`, update `get_waveform_data` to accept an optional `entry_id` and check file existence:

```rust
#[tauri::command]
pub async fn get_waveform_data(
    state: State<'_, Arc<AppStateContainer>>,
    file_path: String,
    width: u32,
    start: Option<f64>,
    end: Option<f64>,
    entry_id: Option<String>,
) -> Result<WaveformData, String> {
    // Check file exists
    if !std::path::Path::new(&file_path).exists() {
        if let Some(ref eid) = entry_id {
            state.mark_entry_missing(eid);
        }
        return Err(format!("Audio file not found: {}", file_path));
    }
    let state_clone = state.inner().clone();
    tokio::task::spawn_blocking(move || -> Result<WaveformData, String> {
        let (peaks, duration) = waveform::extract_peaks(&file_path, width, start, end, &state_clone.peak_cache)?;
        Ok(WaveformData { peaks, duration })
    })
    .await
    .map_err(|e| format!("Waveform task failed: {}", e))?
}
```

- [ ] **Step 4: Update frontend playLibrarySound to pass entryId**

In `src/composables/useAudio.ts`, the `playLibrarySound` function already receives `entryId` as the first parameter (line 152) but doesn't pass it to the backend. Update the invoke call:

```typescript
  async function playLibrarySound(
    entryId: string,
    filePath: string,
    position: number,
    volume: number,
    trimStart?: number | null,
    trimEnd?: number | null,
    loopStart?: number | null,
    loopEnd?: number | null,
  ): Promise<void> {
    try {
      await invoke('play_library_sound', {
        entryId,
        filePath, position, volume,
        trimStart: trimStart ?? null,
        trimEnd: trimEnd ?? null,
        loopStart: loopStart ?? null,
        loopEnd: loopEnd ?? null,
      });
      const appStore = useAppStore();
      appStore.libraryPlayingEntryId = entryId;
    } catch (e) {
      toast.showError(String(e));
    }
  }
```

- [ ] **Step 5: Verify it compiles**

Run both:
- `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo check 2>&1 | head -20`
- `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop && npx vue-tsc --noEmit 2>&1 | tail -10`

Expected: both compile clean

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/actions.rs src-tauri/src/commands.rs src/composables/useAudio.ts
git commit -m "feat: detect missing files in play_library_sound and get_waveform_data commands"
```

---

### Task 7: Polling loop missing file detection

**Files:**
- Modify: `src-tauri/src/audio/orchestrator.rs:227-343`
- Modify: `src-tauri/src/audio/mod.rs:238-324`
- Modify: `src-tauri/src/lib.rs:148-169`

- [ ] **Step 1: Update poll_track to collect missing entry IDs**

In `src-tauri/src/audio/orchestrator.rs`, change `poll_track` signature to accept a mutable vec for collecting missing entries:

```rust
/// Poll a single track for transition actions and execute them.
pub fn poll_track(
    tracks: &mut HashMap<String, ManagedTrack>,
    manager: &mut AudioManager,
    track_id: &str,
    master_volume: f64,
    mute_state: &MuteState,
    missing_entries: &mut Vec<String>,
) {
```

Then, in the match arms where `start_clip` is called and its result is discarded, check for file-not-found on failure:

Replace `let _ = start_clip(...)` calls (lines 317, 320, 329) with a helper closure. Add this inside `poll_track` before the `match action` block:

```rust
    // Helper: attempt start_clip, report missing file on failure
    let try_start_clip = |tracks: &mut HashMap<String, ManagedTrack>,
                          manager: &mut AudioManager,
                          clip_index: usize,
                          start_pos: Option<f64>,
                          apply_fade: bool,
                          missing: &mut Vec<String>| {
        if let Err(_) = start_clip(tracks, manager, track_id, clip_index, start_pos, apply_fade, master_volume, mute_state) {
            if let Some(track) = tracks.get(track_id) {
                if let Some(clip) = track.config.clips.get(clip_index) {
                    if !std::path::Path::new(&clip.file_path).exists() {
                        missing.push(clip.library_entry_id.clone());
                    }
                }
            }
        }
    };
```

Wait — closures that mutably borrow `tracks` won't work alongside passing `tracks` to the closure. Use a regular function call pattern instead. Replace each `let _ = start_clip(...)` with:

For `TransitionAction::StartCrossfade` (line 317):
```rust
        TransitionAction::StartCrossfade { next_clip_index, duration } => {
            if let Some(track) = tracks.get_mut(track_id) {
                let old_sound = track.current_sound.take();
                if let Some(old) = old_sound {
                    track.transitions.begin_crossfade(old, duration);
                }
            }
            if let Err(_) = start_clip(tracks, manager, track_id, next_clip_index, None, false, master_volume, mute_state) {
                if let Some(track) = tracks.get(track_id) {
                    if let Some(clip) = track.config.clips.get(next_clip_index) {
                        if !std::path::Path::new(&clip.file_path).exists() {
                            missing_entries.push(clip.library_entry_id.clone());
                        }
                    }
                }
            }
        }
```

For `TransitionAction::AdvanceClip` (line 320):
```rust
        TransitionAction::AdvanceClip { next_clip_index } => {
            if let Err(_) = start_clip(tracks, manager, track_id, next_clip_index, None, false, master_volume, mute_state) {
                if let Some(track) = tracks.get(track_id) {
                    if let Some(clip) = track.config.clips.get(next_clip_index) {
                        if !std::path::Path::new(&clip.file_path).exists() {
                            missing_entries.push(clip.library_entry_id.clone());
                        }
                    }
                }
            }
        }
```

For `TransitionAction::StartPendingClip` (line 329):
```rust
        TransitionAction::StartPendingClip { clip_index } => {
            if let Err(_) = start_clip(tracks, manager, track_id, clip_index, None, false, master_volume, mute_state) {
                if let Some(track) = tracks.get(track_id) {
                    if let Some(clip) = track.config.clips.get(clip_index) {
                        if !std::path::Path::new(&clip.file_path).exists() {
                            missing_entries.push(clip.library_entry_id.clone());
                        }
                    }
                }
            }
        }
```

Also update the seamless loop crossfade section (around line 285) — currently `sound::play` failure is silently handled. Add file-not-found detection there too:

```rust
                            match sound::play(manager, &mut track.track_handle, params) {
                                Ok(new_snd) => { track.current_sound = Some(new_snd); }
                                Err(_) => {
                                    if !std::path::Path::new(&clip.file_path).exists() {
                                        missing_entries.push(clip.library_entry_id.clone());
                                    }
                                }
                            }
```

- [ ] **Step 2: Update AudioEngine::poll to return missing entry IDs**

In `src-tauri/src/audio/mod.rs`, change `poll` return type and pass `missing_entries` through:

```rust
    pub fn poll(&mut self) -> (Vec<TrackPlaybackState>, Vec<String>) {
        let mut missing_entries: Vec<String> = Vec::new();
        let track_ids: Vec<String> = self.tracks.keys()
            .filter(|id| id.as_str() != LIBRARY_TRACK_ID)
            .cloned()
            .collect();

        for id in &track_ids {
            orchestrator::poll_track(&mut self.tracks, &mut self.manager, id, self.master_volume, &self.mute_state, &mut missing_entries);
        }
```

The rest of the function stays the same until the return. Change the final line from `states` to:

```rust
        (states, missing_entries)
    }
```

- [ ] **Step 3: Update playback_polling_loop in lib.rs**

In `src-tauri/src/lib.rs`, update the polling loop to handle missing entries:

```rust
fn playback_polling_loop(app_handle: tauri::AppHandle, container: Arc<AppStateContainer>) {
    loop {
        std::thread::sleep(Duration::from_millis(50));

        let (states, master_vol, newly_missing) = {
            let mut audio = match container.audio.lock() {
                Ok(a) => a,
                Err(_) => continue,
            };
            let (states, missing) = audio.poll();
            let vol = audio.get_master_volume();
            (states, vol, missing)
        };

        for state in &states {
            let _ = app_handle.emit("playback-state-update", state);
        }
        let _ = app_handle.emit("master-volume-update", master_vol);

        // Report newly discovered missing files
        for entry_id in &newly_missing {
            if container.mark_entry_missing(entry_id) {
                let _ = app_handle.emit("missing-entries-changed", entry_id.clone());
            }
        }

        server::broadcast_state(&container);
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop/src-tauri && cargo check 2>&1 | head -20`
Expected: compiles clean

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/audio/orchestrator.rs src-tauri/src/audio/mod.rs src-tauri/src/lib.rs
git commit -m "feat: detect missing files in polling loop and emit missing-entries-changed event"
```

---

### Task 8: Frontend event listener for missing-entries-changed

**Files:**
- Modify: `src/composables/useAudio.ts`

- [ ] **Step 1: Add listener for missing-entries-changed**

In `src/composables/useAudio.ts`, add a listener inside the `registerListener` function (after line 23):

```typescript
  listen<string>('missing-entries-changed', (event) => {
    const entryId = event.payload;
    store.missingEntryIds.add(entryId);
    const entry = store.findLibraryEntry(entryId);
    const name = entry?.name ?? 'Unknown';
    const toast = useToast();
    toast.showError(`Audio file not found: ${name}`);
  });
```

- [ ] **Step 2: Verify frontend compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop && npx vue-tsc --noEmit 2>&1 | tail -10`
Expected: no type errors

- [ ] **Step 3: Commit**

```bash
git add src/composables/useAudio.ts
git commit -m "feat: listen for missing-entries-changed events from polling loop"
```

---

### Task 9: Waveform editor dialog — file path and relocate

**Files:**
- Modify: `src/components/WaveformEditorDialog.vue`

- [ ] **Step 1: Add relocate function and file path display**

In `src/components/WaveformEditorDialog.vue`, add the relocate import and function in the `<script setup>` section. Add after the existing imports (line 4):

```typescript
import { open } from '@tauri-apps/plugin-dialog';
```

Add the AUDIO_EXTENSIONS constant and relocate function after the `close` function (after line 173):

```typescript
const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];
const isMissing = computed(() => store.isEntryMissing(props.entryId));

async function relocateFile(): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
  });
  if (!path || Array.isArray(path)) return;
  await store.command('relocate_library_entry', { entryId: props.entryId, newFilePath: path });
}
```

- [ ] **Step 2: Add the file path row in the template**

In the template, add the file path display between the `<VCardTitle>` and `<VCardText>` sections. Insert after line 190 (after the closing `</VCardTitle>`), before `<VCardText>`:

```html
      <!-- File path -->
      <div class="d-flex align-center ga-2 px-6 py-2" :style="isMissing ? 'background: rgba(var(--v-theme-warning), 0.08)' : ''">
        <VIcon v-if="isMissing" color="warning" size="small">mdi-alert</VIcon>
        <span
          class="text-caption text-medium-emphasis flex-grow-1 text-truncate"
          :class="isMissing ? 'text-warning' : ''"
          style="direction: rtl; text-align: left;"
          :title="entry.filePath"
        >{{ entry.filePath }}</span>
        <VBtn
          icon="mdi-folder-search"
          size="x-small"
          :variant="isMissing ? 'outlined' : 'text'"
          :color="isMissing ? 'warning' : undefined"
          title="Change file path"
          @click="relocateFile"
        />
      </div>
```

Note: `direction: rtl; text-align: left;` causes CSS text truncation to show the tail end (filename) rather than the drive root.

- [ ] **Step 3: Verify frontend compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop && npx vue-tsc --noEmit 2>&1 | tail -10`
Expected: no type errors

- [ ] **Step 4: Commit**

```bash
git add src/components/WaveformEditorDialog.vue
git commit -m "feat: show file path and relocate button in waveform editor dialog"
```

---

### Task 10: Clip broken indicator with direct relocate

**Files:**
- Modify: `src/components/ClipRow.vue`

- [ ] **Step 1: Add missing detection and relocate to ClipRow**

In `src/components/ClipRow.vue`, add the import for the file dialog (after line 3):

```typescript
import { open } from '@tauri-apps/plugin-dialog';
```

Add the missing state computed and relocate function (after the `editLibraryEntry` function, around line 39):

```typescript
const isMissing = computed(() => store.isEntryMissing(props.clip.libraryEntryId));

const AUDIO_EXTENSIONS = ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'];

async function relocateMissing(): Promise<void> {
  const path = await open({
    multiple: false,
    filters: [{ name: 'Audio', extensions: AUDIO_EXTENSIONS }],
  });
  if (!path || Array.isArray(path)) return;
  await store.command('relocate_library_entry', {
    entryId: props.clip.libraryEntryId,
    newFilePath: path,
  });
}
```

- [ ] **Step 2: Add visual indicator and relocate button in template**

In the template, update the clip row div (line 49-51) to show the missing state:

```html
  <div
    class="clip-row d-flex align-center ga-2 py-2 px-1"
    :class="{ 'clip-active': isCurrentClip, 'clip-missing': isMissing }"
  >
```

Add the warning icon button right before the clip name span (before line 65). Insert after the play button:

```html
    <VBtn
      v-if="isMissing"
      icon="mdi-alert"
      variant="text"
      size="x-small"
      color="warning"
      title="File not found — click to relocate"
      @click.stop="relocateMissing"
    />
```

Update the clip name span to show reduced opacity when missing:

```html
    <span
      class="text-body-2 flex-grow-1 text-truncate"
      :class="{ 'text-primary': isCurrentClip }"
      :style="{ opacity: isMuted || isMissing ? 0.5 : 1 }"
    >{{ clip.name }}</span>
```

- [ ] **Step 3: Add missing style**

Add to the `<style scoped>` section:

```css
.clip-missing {
  border-left: 3px solid rgb(var(--v-theme-warning));
  background: rgba(var(--v-theme-warning), 0.08);
}
```

- [ ] **Step 4: Verify frontend compiles**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop && npx vue-tsc --noEmit 2>&1 | tail -10`
Expected: no type errors

- [ ] **Step 5: Commit**

```bash
git add src/components/ClipRow.vue
git commit -m "feat: show broken indicator on clips with direct relocate action"
```

---

### Task 11: Manual testing

- [ ] **Step 1: Start the dev server**

Run: `cd /c/Users/Infrasparker/Documents/Coding/LuteDrop && npm run tauri dev`

- [ ] **Step 2: Test launch scan**

1. Rename or move an audio file that a library entry references
2. Launch the app
3. Verify: toast shows "X audio file(s) not found"
4. Verify: library entry row shows warning icon and reduced opacity
5. Verify: clips referencing that entry show warning icon

- [ ] **Step 3: Test waveform editor dialog**

1. Click on a library entry to open the waveform editor
2. Verify: file path is visible above the waveform (filename end visible, not drive root)
3. For a missing entry: verify path text is warning-colored and relocate button is prominent
4. Click relocate, pick the moved file
5. Verify: warning clears on the library entry, clips, and dialog

- [ ] **Step 4: Test clip direct relocate**

1. Create a clip pointing to a missing entry
2. Verify: warning icon appears on the clip row
3. Click the warning icon
4. Verify: file picker opens
5. Pick the correct file
6. Verify: warning clears on the clip and the library entry

- [ ] **Step 5: Test command-level detection**

1. With the app running, move a library entry's file
2. Try to preview it in the waveform editor
3. Verify: error toast appears, entry is now marked as missing

- [ ] **Step 6: Test runtime poll detection**

1. Start playing a track with multiple clips in sequence mode
2. While it plays, move/rename the file for the next clip in the sequence
3. When the track tries to advance to that clip, verify: toast appears and the entry/clip get the missing indicator
