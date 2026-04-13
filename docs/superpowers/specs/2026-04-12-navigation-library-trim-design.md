# Navigation, Library Waveform Editor, and Trim Points

## Problem

LuteDrop has no routing — it's a single-view app with a v-if toggle between Soundstage (playback) and Library. The app bar is cluttered with controls for both contexts. The waveform editor for loop points lives in TrackSettings (Soundstage) but loop/trim points are library entry properties. Trim points exist in the database schema but aren't exposed in UI or respected by the audio engine.

## Solution

Add vue-router with a nav rail, restructure the app into proper routes, move the waveform editor to a full-width Library dialog with trim and loop point editing, and make the audio engine respect trim points during playback. Enforce data consistency: loop points are always clamped within trim bounds at write time, starts never exceed ends.

## Routing

### Routes

| Route | Path | View |
|---|---|---|
| Soundstage | `/` | `SoundstageView.vue` |
| Library | `/library` | `LibraryView.vue` |

### App shell (App.vue)

App.vue becomes a minimal shell:
- Left: nav rail (always visible)
- Top: app bar (content changes per route)
- Center: `<router-view />`
- Bottom: persistent transport bar

All current App.vue logic (group tabs, transport controls, dialogs) moves out into route-specific views and the transport bar component.

### Nav rail

Always visible, ~56px wide. Two icons stacked vertically:
- Soundstage: `mdi-speaker-group` (or similar)
- Library: `mdi-bookshelf`

Active route highlighted with primary color. Expands to show text labels on hover.

### App bar (per-route)

**Soundstage**: group tabs, new group button, server URL info. Essentially the current tab bar.

**Library**: search bar, tag filter chips, import file/folder buttons.

### Bottom transport bar (persistent)

Visible on all routes. Contains:
- Master volume slider
- Stop All / Pause All / Resume All buttons
- "N tracks playing" status indicator

This replaces the transport controls currently in the app bar.

### Remote control

The remote (WebSocket client) continues to show only Soundstage functionality. No Library access from remote. No changes needed to the remote control system.

## Library Waveform Editor Dialog

### Opening

A button or click action on a `LibraryEntryRow` opens a full-width dialog (`VDialog`, `max-width` near 100%) for editing that library entry's audio properties.

### Dialog contents

- **Entry name** — editable text field
- **Tags** — editable comma-separated or chip-based input
- **Gain volume** — slider (0.0 to 2.0 or similar range)
- **Waveform display** — the existing `WaveformEditor` component, adapted:
  - Shows the full audio file waveform
  - **Trim markers** (new) — draggable start/end markers defining the playable region. Visually distinct from loop markers (different color, e.g., orange/red vs green for loop).
  - **Loop markers** — draggable start/end markers within the trim region. Same as current loop markers but constrained to the trim region.
  - Dimmed/grayed waveform outside the trim region to visually indicate trimmed sections
- **Numeric inputs** — text fields for trim start, trim end, loop start, loop end for precise entry
- **Close/Save** — closing the dialog saves automatically (each change fires a command immediately, same as current behavior)

### Waveform editor UI constraints

The waveform editor enforces all constraints visually during interaction:

- **Trim start** cannot be dragged past trim end
- **Trim end** cannot be dragged before trim start
- **Loop start** cannot be dragged before trim start or past loop end
- **Loop end** cannot be dragged past trim end or before loop start
- **When trim start is dragged past loop start**: loop start snaps to the new trim start position
- **When trim end is dragged before loop end**: loop end snaps to the new trim end position
- **Numeric inputs** are clamped on blur/enter to respect the same constraints

All four markers are visible simultaneously on the waveform. Trim markers define the outer boundary, loop markers define the inner region.

### Remove waveform editor from TrackSettings

TrackSettings for Seamless mode shows the mode selector and repeat toggle only. No inline waveform. Users edit loop/trim points in the Library.

## Trim Point Support in Audio Engine

### Playback behavior

In `start_clip_inner` (audio/engine.rs), when starting a clip:

1. **Start position**: if `trim_start` is set, use it as the start position (unless a specific seek position is requested that's within the trim range).
2. **End boundary**: if `trim_end` is set:
   - **Seamless mode**: the loop region's end is capped to `trim_end`. The loop region's start is floored to `trim_start`.
   - **Sequence/Shuffle mode**: the engine treats `trim_end` as the end of the clip for advancement purposes. When playback reaches `trim_end`, advance to the next clip.
3. **Crossfade timing**: crossfade triggers are calculated relative to `trim_end` (not the file's actual end). If `trim_end = 120.0` and crossfade overlap is 4 seconds, the crossfade starts at 116.0.

### Clip model changes

Add `trim_start: Option<f64>` and `trim_end: Option<f64>` to the `Clip` struct. Resolved from the library entry at state-load time, same as `library_volume` and loop points.

### Duration reporting

The clip's effective duration for crossfade/advancement calculations is `trim_end - trim_start` (or `trim_end - 0` if no trim_start, or `file_duration - trim_start` if no trim_end).

## Data Consistency Rules

### Invariants

At all times in the database:
- `trim_start < trim_end` (when both are set)
- `loop_start >= trim_start` (when both are set)
- `loop_end <= trim_end` (when both are set)
- `loop_start < loop_end` (when both are set)
- Start values never exceed their corresponding end values

### Enforcement at write time

**When updating trim points** (`update_library_entry_trim`):
1. Validate: if both set, `trim_start < trim_end`. Return error if not.
2. Write trim_start, trim_end.
3. If `loop_start` is set and `loop_start < trim_start` → set `loop_start = trim_start`.
4. If `loop_end` is set and `trim_end` is set and `loop_end > trim_end` → set `loop_end = trim_end`.
5. If after clamping `loop_start >= loop_end` → clear both to NULL.

**When updating loop points** (`update_library_entry_loop`):
1. Read current trim values.
2. If `trim_start` is set and `loop_start < trim_start` → clamp `loop_start = trim_start`.
3. If `trim_end` is set and `loop_end > trim_end` → clamp `loop_end = trim_end`.
4. Validate: `loop_start < loop_end`. Return error if not.
5. Write loop_start, loop_end.

This ensures the database is always in a valid state. The UI gets back correctly clamped values through normal state hydration. The engine trusts what it reads.

## Files Changed

### New files
- `src/router.ts` — vue-router setup with two routes
- `src/views/SoundstageView.vue` — renamed/refactored from PlaybackView.vue
- `src/components/TransportBar.vue` — persistent bottom bar
- `src/components/WaveformEditorDialog.vue` — full-width library entry editor

### Modified files
- `package.json` — add `vue-router` dependency
- `src/main.ts` — register router
- `src/App.vue` — slim shell: nav rail + app bar + router-view + transport bar
- `src/views/LibraryView.vue` — move search/tags/import to app bar area, add waveform dialog trigger
- `src/components/TrackSettings.vue` — remove waveform editor, simplify Seamless mode UI
- `src/components/WaveformEditor.vue` — add trim markers, dim outside trim region, enforce drag constraints
- `src-tauri/src/models.rs` — add trim_start/trim_end to Clip struct
- `src-tauri/src/database/mod.rs` — resolve trim points in load_clips
- `src-tauri/src/database/library.rs` — add clamping logic to update_library_entry_trim and update_library_entry_loop
- `src-tauri/src/audio/engine.rs` — respect trim_start/trim_end in start_clip_inner, adjust crossfade timing
- `generated/` — regenerated types

### Deleted files
- `src/views/PlaybackView.vue` — replaced by SoundstageView.vue
