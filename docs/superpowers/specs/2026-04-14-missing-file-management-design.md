# Missing File Management Design

## Problem

When audio files referenced by library entries are moved, renamed, or become unavailable (e.g., external drive unplugged), the app needs to detect this, surface it clearly across the UI, and provide a straightforward path to fix it. The current system checks for missing files at launch and shows basic indicators in the library view, but the detection is frontend-only, clips don't reflect their broken state, the waveform editor doesn't show file paths or offer relocation, and there's no runtime detection if files disappear mid-session.

## Goals

1. Make the backend the single source of truth for missing-file state
2. Detect missing files at launch, during commands, and during autonomous playback
3. Surface broken state on both library entries and clips that reference them
4. Provide relocation from the waveform editor dialog and directly from broken clips
5. Migrate the library view to a sortable data table (separate branch)

## Architecture

### Branch 1: Core Missing File Management

#### Backend State (`state.rs`)

Add a `missing_entry_ids: Mutex<HashSet<String>>` field to `AppStateContainer`.

Two methods on `AppStateContainer`:
- `mark_entry_missing(id: &str) -> bool` — inserts the ID, returns `true` if newly added
- `mark_entry_found(id: &str) -> bool` — removes the ID, returns `true` if it was present

These are the **only** mutation path for the missing set. All detection code paths call these methods.

#### AppState Integration

Add `missing_entry_ids: Vec<String>` to the `AppState` struct returned to the frontend. When `load_full_state` builds `AppState`, it reads the current snapshot from `container.missing_entry_ids` and includes it. The frontend's `hydrate()` method syncs the local reactive `missingEntryIds` set from this field.

Remove the separate `check_missing_files` Tauri command and the frontend's `checkMissingFiles()` call in `App.vue`. The launch scan still runs (see next section), but it populates the backend set directly — the frontend receives missing state through `AppState` hydration, no separate invoke needed.

#### Detection: Launch Scan

On startup, iterate all library entries, check `Path::new(&entry.file_path).exists()`, call `mark_entry_missing(id)` for each missing entry. This populates the backend set before the first `AppState` is sent to the frontend.

#### Detection: Command-Level

When any Tauri command encounters a file-not-found error while accessing a library entry's file, it calls `container.mark_entry_missing(entry_id)` before returning. Affected commands:
- `play_library_sound` — preview playback in waveform editor
- `get_waveform_data` — waveform rendering
- Any future command that reads from `file_path`

Since most commands return `AppState` via hydrate, the frontend picks up the updated missing set automatically.

When `relocate_library_entry` succeeds and confirms the new file exists, it calls `container.mark_entry_found(entry_id)`.

#### Detection: Polling Loop

The audio engine's `poll()` method auto-advances tracks to the next clip. When `start_clip()` fails because the file doesn't exist:

1. The orchestrator identifies the `library_entry_id` from the failed clip
2. `poll()` returns the failed entry ID alongside normal `TrackPlaybackState` results (e.g., as part of a struct or tuple return)
3. In `lib.rs`, the polling loop receives this, calls `container.mark_entry_missing(entry_id)`
4. If `mark_entry_missing` returns `true` (newly discovered), emit a `missing-entries-changed` Tauri event with the entry ID
5. Frontend listens for `missing-entries-changed`, adds the ID to its local set, shows a toast

The event only fires when the set **changes** — not every 50ms tick. Repeated encounters of the same missing file are no-ops.

#### Frontend Event Listener

Register a listener for `missing-entries-changed` in the audio composable or app initialization:
- Receives the newly-missing entry ID
- Adds it to `store.missingEntryIds`
- Shows a toast: "Audio file not found: {entry_name}"

#### Waveform Editor Dialog (`WaveformEditorDialog.vue`)

Add a file path display and relocate action inside the dialog body, above the waveform, in the same area as the other form fields (name, tags, volume):

- **File path text** — shows the entry's `file_path`, truncated from the left with ellipsis so the filename (tail end) is always visible
- **Relocate button** — folder icon button next to the path. Opens the native file picker, calls `relocate_library_entry`, then `mark_entry_found` clears the missing state
- **Missing state styling** — if the entry is missing, the path text gets a warning color and the relocate button is more prominent (outlined/colored). If the entry is not missing, the button is still present but subtle (users may want to change the path even for valid files)

#### Clip Broken Indicator

In the clip components (`TrackClipList.vue` / clip rendering):

- **Visual indicator** — when `store.isEntryMissing(clip.library_entry_id)` is true, show a warning icon and reduced opacity on the clip, matching the visual language of the library entry row
- **Direct relocate on click** — clicking the warning icon opens the native file picker directly (skips the dialog), calls `relocate_library_entry` + `mark_entry_found`. Quick fix path.
- **Dialog still accessible** — the normal way to open the waveform dialog for a clip's entry still works, providing the full-context path for users who need it
- **Reactive clearing** — when a missing entry is relocated (from any path: library view, waveform dialog, or clip indicator), the clip's indicator clears automatically because `missingEntryIds` is reactive

#### Library Entry Row (`LibraryEntryRow.vue`)

Existing visual indicators (warning icon, reduced opacity) and relocate button remain. No changes needed — they already read from `store.isEntryMissing()`. The relocate flow just needs to call `mark_entry_found` on success, which happens automatically since it goes through the same `relocate_library_entry` command.

### Branch 2: Library Data Table Migration

Separate branch, merged after Branch 1 is tested and stable.

#### Replace VList with v-data-table

Replace the current `VList` + `VListItem` + `LibraryEntryRow` rendering in `LibraryView.vue` with Vuetify's `v-data-table`.

**Columns:**
- Status — missing indicator icon (sortable: missing entries sort to top)
- Name — entry name (sortable)
- Duration — formatted duration (sortable)
- Tags — rendered as chips
- Actions — play/stop, relocate, delete buttons

**Row behavior:**
- Clickable rows via `rowProps` — clicking a row opens the waveform editor dialog
- Custom cell content via `item.{key}` slots for icons, chips, buttons
- `hover` prop for visual feedback
- Selection support if needed (checkboxes via `show-select`)

**Sorting:**
- Built-in click-to-sort on column headers
- Status column uses a custom sort function: missing entries (`isEntryMissing`) sort before found entries
- Default sort: no sort applied (original order)

**Styling:**
- Density and CSS adjustments to maintain a list-like feel rather than a spreadsheet look
- Consistent with the rest of the app's visual language

## State Flow Summary

```
Detection Sources              Backend Truth              Frontend Sync
─────────────────            ──────────────────          ───────────────
Launch scan          ──┐
                       ├──▶  AppStateContainer     ──▶  AppState hydration
Command errors       ──┤     .missing_entry_ids          (every command)
                       │     (Mutex<HashSet>)
Poll loop errors     ──┘          │                 ──▶  missing-entries-changed
                                  │                      event (poll loop only,
                                  ▼                      on set change)
                          mark_entry_missing()
                          mark_entry_found()
                          (single mutation path)
```

## What's NOT Changing

- Database schema — no new tables or columns. Missing state is runtime-derived.
- `LibraryEntry` model — `file_path` field stays the same.
- Clip model — `library_entry_id` foreign key stays the same.
- Audio engine internals — Kira playback, crossfade looping, etc. untouched.
