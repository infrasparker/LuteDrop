# LuteDrop Tauri + Rust Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate LuteDrop from Electron + Web Audio to Tauri + Rust (kira) for native streaming audio with loop points, while preserving the Vue 3 + Vuetify frontend.

**Architecture:** Single Rust binary via Tauri. Rust handles audio (kira), persistence (serde), and file I/O. Vue 3 frontend communicates through Tauri's `invoke()` IPC. Types defined in Rust, auto-generated to TypeScript via `ts-rs`.

**Tech Stack:** Tauri 2, Rust, kira (audio), symphonia (waveform), serde (JSON), ts-rs (type gen), Vue 3, Vuetify, Pinia, TypeScript

---

## File Map

### New files (Rust backend)
- `src-tauri/Cargo.toml` — Rust dependencies
- `src-tauri/tauri.conf.json` — Tauri app config (window, permissions, app ID)
- `src-tauri/build.rs` — Tauri build script (required boilerplate)
- `src-tauri/src/main.rs` — App entry, registers commands and managed state
- `src-tauri/src/models.rs` — All data types (serde + ts-rs), source of truth
- `src-tauri/src/state.rs` — AppStateContainer with RwLock, shared across commands
- `src-tauri/src/persistence.rs` — Load/save JSON, debounced writes
- `src-tauri/src/audio/mod.rs` — Module declaration
- `src-tauri/src/audio/engine.rs` — kira AudioEngine: playback, modes, transitions, volume, fades
- `src-tauri/src/commands.rs` — All `#[tauri::command]` handlers
- `src-tauri/src/waveform.rs` — symphonia-based peak extraction

### New files (frontend)
- `src/composables/useAudio.ts` — Wraps Tauri invoke() calls for audio commands
- `src/components/WaveformEditor.vue` — Canvas waveform with draggable markers
- `generated/` — ts-rs auto-generated TypeScript types (one file per type)

### Migrated files (frontend — adapted from Electron version)
- `src/main.ts` — Vue app entry (same, no changes needed)
- `src/App.vue` — Remove Electron-specific code, use invoke() + useAudio
- `src/plugins/vuetify.ts` — Identical (dragon's hoard theme)
- `src/stores/appStore.ts` — Replace window.api with invoke()
- `src/views/PlaybackView.vue` — Identical (no Electron-specific code)
- `src/components/GroupTab.vue` — Identical
- `src/components/CompCard.vue` — Replace useAudioEngine with useAudio
- `src/components/CompDetails.vue` — Replace useAudioEngine with useAudio
- `src/components/TrackControl.vue` — Replace useAudioEngine and window.api with useAudio
- `src/components/ClipRow.vue` — Minor: add invoke for real-time clip volume

### Config files
- `package.json` — Vue + Vuetify + Tauri deps (no Electron)
- `vite.config.ts` — Standard Vite + Vue + Vuetify (no electron-vite)
- `tsconfig.json` — Standard, with path alias for generated types
- `index.html` — Standard Vite entry

### Deleted (not carried over from Electron branch)
- `src/main/` (Electron main process)
- `src/preload/` (Electron bridge)
- `src/renderer/src/audio/` (Web Audio engine)
- `src/renderer/src/composables/useAudioEngine.ts`
- `src/shared/types.ts` (replaced by generated types from Rust)
- `src/remote/` (deferred to Phase 2)
- `electron.vite.config.ts`, `electron-builder.yml`, `vite.remote.config.ts`

---

### Task 1: Install Rust & Scaffold Project

**Files:**
- Create: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `src-tauri/src/main.rs`
- Create: `package.json`, `vite.config.ts`, `tsconfig.json`, `index.html`
- Create: `src/main.ts`, `src/App.vue`, `src/plugins/vuetify.ts`

- [ ] **Step 1: Install Rust**

Run:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows, download and run `rustup-init.exe` from https://rustup.rs/ if curl isn't available.

After install, restart your terminal and verify:
```bash
rustc --version
cargo --version
```

Expected: version numbers for both (e.g., `rustc 1.82.x`, `cargo 1.82.x`)

- [ ] **Step 2: Install Tauri system dependencies**

Tauri 2 on Windows needs the WebView2 runtime (comes pre-installed on Windows 11) and Visual Studio C++ build tools. If you already have VS Build Tools for Node native modules, you're set.

Verify:
```bash
cargo install tauri-cli --version "^2"
```

Expected: installs the `cargo-tauri` CLI.

- [ ] **Step 3: Create the tauri-rewrite branch and clean out Electron files**

```bash
cd C:/Users/Infrasparker/Documents/Coding/LuteDrop
git checkout -b tauri-rewrite
git rm -rf src/ electron.vite.config.ts electron-builder.yml vite.remote.config.ts tsconfig.web.json tsconfig.node.json resources/
git commit -m "chore: remove Electron files for Tauri rewrite"
```

- [ ] **Step 4: Create package.json**

```json
{
  "name": "lutedrop",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vue-tsc --noEmit && vite build",
    "tauri": "tauri"
  },
  "dependencies": {
    "@mdi/font": "^7.4.47",
    "@tauri-apps/api": "^2",
    "@tauri-apps/plugin-dialog": "^2",
    "pinia": "^3.0.4",
    "vue": "^3.5.25",
    "vuedraggable": "^4.1.0",
    "vuetify": "^4.0.5"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2",
    "@vitejs/plugin-vue": "^6.0.2",
    "typescript": "^5.9.3",
    "vite": "^7.2.6",
    "vite-plugin-vuetify": "^2.1.3",
    "vue-tsc": "^3.1.6"
  }
}
```

- [ ] **Step 5: Create vite.config.ts**

```typescript
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import vuetify from 'vite-plugin-vuetify';
import { resolve } from 'path';

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [
    vue(),
    vuetify({ autoImport: true }),
  ],
  resolve: {
    alias: {
      '@generated': resolve(__dirname, 'generated'),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: 'ws', host, port: 1421 }
      : undefined,
  },
});
```

- [ ] **Step 6: Create tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2021",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "jsx": "preserve",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "esModuleInterop": true,
    "lib": ["ES2021", "DOM", "DOM.Iterable"],
    "skipLibCheck": true,
    "noEmit": true,
    "paths": {
      "@generated/*": ["./generated/*"]
    }
  },
  "include": ["src/**/*.ts", "src/**/*.vue", "generated/**/*.ts"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

Create `tsconfig.node.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true,
    "composite": true
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **Step 7: Create index.html**

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>LuteDrop</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **Step 8: Create src/main.ts**

```typescript
import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import { vuetify } from './plugins/vuetify';

const app = createApp(App);
app.use(createPinia());
app.use(vuetify);
app.mount('#app');
```

- [ ] **Step 9: Create src/plugins/vuetify.ts**

Copy directly from Electron version — identical file:

```typescript
import 'vuetify/styles';
import '@mdi/font/css/materialdesignicons.css';
import { createVuetify } from 'vuetify';

export const vuetify = createVuetify({
  defaults: {
    VCard: { variant: 'flat', rounded: 'lg' },
    VBtn: { rounded: 'pill' },
    VTextField: { variant: 'solo-filled', rounded: 'lg' },
    VSelect: { variant: 'solo-filled', rounded: 'lg' },
    VSlider: { color: 'primary', trackColor: 'surface-light' },
  },
  theme: {
    defaultTheme: 'lutedrop',
    themes: {
      lutedrop: {
        dark: true,
        colors: {
          background: '#0A0A0C',
          surface: '#141418',
          'surface-light': '#1E1E24',
          'surface-variant': '#28282F',
          primary: '#F0A830',
          secondary: '#FF6D00',
          error: '#D32F2F',
          success: '#FF6D00',
          warning: '#BF360C',
          info: '#F0A830',
          'on-background': '#E8E4DC',
          'on-surface': '#E8E4DC',
          'on-primary': '#0A0A0C',
          'on-secondary': '#0A0A0C',
        },
      },
    },
  },
});
```

- [ ] **Step 10: Create a minimal src/App.vue (placeholder)**

```vue
<script setup lang="ts">
</script>

<template>
  <VApp>
    <VMain class="d-flex align-center justify-center">
      <h1 class="text-primary">LuteDrop</h1>
    </VMain>
  </VApp>
</template>
```

- [ ] **Step 11: Create src-tauri/Cargo.toml**

```toml
[package]
name = "lutedrop"
version = "0.1.0"
edition = "2021"

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
ts-rs = { version = "10", features = ["serde-compat"] }
kira = "0.9"
symphonia = { version = "0.5", features = ["mp3", "wav", "flac", "ogg", "aac"] }
```

Note: Check the latest versions of these crates at crates.io when implementing. The version numbers above are approximate for early-mid 2026.

- [ ] **Step 12: Create src-tauri/build.rs**

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 13: Create src-tauri/tauri.conf.json**

```json
{
  "$schema": "https://raw.githubusercontent.com/nicktomlin/tauri/main/crates/tauri-utils/schema.json",
  "productName": "LuteDrop",
  "version": "0.1.0",
  "identifier": "com.lutedrop",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build"
  },
  "app": {
    "windows": [
      {
        "title": "LuteDrop",
        "width": 1200,
        "height": 800
      }
    ],
    "security": {
      "csp": null
    }
  },
  "plugins": {
    "dialog": {
      "open": true
    }
  }
}
```

- [ ] **Step 14: Create src-tauri/src/main.rs (minimal)**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}
```

- [ ] **Step 15: Install npm dependencies and verify the app runs**

```bash
npm install
cargo tauri dev
```

Expected: A Tauri window opens showing "LuteDrop" in gold text on a dark background (the Vuetify theme). The Rust backend compiles (first build takes a few minutes).

- [ ] **Step 16: Commit**

```bash
git add -A
git commit -m "scaffold: Tauri 2 + Vue 3 + Vuetify project"
```

---

### Task 2: Rust Data Model + Type Generation

**Files:**
- Create: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/main.rs` (add module declaration)

- [ ] **Step 1: Create src-tauri/src/models.rs**

This is the source of truth for all types. The `#[derive(TS)]` macro auto-generates TypeScript when you run `cargo test`.

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub volume: f64,
    pub trim_start: f64,
    pub trim_end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrackTransition {
    Gapless,
    Gap { min: f64, max: f64 },
    Crossfade { duration: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TrackMode {
    Seamless {
        #[serde(rename = "loopPoint")]
        loop_point: f64,
    },
    Sequence {
        #[serde(rename = "loop")]
        should_loop: bool,
        transition: TrackTransition,
    },
    Shuffle {
        #[serde(rename = "loop")]
        should_loop: bool,
        transition: TrackTransition,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub clips: Vec<Clip>,
    pub volume: f64,
    pub mode: TrackMode,
    pub fade_in: f64,
    pub fade_out: f64,
    pub pan: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
pub struct Comp {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
pub struct Group {
    pub id: String,
    pub name: String,
    pub comps: Vec<Comp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub groups: Vec<Group>,
    pub last_saved_at: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            last_saved_at: None,
        }
    }
}

/// Sent from Rust to the frontend when track playback state changes.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../generated/")]
#[serde(rename_all = "camelCase")]
pub struct TrackPlaybackState {
    pub track_id: String,
    pub is_playing: bool,
    pub is_paused: bool,
    pub current_clip_id: Option<String>,
    pub current_time: f64,
}
```

- [ ] **Step 2: Add module declaration to main.rs**

Replace `src-tauri/src/main.rs` with:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}
```

- [ ] **Step 3: Generate TypeScript types**

```bash
cd src-tauri
cargo test
```

Expected: Tests pass and `generated/` directory is created at the project root containing `.ts` files for each type (e.g., `Clip.ts`, `Track.ts`, `AppState.ts`, etc.).

Verify the generated types match the Electron version's `types.ts` structure by inspecting one:
```bash
cat ../generated/Clip.ts
```

Expected output like:
```typescript
// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

export interface Clip { id: string; name: string; filePath: string; volume: number; trimStart: number; trimEnd: number; }
```

- [ ] **Step 4: Create generated/index.ts barrel export**

Manually create this file to re-export all generated types from one place:

```typescript
export type { Clip } from './Clip';
export type { TrackTransition } from './TrackTransition';
export type { TrackMode } from './TrackMode';
export type { Track } from './Track';
export type { Comp } from './Comp';
export type { Group } from './Group';
export type { AppState } from './AppState';
export type { TrackPlaybackState } from './TrackPlaybackState';
```

Note: After running `cargo test`, check the actual filenames generated and adjust this file to match.

- [ ] **Step 5: Verify it compiles**

```bash
cd src-tauri && cargo build
```

Expected: Compiles with no errors.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: Rust data model with ts-rs type generation"
```

---

### Task 3: App State & Persistence

**Files:**
- Create: `src-tauri/src/state.rs`
- Create: `src-tauri/src/persistence.rs`
- Create: `src-tauri/src/commands.rs` (persistence commands only for now)
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Create src-tauri/src/state.rs**

This is the shared state container that all Tauri commands access. `RwLock` allows multiple concurrent readers (audio engine reading track config) but exclusive writers (saving state).

```rust
use std::sync::RwLock;
use crate::models::AppState;

/// Managed state shared across all Tauri commands.
/// Access in commands via `state: tauri::State<'_, AppStateContainer>`.
pub struct AppStateContainer {
    pub data: RwLock<AppState>,
}

impl AppStateContainer {
    pub fn new(initial: AppState) -> Self {
        Self {
            data: RwLock::new(initial),
        }
    }
}
```

- [ ] **Step 2: Create src-tauri/src/persistence.rs**

```rust
use std::fs;
use std::path::PathBuf;
use tauri::Manager;
use crate::models::AppState;

/// Returns the path to the JSON data file in the OS app data directory.
pub fn data_file_path(app: &tauri::AppHandle) -> PathBuf {
    let app_data = app
        .path()
        .app_data_dir()
        .expect("failed to get app data dir");
    fs::create_dir_all(&app_data).ok();
    app_data.join("lutedrop-data.json")
}

pub fn load_from_disk(path: &PathBuf) -> AppState {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => AppState::default(),
    }
}

pub fn save_to_disk(path: &PathBuf, state: &AppState) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}
```

- [ ] **Step 3: Create src-tauri/src/commands.rs (persistence commands)**

```rust
use tauri::State;
use crate::models::AppState;
use crate::persistence;
use crate::state::AppStateContainer;

#[tauri::command]
pub fn load_data(state: State<'_, AppStateContainer>) -> Result<AppState, String> {
    let data = state.data.read().map_err(|e| e.to_string())?;
    Ok(data.clone())
}

#[tauri::command]
pub fn save_data(
    app: tauri::AppHandle,
    state: State<'_, AppStateContainer>,
    new_state: AppState,
) -> Result<(), String> {
    let mut data = state.data.write().map_err(|e| e.to_string())?;

    // Update in-memory state
    *data = AppState {
        groups: new_state.groups,
        last_saved_at: Some(chrono::Utc::now().to_rfc3339()),
    };

    // Write to disk
    let path = persistence::data_file_path(&app);
    persistence::save_to_disk(&path, &data)?;

    Ok(())
}
```

Note: For the timestamp, add `chrono` to Cargo.toml dependencies:
```toml
chrono = "0.4"
```

Alternatively, use a simpler approach without chrono — just use the frontend's timestamp or skip it for now. To avoid the extra dependency, replace the chrono line with:

```rust
        last_saved_at: Some(format!("{:?}", std::time::SystemTime::now())),
```

Or better, accept the timestamp from the frontend by keeping `new_state.last_saved_at` logic simple:

```rust
    *data = AppState {
        groups: new_state.groups,
        last_saved_at: new_state.last_saved_at,
    };
```

The frontend already sets the timestamp in the Electron version. Keep it simple — let the frontend set `lastSavedAt` and Rust just persists it.

- [ ] **Step 4: Wire up in main.rs**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod models;
mod persistence;
mod state;

use state::AppStateContainer;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Load saved data from disk on startup
            let path = persistence::data_file_path(&app.handle());
            let initial_state = persistence::load_from_disk(&path);
            app.manage(AppStateContainer::new(initial_state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::load_data,
            commands::save_data,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}
```

- [ ] **Step 5: Verify it compiles and runs**

```bash
cargo tauri dev
```

Expected: App opens. No visible changes in the UI yet, but the Rust backend loads/saves state and the commands are registered.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: persistence layer with load/save commands"
```

---

### Task 4: Frontend Store & Base UI

**Files:**
- Create: `src/stores/appStore.ts`
- Create: `src/views/PlaybackView.vue`
- Create: `src/components/GroupTab.vue`
- Create: `src/components/CompCard.vue`
- Create: `src/components/CompDetails.vue`
- Create: `src/components/TrackControl.vue`
- Create: `src/components/ClipRow.vue`
- Modify: `src/App.vue`

This task migrates the Electron frontend to Tauri. The key changes are:
1. `window.api.*` calls become `invoke()` from `@tauri-apps/api/core`
2. File picker uses `@tauri-apps/plugin-dialog` instead of Electron's dialog
3. Audio-related code is removed (placeholder until Task 7)
4. Types import from `@generated/index` instead of `@shared/types`

- [ ] **Step 1: Create src/stores/appStore.ts**

Migrated from Electron version. Key change: `save()` now calls Tauri `invoke()`.

```typescript
import { defineStore } from 'pinia';
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Group, Comp, Track, Clip, AppState, TrackPlaybackState, CompStatus } from '@generated/index';

function generateId(): string {
  return crypto.randomUUID();
}

export const useAppStore = defineStore('app', () => {
  const groups = ref<Group[]>([]);
  const playbackStates = ref<Map<string, TrackPlaybackState>>(new Map());

  // --- Groups ---

  function addGroup(name: string): Group {
    const group: Group = { id: generateId(), name, comps: [] };
    groups.value.push(group);
    save();
    return group;
  }

  function deleteGroup(groupId: string): void {
    groups.value = groups.value.filter((g) => g.id !== groupId);
    save();
  }

  function renameGroup(groupId: string, name: string): void {
    const group = groups.value.find((g) => g.id === groupId);
    if (group) group.name = name;
    save();
  }

  // --- Comps ---

  function addComp(groupId: string, name: string): Comp | null {
    const group = groups.value.find((g) => g.id === groupId);
    if (!group) return null;
    const comp: Comp = { id: generateId(), name, tracks: [] };
    group.comps.push(comp);
    save();
    return comp;
  }

  function deleteComp(groupId: string, compId: string): void {
    const group = groups.value.find((g) => g.id === groupId);
    if (!group) return;
    group.comps = group.comps.filter((c) => c.id !== compId);
    save();
  }

  // --- Tracks ---

  function addTrack(compId: string, name: string): Track | null {
    const comp = findComp(compId);
    if (!comp) return null;
    const track: Track = {
      id: generateId(),
      name,
      clips: [],
      volume: 1,
      mode: { type: 'sequence', loop: true, transition: { type: 'gapless' } },
      fadeIn: 0,
      fadeOut: 0,
      pan: 0,
    };
    comp.tracks.push(track);
    save();
    return track;
  }

  function deleteTrack(compId: string, trackId: string): void {
    const comp = findComp(compId);
    if (!comp) return;
    comp.tracks = comp.tracks.filter((t) => t.id !== trackId);
    save();
  }

  function updateTrack(compId: string, trackId: string, updates: Partial<Track>): void {
    const comp = findComp(compId);
    if (!comp) return;
    const track = comp.tracks.find((t) => t.id === trackId);
    if (!track) return;
    Object.assign(track, updates);
    save();
  }

  // --- Clips ---

  function addClip(trackId: string, name: string, filePath: string): Clip | null {
    const track = findTrack(trackId);
    if (!track) return null;
    const clip: Clip = {
      id: generateId(),
      name,
      filePath,
      volume: 1,
      trimStart: 0,
      trimEnd: 0,
    };
    track.clips.push(clip);
    save();
    return clip;
  }

  function deleteClip(trackId: string, clipId: string): void {
    const track = findTrack(trackId);
    if (!track) return;
    track.clips = track.clips.filter((c) => c.id !== clipId);
    save();
  }

  // --- Playback state ---

  function setTrackPlaybackState(trackId: string, state: TrackPlaybackState): void {
    playbackStates.value.set(trackId, state);
  }

  function clearTrackPlaybackState(trackId: string): void {
    playbackStates.value.delete(trackId);
  }

  function getCompStatus(compId: string): CompStatus {
    const comp = findComp(compId);
    if (!comp || comp.tracks.length === 0) return 'inactive';

    const playing = comp.tracks.filter((t) => {
      const state = playbackStates.value.get(t.id);
      return state?.isPlaying;
    });

    if (playing.length === 0) return 'inactive';
    if (playing.length === comp.tracks.length) return 'active';
    return 'partial';
  }

  // --- Persistence ---

  function hydrate(state: AppState): void {
    groups.value = state.groups;
  }

  function dehydrate(): AppState {
    return JSON.parse(JSON.stringify({
      groups: groups.value,
      lastSavedAt: new Date().toISOString(),
    }));
  }

  function save(): void {
    invoke('save_data', { newState: dehydrate() });
  }

  // --- Lookups ---

  function findComp(compId: string): Comp | undefined {
    for (const group of groups.value) {
      const comp = group.comps.find((c) => c.id === compId);
      if (comp) return comp;
    }
    return undefined;
  }

  function findTrack(trackId: string): Track | undefined {
    for (const group of groups.value) {
      for (const comp of group.comps) {
        const track = comp.tracks.find((t) => t.id === trackId);
        if (track) return track;
      }
    }
    return undefined;
  }

  return {
    groups,
    playbackStates,
    addGroup,
    deleteGroup,
    renameGroup,
    addComp,
    deleteComp,
    addTrack,
    deleteTrack,
    updateTrack,
    addClip,
    deleteClip,
    setTrackPlaybackState,
    clearTrackPlaybackState,
    getCompStatus,
    hydrate,
    dehydrate,
    findComp,
    findTrack,
    save,
  };
});
```

Note: `CompStatus` is a simple type alias — if ts-rs doesn't generate it (since it was just `type CompStatus = 'active' | 'partial' | 'inactive'` in the Electron version), define it locally in this file:

```typescript
type CompStatus = 'active' | 'partial' | 'inactive';
```

- [ ] **Step 2: Create src/App.vue**

```vue
<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import type { AppState } from '@generated/index';
import PlaybackView from './views/PlaybackView.vue';

const store = useAppStore();
const masterVolume = ref(1);

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);
});

function setMasterVolume(v: number): void {
  masterVolume.value = v;
  // TODO: invoke('set_master_volume', { volume: v }) — wired in Task 7
}

function stopAll(): void {
  // TODO: invoke('stop_all') — wired in Task 7
}
</script>

<template>
  <VApp>
    <VAppBar density="compact" color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
      </VAppBarTitle>

      <template #append>
        <VIcon class="mr-1">mdi-volume-high</VIcon>
        <VSlider
          :model-value="masterVolume"
          @update:model-value="setMasterVolume"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 120px"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="stopAll"
        />
      </template>
    </VAppBar>

    <VMain>
      <PlaybackView />
    </VMain>
  </VApp>
</template>
```

- [ ] **Step 3: Create src/views/PlaybackView.vue**

Copy directly from the Electron version. Only change: import path for types.

```vue
<script setup lang="ts">
import { ref } from 'vue';
import { storeToRefs } from 'pinia';
import { useAppStore } from '../stores/appStore';
import GroupTab from '../components/GroupTab.vue';

const store = useAppStore();
const { groups } = storeToRefs(store);

const activeTab = ref(0);
const showNewGroupDialog = ref(false);
const newGroupName = ref('');
const showNewCompDialog = ref(false);
const newCompName = ref('');
const newCompGroupId = ref('');

function createGroup(): void {
  if (!newGroupName.value.trim()) return;
  store.addGroup(newGroupName.value.trim());
  newGroupName.value = '';
  showNewGroupDialog.value = false;
}

function deleteGroup(groupId: string): void {
  store.deleteGroup(groupId);
  if (activeTab.value >= groups.value.length) {
    activeTab.value = Math.max(0, groups.value.length - 1);
  }
}

function openNewCompDialog(groupId: string): void {
  newCompGroupId.value = groupId;
  newCompName.value = '';
  showNewCompDialog.value = true;
}

function createComp(): void {
  if (!newCompName.value.trim()) return;
  store.addComp(newCompGroupId.value, newCompName.value.trim());
  newCompName.value = '';
  showNewCompDialog.value = false;
}
</script>

<template>
  <VContainer fluid>
    <div class="d-flex align-center mb-4">
      <VTabs v-model="activeTab" density="compact" v-if="groups.length > 0">
        <VTab v-for="(group, i) in groups" :key="group.id" :value="i">
          {{ group.name }}
          <VBtn
            icon="mdi-close"
            variant="text"
            size="x-small"
            class="ml-1"
            @click.stop="deleteGroup(group.id)"
          />
        </VTab>
      </VTabs>
      <VBtn
        icon="mdi-plus"
        variant="tonal"
        size="small"
        class="ml-2"
        @click="showNewGroupDialog = true"
      />
    </div>

    <GroupTab
      v-if="groups.length > 0 && groups[activeTab]"
      :group="groups[activeTab]"
      @add-comp="openNewCompDialog"
    />

    <div v-else class="text-center text-medium-emphasis pa-8">
      No groups yet. Click + to create one.
    </div>

    <VDialog v-model="showNewGroupDialog" max-width="400">
      <VCard>
        <VCardTitle>New Group</VCardTitle>
        <VCardText>
          <VTextField
            v-model="newGroupName"
            label="Group Name"
            variant="outlined"
            autofocus
            @keydown.enter="createGroup"
          />
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showNewGroupDialog = false">Cancel</VBtn>
          <VBtn color="primary" @click="createGroup">Create</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>

    <VDialog v-model="showNewCompDialog" max-width="400">
      <VCard>
        <VCardTitle>New Composition</VCardTitle>
        <VCardText>
          <VTextField
            v-model="newCompName"
            label="Composition Name"
            variant="outlined"
            autofocus
            @keydown.enter="createComp"
          />
        </VCardText>
        <VCardActions>
          <VSpacer />
          <VBtn @click="showNewCompDialog = false">Cancel</VBtn>
          <VBtn color="primary" @click="createComp">Create</VBtn>
        </VCardActions>
      </VCard>
    </VDialog>
  </VContainer>
</template>
```

- [ ] **Step 4: Create src/components/GroupTab.vue**

Identical to Electron version, just update the import path:

```vue
<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Group } from '@generated/index';
import CompCard from './CompCard.vue';
import CompDetails from './CompDetails.vue';

const props = defineProps<{
  group: Group;
}>();

const emit = defineEmits<{
  'add-comp': [groupId: string];
}>();

const selectedCompId = ref<string | null>(null);

const selectedComp = computed(() =>
  props.group.comps.find((c) => c.id === selectedCompId.value) ?? null
);
</script>

<template>
  <div class="d-flex" style="height: calc(100vh - 120px);">
    <div style="width: 300px; min-width: 300px; overflow-y: auto;" class="pr-3">
      <div class="d-flex align-center mb-3">
        <VSpacer />
        <VBtn
          prepend-icon="mdi-plus"
          variant="tonal"
          size="small"
          @click="emit('add-comp', group.id)"
        >
          Add Comp
        </VBtn>
      </div>

      <div class="d-flex flex-wrap ga-2">
        <CompCard
          v-for="comp in group.comps"
          :key="comp.id"
          :comp="comp"
          :selected="comp.id === selectedCompId"
          @select="selectedCompId = $event"
        />
      </div>

      <div v-if="group.comps.length === 0" class="text-medium-emphasis text-center pa-4">
        No comps yet.
      </div>
    </div>

    <VDivider vertical class="mx-2" />
    <div style="flex: 1; overflow-y: auto;" class="pl-3">
      <CompDetails
        v-if="selectedComp"
        :comp="selectedComp"
        :group-id="group.id"
      />
      <div v-else class="text-medium-emphasis text-center pa-8">
        Select a composition to see its tracks.
      </div>
    </div>
  </div>
</template>
```

- [ ] **Step 5: Create src/components/CompCard.vue**

Audio calls are stubbed with TODO comments — wired in Task 7.

```vue
<script setup lang="ts">
import { computed } from 'vue';
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';

const props = defineProps<{
  comp: Comp;
  selected: boolean;
}>();

const emit = defineEmits<{
  select: [compId: string];
}>();

const store = useAppStore();

const status = computed(() => store.getCompStatus(props.comp.id));

const borderStyle = computed(() => {
  switch (status.value) {
    case 'active': return '2px solid #FF6D00';
    case 'partial': return '2px solid #BF360C';
    default: return '2px solid transparent';
  }
});

function toggleComp(): void {
  // TODO: invoke('activate_comp'/'stop_comp') — wired in Task 7
}

function stopComp(): void {
  // TODO: invoke('stop_comp') — wired in Task 7
}
</script>

<template>
  <VCard
    :color="selected ? 'primary' : 'surface-light'"
    variant="flat"
    class="comp-card pa-2 text-center"
    :style="{ width: '130px', cursor: 'pointer', border: borderStyle }"
    @click="emit('select', comp.id)"
  >
    <div class="d-flex justify-center ga-1 mb-2">
      <VBtn
        :icon="status === 'active' ? 'mdi-pause' : 'mdi-play'"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="toggleComp"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="stopComp"
      />
    </div>
    <div class="text-subtitle-2 font-weight-bold text-truncate">{{ comp.name }}</div>
    <div class="text-caption" style="opacity: 0.6;">{{ comp.tracks.length }} tracks</div>
  </VCard>
</template>
```

- [ ] **Step 6: Create src/components/CompDetails.vue**

```vue
<script setup lang="ts">
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import TrackControl from './TrackControl.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
}>();

const store = useAppStore();

function deleteComp(): void {
  // TODO: invoke('stop_comp') — wired in Task 7
  store.deleteComp(props.groupId, props.comp.id);
}

function addTrack(): void {
  // TODO: use Tauri dialog to pick files, then add tracks — wired in Task 7
}
</script>

<template>
  <div>
    <div class="d-flex align-center mb-3">
      <span class="text-h6 font-weight-bold">{{ comp.name }}</span>
      <VSpacer />
      <VBtn
        prepend-icon="mdi-plus"
        variant="tonal"
        size="small"
        class="mr-2"
        @click="addTrack"
      >
        Add Track
      </VBtn>
      <VBtn
        prepend-icon="mdi-delete"
        variant="tonal"
        color="error"
        size="small"
        @click="deleteComp"
      >
        Delete
      </VBtn>
    </div>

    <TrackControl
      v-for="track in comp.tracks"
      :key="track.id"
      :track="track"
      :comp-id="comp.id"
    />

    <div v-if="comp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
      No tracks yet. Click "Add Track" to import audio files.
    </div>
  </div>
</template>
```

- [ ] **Step 7: Create src/components/TrackControl.vue**

Audio calls stubbed — wired in Task 7. The full mode/transition settings UI carries over.

```vue
<script setup lang="ts">
import { ref, computed } from 'vue';
import type { Track, TrackMode, TrackTransition, Clip } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { open } from '@tauri-apps/plugin-dialog';
import draggable from 'vuedraggable';
import ClipRow from './ClipRow.vue';

const props = defineProps<{
  track: Track;
  compId: string;
}>();

const store = useAppStore();

const expanded = ref(false);
const playbackState = computed(() => store.playbackStates.get(props.track.id));
const isPlaying = computed(() => playbackState.value?.isPlaying ?? false);

// --- Track settings ---

const modeOptions = [
  { title: 'Seamless', value: 'seamless' },
  { title: 'Sequence', value: 'sequence' },
  { title: 'Shuffle', value: 'shuffle' },
];

const transitionOptions = [
  { title: 'Gapless', value: 'gapless' },
  { title: 'Gap', value: 'gap' },
  { title: 'Crossfade', value: 'crossfade' },
];

const modeType = computed(() => props.track.mode.type);
const hasTransition = computed(() => modeType.value !== 'seamless');
const transitionType = computed(() =>
  hasTransition.value ? (props.track.mode as any).transition.type : 'gapless'
);

function updateMode(type: string): void {
  let mode: TrackMode;
  if (type === 'seamless') {
    mode = { type: 'seamless', loopPoint: 0 };
  } else {
    const transition = hasTransition.value
      ? (props.track.mode as any).transition
      : { type: 'gapless' as const };
    mode = { type: type as 'sequence' | 'shuffle', loop: true, transition };
  }
  store.updateTrack(props.compId, props.track.id, { mode });
}

function updateTransition(type: string): void {
  let transition: TrackTransition;
  switch (type) {
    case 'gap': transition = { type: 'gap', min: 0, max: 5 }; break;
    case 'crossfade': transition = { type: 'crossfade', duration: 2 }; break;
    default: transition = { type: 'gapless' }; break;
  }
  const mode = props.track.mode as { type: 'sequence' | 'shuffle'; loop: boolean; transition: TrackTransition };
  store.updateTrack(props.compId, props.track.id, {
    mode: { ...mode, transition },
  });
}

function updateField(updates: Partial<Track>): void {
  store.updateTrack(props.compId, props.track.id, updates);
}

function updateLoopPoint(v: number): void {
  updateField({ mode: { type: 'seamless', loopPoint: Number(v) } });
}

function updateLoop(v: boolean): void {
  const mode = props.track.mode as any;
  updateField({ mode: { ...mode, loop: v } });
}

function updateGapMin(v: string): void {
  const mode = props.track.mode as any;
  updateField({ mode: { ...mode, transition: { type: 'gap', min: Number(v), max: mode.transition.max } } });
}

function updateGapMax(v: string): void {
  const mode = props.track.mode as any;
  updateField({ mode: { ...mode, transition: { type: 'gap', min: mode.transition.min, max: Number(v) } } });
}

function updateCrossfade(v: string): void {
  const mode = props.track.mode as any;
  updateField({ mode: { ...mode, transition: { type: 'crossfade', duration: Number(v) } } });
}

function updatePan(v: number): void {
  updateField({ pan: v });
}

function updateFadeIn(v: string): void {
  updateField({ fadeIn: Number(v) });
}

function updateFadeOut(v: string): void {
  updateField({ fadeOut: Number(v) });
}

async function addClips(): Promise<void> {
  const paths = await open({
    multiple: true,
    filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'] }],
  });
  if (!paths) return;
  const pathList = Array.isArray(paths) ? paths : [paths];
  for (const filePath of pathList) {
    const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';
    store.addClip(props.track.id, fileName, filePath);
  }
}

function playTrack(): void {
  // TODO: invoke('play_track') — wired in Task 7
}

function pauseTrack(): void {
  // TODO: invoke('pause_track') — wired in Task 7
}

function stopTrack(): void {
  // TODO: invoke('stop_track') — wired in Task 7
}

function setVolume(v: number): void {
  // TODO: invoke('set_track_volume') — wired in Task 7
  updateField({ volume: v });
}

function deleteTrack(): void {
  // TODO: invoke('stop_track') — wired in Task 7
  store.deleteTrack(props.compId, props.track.id);
}
</script>

<template>
  <VCard class="mb-2" color="surface-light">
    <div class="d-flex align-center ga-2 pa-2">
      <VBtn
        :icon="isPlaying ? 'mdi-pause' : 'mdi-play'"
        :color="isPlaying ? 'secondary' : 'primary'"
        variant="tonal"
        size="small"
        rounded="circle"
        @click="isPlaying ? pauseTrack() : playTrack()"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="small"
        rounded="circle"
        @click="stopTrack"
      />

      <span class="text-subtitle-2 font-weight-bold flex-grow-1">{{ track.name }}</span>

      <span class="text-caption text-medium-emphasis">
        {{ track.clips.length }} clip{{ track.clips.length !== 1 ? 's' : '' }}
        &middot; {{ track.mode.type }}
      </span>

      <VSlider
        :model-value="track.volume"
        @update:model-value="setVolume"
        :min="0"
        :max="1"
        :step="0.01"
        hide-details
        density="compact"
        style="max-width: 120px"
      />

      <VBtn
        :icon="expanded ? 'mdi-chevron-up' : 'mdi-chevron-down'"
        variant="text"
        size="small"
        @click="expanded = !expanded"
      />
    </div>

    <VExpandTransition>
      <div v-if="expanded">
        <VDivider />
        <div class="pa-3">
          <VRow dense class="mb-2">
            <VCol cols="3">
              <VSelect
                :model-value="modeType"
                @update:model-value="updateMode"
                :items="modeOptions"
                label="Mode"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
            <VCol cols="3" v-if="modeType === 'seamless'">
              <VTextField
                :model-value="(track.mode as any).loopPoint"
                @update:model-value="updateLoopPoint"
                label="Loop Point (s)"
                type="number"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
            <VCol cols="3" v-if="hasTransition">
              <VSelect
                :model-value="transitionType"
                @update:model-value="updateTransition"
                :items="transitionOptions"
                label="Transition"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
            <VCol cols="3" v-if="hasTransition">
              <VCheckbox
                :model-value="(track.mode as any).loop"
                @update:model-value="updateLoop"
                label="Loop"
                density="compact"
                hide-details
              />
            </VCol>
          </VRow>

          <VRow dense class="mb-2" v-if="hasTransition && transitionType === 'gap'">
            <VCol cols="3">
              <VTextField
                :model-value="(track.mode as any).transition.min"
                @update:model-value="updateGapMin"
                label="Gap Min (s)"
                type="number"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
            <VCol cols="3">
              <VTextField
                :model-value="(track.mode as any).transition.max"
                @update:model-value="updateGapMax"
                label="Gap Max (s)"
                type="number"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
          </VRow>

          <VRow dense class="mb-2" v-if="hasTransition && transitionType === 'crossfade'">
            <VCol cols="3">
              <VTextField
                :model-value="(track.mode as any).transition.duration"
                @update:model-value="updateCrossfade"
                label="Crossfade (s)"
                type="number"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
          </VRow>

          <VRow dense class="mb-3">
            <VCol cols="3">
              <VSlider
                :model-value="track.pan"
                @update:model-value="updatePan"
                label="Pan"
                :min="-1"
                :max="1"
                :step="0.01"
                density="compact"
                hide-details
              />
            </VCol>
            <VCol cols="3">
              <VTextField
                :model-value="track.fadeIn"
                @update:model-value="updateFadeIn"
                label="Fade In (s)"
                type="number"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
            <VCol cols="3">
              <VTextField
                :model-value="track.fadeOut"
                @update:model-value="updateFadeOut"
                label="Fade Out (s)"
                type="number"
                variant="outlined"
                density="compact"
                hide-details
              />
            </VCol>
          </VRow>

          <div class="d-flex align-center mb-2">
            <span class="text-subtitle-2">Clips</span>
            <VSpacer />
            <VBtn prepend-icon="mdi-plus" variant="tonal" size="x-small" @click="addClips">
              Add Clips
            </VBtn>
          </div>

          <draggable
            :model-value="track.clips"
            @update:model-value="(newClips: Clip[]) => updateField({ clips: newClips })"
            item-key="id"
            handle=".drag-handle"
          >
            <template #item="{ element: clip }">
              <ClipRow :clip="clip" :track-id="track.id" />
            </template>
          </draggable>

          <div v-if="track.clips.length === 0" class="text-caption text-medium-emphasis">
            No clips yet.
          </div>

          <div class="d-flex justify-end mt-3">
            <VBtn
              prepend-icon="mdi-delete"
              variant="tonal"
              color="error"
              size="small"
              @click="deleteTrack"
            >
              Delete Track
            </VBtn>
          </div>
        </div>
      </div>
    </VExpandTransition>
  </VCard>
</template>
```

- [ ] **Step 8: Create src/components/ClipRow.vue**

```vue
<script setup lang="ts">
import type { Clip } from '@generated/index';
import { useAppStore } from '../stores/appStore';

const props = defineProps<{
  clip: Clip;
  trackId: string;
}>();

const store = useAppStore();

function updateVolume(volume: number): void {
  props.clip.volume = volume;
  store.save();
  // TODO: invoke('set_clip_volume') for real-time adjustment — wired in Task 7
}

function remove(): void {
  store.deleteClip(props.trackId, props.clip.id);
}
</script>

<template>
  <div class="d-flex align-center ga-2 py-1">
    <VIcon size="small" class="text-medium-emphasis drag-handle" style="cursor: grab;">mdi-drag</VIcon>
    <span class="text-body-2 flex-grow-1 text-truncate" style="max-width: 200px;">
      {{ clip.name }}
    </span>
    <VSlider
      :model-value="clip.volume"
      @update:model-value="updateVolume"
      :min="0"
      :max="1"
      :step="0.01"
      hide-details
      density="compact"
      style="max-width: 100px"
      prepend-icon="mdi-volume-high"
    />
    <VBtn icon="mdi-close" variant="text" size="x-small" @click="remove" />
  </div>
</template>
```

- [ ] **Step 9: Verify the full UI works with persistence**

```bash
cargo tauri dev
```

Expected: App opens with the dragon's hoard theme. You can create groups, comps, add tracks (file picker opens via Tauri dialog), add clips, adjust settings, and drag-reorder clips. Data persists across app restarts. Audio buttons are present but don't play anything yet.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat: migrate Vue frontend with Tauri persistence"
```

---

### Task 5: Audio Engine (Rust + kira)

**Files:**
- Create: `src-tauri/src/audio/mod.rs`
- Create: `src-tauri/src/audio/engine.rs`
- Modify: `src-tauri/Cargo.toml` (already has kira)

This is the core of the migration. The audio engine handles all playback logic in Rust using kira.

**Important:** kira's API may have changed since this plan was written. Consult the [kira docs](https://docs.rs/kira) and examples during implementation. The architecture below is correct — the exact method signatures may need adjustment.

- [ ] **Step 1: Create src-tauri/src/audio/mod.rs**

```rust
pub mod engine;
```

- [ ] **Step 2: Create src-tauri/src/audio/engine.rs — struct definitions**

```rust
use std::collections::HashMap;
use std::time::Duration;
use kira::{
    manager::{AudioManager, AudioManagerSettings, backend::DefaultBackend},
    sound::static_sound::{StaticSoundData, StaticSoundSettings, StaticSoundHandle},
    track::{TrackBuilder, TrackHandle},
    tween::Tween,
    Volume,
};
use crate::models::{Track, TrackMode, TrackTransition, Clip};

/// State for a single active track in the audio engine.
struct ActiveTrack {
    /// kira mixer sub-track (controls volume + pan for this track)
    mixer_track: TrackHandle,
    /// Handle to the currently playing sound (if any)
    sound_handle: Option<StaticSoundHandle>,
    /// The track config from AppState
    config: Track,
    /// Current position in the clip sequence
    clip_index: usize,
    /// Playback state
    is_playing: bool,
    is_paused: bool,
    current_clip_id: Option<String>,
}

pub struct AudioEngine {
    manager: AudioManager<DefaultBackend>,
    tracks: HashMap<String, ActiveTrack>,
    master_volume: f64,
}
```

- [ ] **Step 3: Implement AudioEngine::new and track setup**

```rust
impl AudioEngine {
    pub fn new() -> Result<Self, String> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| format!("Failed to create audio manager: {}", e))?;

        Ok(Self {
            manager,
            tracks: HashMap::new(),
            master_volume: 1.0,
        })
    }

    /// Set up a track in the engine with its config.
    /// Creates a kira mixer sub-track for volume/pan routing.
    pub fn setup_track(&mut self, track: &Track) -> Result<(), String> {
        // Remove existing track if re-setting up
        if self.tracks.contains_key(&track.id) {
            self.remove_track(&track.id);
        }

        let mixer_track = self.manager
            .add_sub_track(TrackBuilder::new())
            .map_err(|e| format!("Failed to create mixer track: {}", e))?;

        self.tracks.insert(track.id.clone(), ActiveTrack {
            mixer_track,
            sound_handle: None,
            config: track.clone(),
            clip_index: 0,
            is_playing: false,
            is_paused: false,
            current_clip_id: None,
        });

        Ok(())
    }

    pub fn remove_track(&mut self, track_id: &str) {
        if let Some(mut active) = self.tracks.remove(track_id) {
            if let Some(mut handle) = active.sound_handle.take() {
                let _ = handle.stop(Tween::default());
            }
        }
    }

    /// Update the track config (called when frontend saves changes).
    pub fn update_track_config(&mut self, track: &Track) {
        if let Some(active) = self.tracks.get_mut(&track.id) {
            active.config = track.clone();
        }
    }
}
```

- [ ] **Step 4: Implement play, pause, stop**

```rust
impl AudioEngine {
    pub fn play_track(&mut self, track_id: &str) -> Result<(), String> {
        // Ensure the track is set up
        let active = self.tracks.get_mut(track_id)
            .ok_or_else(|| format!("Track {} not found in engine", track_id))?;

        if active.is_paused {
            // Resume from pause
            if let Some(ref mut handle) = active.sound_handle {
                handle.resume(Tween::default())
                    .map_err(|e| format!("Failed to resume: {}", e))?;
            }
            active.is_paused = false;
            active.is_playing = true;
            return Ok(());
        }

        // Start from the beginning
        active.clip_index = 0;
        let clip_id = self.pick_first_clip(track_id)?;
        if let Some(clip_id) = clip_id {
            self.start_clip(track_id, &clip_id)?;
        }

        Ok(())
    }

    pub fn pause_track(&mut self, track_id: &str) -> Result<(), String> {
        let active = self.tracks.get_mut(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        if let Some(ref mut handle) = active.sound_handle {
            handle.pause(Tween::default())
                .map_err(|e| format!("Failed to pause: {}", e))?;
        }
        active.is_playing = false;
        active.is_paused = true;
        Ok(())
    }

    pub fn stop_track(&mut self, track_id: &str) -> Result<(), String> {
        let active = self.tracks.get_mut(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        let fade_out = active.config.fade_out;
        let tween = if fade_out > 0.0 {
            Tween {
                duration: Duration::from_secs_f64(fade_out),
                ..Default::default()
            }
        } else {
            Tween::default()
        };

        if let Some(ref mut handle) = active.sound_handle {
            let _ = handle.stop(tween);
        }
        active.sound_handle = None;
        active.is_playing = false;
        active.is_paused = false;
        active.clip_index = 0;
        active.current_clip_id = None;
        Ok(())
    }

    pub fn stop_all(&mut self) {
        let track_ids: Vec<String> = self.tracks.keys().cloned().collect();
        for id in track_ids {
            let _ = self.stop_track(&id);
        }
    }
}
```

- [ ] **Step 5: Implement start_clip (loads file, applies mode settings, plays)**

```rust
impl AudioEngine {
    /// Load a clip from disk and start playing it on the given track.
    fn start_clip(&mut self, track_id: &str, clip_id: &str) -> Result<(), String> {
        let active = self.tracks.get_mut(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        let clip = active.config.clips.iter()
            .find(|c| c.id == clip_id)
            .ok_or_else(|| format!("Clip {} not found", clip_id))?
            .clone();

        // Stop any currently playing sound on this track
        if let Some(mut old_handle) = active.sound_handle.take() {
            let _ = old_handle.stop(Tween::default());
        }

        // Build sound settings
        let mut settings = StaticSoundSettings::new()
            .output_destination(&active.mixer_track)
            .volume(Volume::Amplitude(
                self.master_volume * active.config.volume * clip.volume
            ));

        // Apply mode-specific settings
        match &active.config.mode {
            TrackMode::Seamless { loop_point } => {
                let end = if clip.trim_end > 0.0 { clip.trim_end } else { f64::MAX };
                settings = settings.loop_region(*loop_point..end);
            }
            _ => {
                // Non-looping for sequence/shuffle — engine advances clips on completion
            }
        }

        // Apply trim start as playback offset
        if clip.trim_start > 0.0 {
            settings = settings.playback_region(clip.trim_start..);
        }

        // Apply trim end
        if clip.trim_end > 0.0 {
            settings = settings.playback_region(clip.trim_start..clip.trim_end);
        }

        // Fade in
        if active.config.fade_in > 0.0 {
            settings = settings.volume(Volume::Amplitude(0.0));
        }

        // Load and play
        let sound_data = StaticSoundData::from_file(&clip.file_path, settings)
            .map_err(|e| format!("Failed to load audio file '{}': {}", clip.file_path, e))?;

        let mut handle = self.manager.play(sound_data)
            .map_err(|e| format!("Failed to play: {}", e))?;

        // Apply fade in via tween
        if active.config.fade_in > 0.0 {
            let target_volume = self.master_volume * active.config.volume * clip.volume;
            let _ = handle.set_volume(
                Volume::Amplitude(target_volume),
                Tween {
                    duration: Duration::from_secs_f64(active.config.fade_in),
                    ..Default::default()
                },
            );
        }

        // Apply pan
        // Note: kira tracks have panning — set on the mixer track, not the sound
        // If kira's TrackBuilder supports panning, set it there during setup_track.
        // Otherwise, use a stereo panner effect or handle via kira's track routing.

        active.sound_handle = Some(handle);
        active.is_playing = true;
        active.is_paused = false;
        active.current_clip_id = Some(clip_id.to_string());

        Ok(())
    }
}
```

**Note on pan:** kira's track/sound panning API may differ. Check kira docs for the correct way to set stereo panning. It may be a setting on the sound, the track, or via a kira effect. The concept is the same — left/right balance for the track.

- [ ] **Step 6: Implement clip selection (first, next, random)**

```rust
impl AudioEngine {
    fn pick_first_clip(&self, track_id: &str) -> Result<Option<String>, String> {
        let active = self.tracks.get(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        if active.config.clips.is_empty() {
            return Ok(None);
        }

        match &active.config.mode {
            TrackMode::Shuffle { .. } => Ok(Some(self.pick_random_clip(track_id)?)),
            _ => Ok(Some(active.config.clips[0].id.clone())),
        }
    }

    fn pick_next_clip(&self, track_id: &str) -> Result<Option<String>, String> {
        let active = self.tracks.get(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        match &active.config.mode {
            TrackMode::Seamless { .. } => Ok(None), // seamless loops internally
            TrackMode::Sequence { should_loop, .. } => {
                let next = active.clip_index + 1;
                if next >= active.config.clips.len() {
                    if *should_loop {
                        Ok(Some(active.config.clips[0].id.clone()))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(Some(active.config.clips[next].id.clone()))
                }
            }
            TrackMode::Shuffle { should_loop, .. } => {
                if !should_loop && active.clip_index >= active.config.clips.len() - 1 {
                    Ok(None)
                } else {
                    Ok(Some(self.pick_random_clip(track_id)?))
                }
            }
        }
    }

    fn pick_random_clip(&self, track_id: &str) -> Result<String, String> {
        let active = self.tracks.get(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        use rand::Rng;
        let mut rng = rand::thread_rng();
        let index = rng.gen_range(0..active.config.clips.len());
        Ok(active.config.clips[index].id.clone())
    }
}
```

Add `rand` to `Cargo.toml`:
```toml
rand = "0.8"
```

- [ ] **Step 7: Implement clip advancement polling**

The engine needs a background task that checks if sounds have finished and advances to the next clip. This runs on a tokio task and checks every 10ms.

```rust
use std::sync::{Arc, Mutex};
use tokio::time;

impl AudioEngine {
    /// Check all active tracks for sounds that have finished.
    /// Returns a list of (track_id, new_clip_id) that need to be started.
    pub fn check_finished_sounds(&mut self) -> Vec<(String, Option<String>)> {
        let mut finished = Vec::new();

        for (track_id, active) in &self.tracks {
            if !active.is_playing || active.is_paused {
                continue;
            }

            // Check if the sound has stopped (finished playing naturally)
            let sound_finished = match &active.sound_handle {
                Some(handle) => {
                    // kira's handle has a state() method — check docs for exact API
                    // This checks if the sound reached its end
                    handle.state() == kira::sound::static_sound::PlaybackState::Stopped
                }
                None => true,
            };

            if sound_finished {
                // Determine next clip
                let next_clip = self.pick_next_clip(track_id).unwrap_or(None);
                finished.push((track_id.clone(), next_clip));
            }
        }

        finished
    }

    /// Advance a track to the next clip with the configured transition.
    pub fn advance_track(&mut self, track_id: &str, next_clip_id: &str) -> Result<(), String> {
        let active = self.tracks.get(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        let transition = match &active.config.mode {
            TrackMode::Seamless { .. } => return Ok(()), // shouldn't happen
            TrackMode::Sequence { transition, .. } => transition.clone(),
            TrackMode::Shuffle { transition, .. } => transition.clone(),
        };

        // Update clip index
        let new_index = active.config.clips.iter()
            .position(|c| c.id == next_clip_id)
            .unwrap_or(0);
        let active = self.tracks.get_mut(track_id).unwrap();
        active.clip_index = new_index;

        match transition {
            TrackTransition::Gapless => {
                self.start_clip(track_id, next_clip_id)?;
            }
            TrackTransition::Gap { min, max } => {
                // For gap transitions, the caller should schedule a delayed start.
                // Store the gap info and handle timing in the polling loop.
                // For v1: use a simple sleep in the polling loop.
                // The start_clip call happens after the gap delay.
                let delay_ms = {
                    use rand::Rng;
                    let mut rng = rand::thread_rng();
                    (rng.gen_range(min..max) * 1000.0) as u64
                };
                // Store pending gap — the polling loop will handle the delay
                let active = self.tracks.get_mut(track_id).unwrap();
                active.is_playing = true; // still "playing" during gap
                active.current_clip_id = None;
                // The polling loop needs to handle the delay.
                // A simpler v1 approach: just block with std::thread::sleep
                // (not ideal but functional for v1)
                std::thread::sleep(Duration::from_millis(delay_ms));
                self.start_clip(track_id, next_clip_id)?;
            }
            TrackTransition::Crossfade { duration } => {
                // Start the new clip immediately — the old clip's handle is still
                // playing (it just "stopped" meaning it reached its end, but with
                // crossfade we want overlap). For true crossfade, we'd start the
                // new clip slightly before the old one ends.
                // v1: start the new clip with a fade-in. The old clip fades out
                // naturally since it's ending.
                let active = self.tracks.get_mut(track_id).unwrap();
                active.config.fade_in = duration; // temporarily set fade_in for the new clip
                self.start_clip(track_id, next_clip_id)?;
                let active = self.tracks.get_mut(track_id).unwrap();
                active.config.fade_in = 0.0; // restore
            }
        }

        Ok(())
    }
}
```

**Note on crossfade:** True crossfade (two clips overlapping) requires playing two sounds simultaneously on the same track. This is achievable in kira by keeping both sound handles alive temporarily. The v1 approach above is a simplification — the old clip ends, the new clip fades in. Refine in a future pass for true overlap.

**Note on gap transitions:** The `std::thread::sleep` approach blocks the audio engine's mutex. A better approach uses a pending state with a timestamp, checked during polling. Refine this when wiring up the polling loop.

- [ ] **Step 8: Implement volume controls**

```rust
impl AudioEngine {
    pub fn set_master_volume(&mut self, volume: f64) {
        self.master_volume = volume.clamp(0.0, 1.0);
        // Update all active sounds to reflect new master volume
        for active in self.tracks.values_mut() {
            if let Some(ref mut handle) = active.sound_handle {
                let clip_volume = active.current_clip_id.as_ref()
                    .and_then(|id| active.config.clips.iter().find(|c| &c.id == id))
                    .map(|c| c.volume)
                    .unwrap_or(1.0);
                let _ = handle.set_volume(
                    Volume::Amplitude(self.master_volume * active.config.volume * clip_volume),
                    Tween::default(),
                );
            }
        }
    }

    pub fn set_track_volume(&mut self, track_id: &str, volume: f64) -> Result<(), String> {
        let active = self.tracks.get_mut(track_id)
            .ok_or_else(|| format!("Track {} not found", track_id))?;

        active.config.volume = volume.clamp(0.0, 1.0);

        if let Some(ref mut handle) = active.sound_handle {
            let clip_volume = active.current_clip_id.as_ref()
                .and_then(|id| active.config.clips.iter().find(|c| &c.id == id))
                .map(|c| c.volume)
                .unwrap_or(1.0);
            let _ = handle.set_volume(
                Volume::Amplitude(self.master_volume * active.config.volume * clip_volume),
                Tween::default(),
            );
        }
        Ok(())
    }

    pub fn set_clip_volume(&mut self, clip_id: &str, volume: f64) -> Result<(), String> {
        // Find which track this clip belongs to and update the volume
        for active in self.tracks.values_mut() {
            if let Some(clip) = active.config.clips.iter_mut().find(|c| c.id == clip_id) {
                clip.volume = volume.clamp(0.0, 1.0);

                // If this clip is currently playing, update the sound volume
                if active.current_clip_id.as_deref() == Some(clip_id) {
                    if let Some(ref mut handle) = active.sound_handle {
                        let _ = handle.set_volume(
                            Volume::Amplitude(self.master_volume * active.config.volume * clip.volume),
                            Tween::default(),
                        );
                    }
                }
                return Ok(());
            }
        }
        Err(format!("Clip {} not found in any active track", clip_id))
    }

    /// Get playback state for all active tracks (used for frontend events).
    pub fn get_all_playback_states(&self) -> Vec<crate::models::TrackPlaybackState> {
        self.tracks.iter().map(|(id, active)| {
            crate::models::TrackPlaybackState {
                track_id: id.clone(),
                is_playing: active.is_playing,
                is_paused: active.is_paused,
                current_clip_id: active.current_clip_id.clone(),
                current_time: 0.0, // TODO: get from kira handle.position()
            }
        }).collect()
    }
}
```

- [ ] **Step 9: Verify it compiles**

```bash
cd src-tauri && cargo build
```

Expected: Compiles. Fix any kira API mismatches by checking docs.rs/kira.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "feat: kira audio engine with playback modes, volume, and fades"
```

---

### Task 6: Audio Tauri Commands & Events

**Files:**
- Modify: `src-tauri/src/commands.rs` (add audio commands)
- Modify: `src-tauri/src/state.rs` (add AudioEngine to state)
- Modify: `src-tauri/src/main.rs` (register audio commands, start polling)

- [ ] **Step 1: Add AudioEngine to state container**

Update `src-tauri/src/state.rs`:

```rust
use std::sync::{Mutex, RwLock};
use crate::audio::engine::AudioEngine;
use crate::models::AppState;

pub struct AppStateContainer {
    pub data: RwLock<AppState>,
    pub audio: Mutex<AudioEngine>,
}

impl AppStateContainer {
    pub fn new(initial: AppState) -> Result<Self, String> {
        Ok(Self {
            data: RwLock::new(initial),
            audio: Mutex::new(AudioEngine::new()?),
        })
    }
}
```

- [ ] **Step 2: Add audio commands to commands.rs**

```rust
use crate::audio::engine::AudioEngine;

#[tauri::command]
pub fn play_track(
    state: State<'_, AppStateContainer>,
    track_id: String,
) -> Result<(), String> {
    let data = state.data.read().map_err(|e| e.to_string())?;
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;

    // Find the track config in app state
    let track = find_track_in_state(&data, &track_id)
        .ok_or_else(|| format!("Track {} not found in state", track_id))?;

    // Ensure track is set up in the engine
    if !audio.has_track(&track_id) {
        audio.setup_track(&track)?;
    }

    audio.play_track(&track_id)
}

#[tauri::command]
pub fn pause_track(
    state: State<'_, AppStateContainer>,
    track_id: String,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.pause_track(&track_id)
}

#[tauri::command]
pub fn stop_track(
    state: State<'_, AppStateContainer>,
    track_id: String,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_track(&track_id)
}

#[tauri::command]
pub fn set_master_volume(
    state: State<'_, AppStateContainer>,
    volume: f64,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.set_master_volume(volume);
    Ok(())
}

#[tauri::command]
pub fn set_track_volume(
    state: State<'_, AppStateContainer>,
    track_id: String,
    volume: f64,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.set_track_volume(&track_id, volume)
}

#[tauri::command]
pub fn set_clip_volume(
    state: State<'_, AppStateContainer>,
    clip_id: String,
    volume: f64,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.set_clip_volume(&clip_id, volume)
}

#[tauri::command]
pub fn activate_comp(
    state: State<'_, AppStateContainer>,
    comp_id: String,
) -> Result<(), String> {
    let data = state.data.read().map_err(|e| e.to_string())?;
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;

    let comp = find_comp_in_state(&data, &comp_id)
        .ok_or_else(|| format!("Comp {} not found", comp_id))?;

    for track in &comp.tracks {
        if !audio.has_track(&track.id) {
            audio.setup_track(track)?;
        }
        audio.play_track(&track.id)?;
    }
    Ok(())
}

#[tauri::command]
pub fn stop_comp(
    state: State<'_, AppStateContainer>,
    comp_id: String,
) -> Result<(), String> {
    let data = state.data.read().map_err(|e| e.to_string())?;
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;

    let comp = find_comp_in_state(&data, &comp_id)
        .ok_or_else(|| format!("Comp {} not found", comp_id))?;

    for track in &comp.tracks {
        let _ = audio.stop_track(&track.id);
    }
    Ok(())
}

#[tauri::command]
pub fn stop_all(
    state: State<'_, AppStateContainer>,
) -> Result<(), String> {
    let mut audio = state.audio.lock().map_err(|e| e.to_string())?;
    audio.stop_all();
    Ok(())
}

// --- Helpers ---

fn find_track_in_state(data: &AppState, track_id: &str) -> Option<crate::models::Track> {
    for group in &data.groups {
        for comp in &group.comps {
            if let Some(track) = comp.tracks.iter().find(|t| t.id == track_id) {
                return Some(track.clone());
            }
        }
    }
    None
}

fn find_comp_in_state(data: &AppState, comp_id: &str) -> Option<crate::models::Comp> {
    for group in &data.groups {
        if let Some(comp) = group.comps.iter().find(|c| c.id == comp_id) {
            return Some(comp.clone());
        }
    }
    None
}
```

- [ ] **Step 3: Add has_track method to AudioEngine**

In `src-tauri/src/audio/engine.rs`, add:

```rust
impl AudioEngine {
    pub fn has_track(&self, track_id: &str) -> bool {
        self.tracks.contains_key(track_id)
    }
}
```

- [ ] **Step 4: Set up playback state polling and events in main.rs**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod commands;
mod models;
mod persistence;
mod state;

use state::AppStateContainer;
use std::sync::Arc;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = persistence::data_file_path(&app.handle());
            let initial_state = persistence::load_from_disk(&path);
            let container = AppStateContainer::new(initial_state)
                .expect("Failed to initialize audio engine");
            app.manage(container);

            // Start background polling for playback state changes
            let app_handle = app.handle().clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(
                    std::time::Duration::from_millis(50)
                );
                loop {
                    interval.tick().await;

                    let state = app_handle.state::<AppStateContainer>();
                    let mut audio = match state.audio.lock() {
                        Ok(guard) => guard,
                        Err(_) => continue,
                    };

                    // Check for finished sounds and advance clips
                    let finished = audio.check_finished_sounds();
                    for (track_id, next_clip) in finished {
                        match next_clip {
                            Some(clip_id) => {
                                let _ = audio.advance_track(&track_id, &clip_id);
                            }
                            None => {
                                // Track finished, no more clips
                                let _ = audio.stop_track(&track_id);
                            }
                        }
                    }

                    // Emit playback state to frontend
                    let states = audio.get_all_playback_states();
                    drop(audio); // release lock before emitting

                    for ps in &states {
                        let _ = app_handle.emit("playback-state-update", ps);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::load_data,
            commands::save_data,
            commands::play_track,
            commands::pause_track,
            commands::stop_track,
            commands::set_master_volume,
            commands::set_track_volume,
            commands::set_clip_volume,
            commands::activate_comp,
            commands::stop_comp,
            commands::stop_all,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LuteDrop");
}
```

- [ ] **Step 5: Verify it compiles**

```bash
cd src-tauri && cargo build
```

Expected: Compiles. All commands registered.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: audio commands and playback state polling"
```

---

### Task 7: Frontend Audio Integration

**Files:**
- Create: `src/composables/useAudio.ts`
- Modify: `src/App.vue`, `src/components/CompCard.vue`, `src/components/CompDetails.vue`, `src/components/TrackControl.vue`, `src/components/ClipRow.vue`

- [ ] **Step 1: Create src/composables/useAudio.ts**

```typescript
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { useAppStore } from '../stores/appStore';
import type { TrackPlaybackState } from '@generated/index';

let listenerRegistered = false;

export function useAudio() {
  const store = useAppStore();

  if (!listenerRegistered) {
    listenerRegistered = true;

    // Listen for playback state updates from Rust
    listen<TrackPlaybackState>('playback-state-update', (event) => {
      store.setTrackPlaybackState(event.payload.trackId, event.payload);
    });
  }

  async function playTrack(trackId: string): Promise<void> {
    await invoke('play_track', { trackId });
  }

  async function pauseTrack(trackId: string): Promise<void> {
    await invoke('pause_track', { trackId });
  }

  async function stopTrack(trackId: string): Promise<void> {
    await invoke('stop_track', { trackId });
  }

  async function setMasterVolume(volume: number): Promise<void> {
    await invoke('set_master_volume', { volume });
  }

  async function setTrackVolume(trackId: string, volume: number): Promise<void> {
    await invoke('set_track_volume', { trackId, volume });
  }

  async function setClipVolume(clipId: string, volume: number): Promise<void> {
    await invoke('set_clip_volume', { clipId, volume });
  }

  async function activateComp(compId: string): Promise<void> {
    await invoke('activate_comp', { compId });
  }

  async function stopComp(compId: string): Promise<void> {
    await invoke('stop_comp', { compId });
  }

  async function stopAll(): Promise<void> {
    await invoke('stop_all');
  }

  async function importTracksToComp(compId: string): Promise<void> {
    const paths = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'ogg', 'flac', 'aac', 'm4a', 'wma'] }],
    });
    if (!paths) return;
    const pathList = Array.isArray(paths) ? paths : [paths];

    for (const filePath of pathList) {
      const fileName = filePath.split(/[/\\]/).pop()?.replace(/\.[^.]+$/, '') || 'Unknown';
      const track = store.addTrack(compId, fileName);
      if (!track) continue;
      store.addClip(track.id, fileName, filePath);
    }
  }

  return {
    playTrack,
    pauseTrack,
    stopTrack,
    setMasterVolume,
    setTrackVolume,
    setClipVolume,
    activateComp,
    stopComp,
    stopAll,
    importTracksToComp,
  };
}
```

- [ ] **Step 2: Wire up App.vue with useAudio**

Replace the TODO stubs in `src/App.vue`:

```vue
<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import type { AppState } from '@generated/index';
import PlaybackView from './views/PlaybackView.vue';

const store = useAppStore();
const audio = useAudio();
const masterVolume = ref(1);

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);
});
</script>

<template>
  <VApp>
    <VAppBar density="compact" color="surface">
      <VAppBarTitle>
        <span class="text-primary font-weight-bold">LuteDrop</span>
      </VAppBarTitle>

      <template #append>
        <VIcon class="mr-1">mdi-volume-high</VIcon>
        <VSlider
          :model-value="masterVolume"
          @update:model-value="(v: number) => { masterVolume = v; audio.setMasterVolume(v); }"
          :min="0"
          :max="1"
          :step="0.01"
          hide-details
          density="compact"
          style="width: 120px"
        />
        <VBtn
          icon="mdi-stop"
          variant="tonal"
          size="small"
          class="ml-2"
          @click="audio.stopAll()"
        />
      </template>
    </VAppBar>

    <VMain>
      <PlaybackView />
    </VMain>
  </VApp>
</template>
```

- [ ] **Step 3: Wire up CompCard.vue**

Replace the TODO stubs:

```vue
<script setup lang="ts">
import { computed } from 'vue';
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';

const props = defineProps<{
  comp: Comp;
  selected: boolean;
}>();

const emit = defineEmits<{
  select: [compId: string];
}>();

const store = useAppStore();
const audio = useAudio();

const status = computed(() => store.getCompStatus(props.comp.id));

const borderStyle = computed(() => {
  switch (status.value) {
    case 'active': return '2px solid #FF6D00';
    case 'partial': return '2px solid #BF360C';
    default: return '2px solid transparent';
  }
});
</script>

<template>
  <VCard
    :color="selected ? 'primary' : 'surface-light'"
    variant="flat"
    class="comp-card pa-2 text-center"
    :style="{ width: '130px', cursor: 'pointer', border: borderStyle }"
    @click="emit('select', comp.id)"
  >
    <div class="d-flex justify-center ga-1 mb-2">
      <VBtn
        :icon="status === 'active' ? 'mdi-pause' : 'mdi-play'"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="status === 'active' ? audio.stopComp(comp.id) : audio.activateComp(comp.id)"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="x-small"
        rounded="circle"
        @click.stop="audio.stopComp(comp.id)"
      />
    </div>
    <div class="text-subtitle-2 font-weight-bold text-truncate">{{ comp.name }}</div>
    <div class="text-caption" style="opacity: 0.6;">{{ comp.tracks.length }} tracks</div>
  </VCard>
</template>
```

- [ ] **Step 4: Wire up CompDetails.vue**

```vue
<script setup lang="ts">
import type { Comp } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import { useAudio } from '../composables/useAudio';
import TrackControl from './TrackControl.vue';

const props = defineProps<{
  comp: Comp;
  groupId: string;
}>();

const store = useAppStore();
const audio = useAudio();

function deleteComp(): void {
  audio.stopComp(props.comp.id);
  store.deleteComp(props.groupId, props.comp.id);
}
</script>

<template>
  <div>
    <div class="d-flex align-center mb-3">
      <span class="text-h6 font-weight-bold">{{ comp.name }}</span>
      <VSpacer />
      <VBtn
        prepend-icon="mdi-plus"
        variant="tonal"
        size="small"
        class="mr-2"
        @click="audio.importTracksToComp(comp.id)"
      >
        Add Track
      </VBtn>
      <VBtn
        prepend-icon="mdi-delete"
        variant="tonal"
        color="error"
        size="small"
        @click="deleteComp"
      >
        Delete
      </VBtn>
    </div>

    <TrackControl
      v-for="track in comp.tracks"
      :key="track.id"
      :track="track"
      :comp-id="comp.id"
    />

    <div v-if="comp.tracks.length === 0" class="text-medium-emphasis text-center pa-8">
      No tracks yet. Click "Add Track" to import audio files.
    </div>
  </div>
</template>
```

- [ ] **Step 5: Wire up TrackControl.vue audio functions**

Replace the TODO stubs in `TrackControl.vue`. The changes are in the `<script>` section — replace the stub functions:

```typescript
// Replace these function bodies:

function playTrack(): void {
  audio.playTrack(props.track.id);
}

function pauseTrack(): void {
  audio.pauseTrack(props.track.id);
}

function stopTrack(): void {
  audio.stopTrack(props.track.id);
}

function setVolume(v: number): void {
  audio.setTrackVolume(props.track.id, v);
  updateField({ volume: v });
}

function deleteTrack(): void {
  audio.stopTrack(props.track.id);
  store.deleteTrack(props.compId, props.track.id);
}
```

Also add the import at the top of the script:
```typescript
import { useAudio } from '../composables/useAudio';
const audio = useAudio();
```

- [ ] **Step 6: Wire up ClipRow.vue with real-time volume**

Update the `updateVolume` function:

```typescript
import { useAudio } from '../composables/useAudio';
const audio = useAudio();

function updateVolume(volume: number): void {
  props.clip.volume = volume;
  store.save();
  audio.setClipVolume(props.clip.id, volume);
}
```

- [ ] **Step 7: Verify audio playback end to end**

```bash
cargo tauri dev
```

Expected: Create a group, comp, and track. Add an audio file. Click play — audio plays through the Rust engine. Volume sliders adjust in real time. Stop/pause work. Multiple tracks in a comp all play when comp is activated. Comp status indicators update based on playback state events.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: wire frontend audio controls to Rust engine"
```

---

### Task 8: Waveform Extraction & Editor

**Files:**
- Create: `src-tauri/src/waveform.rs`
- Create: `src/components/WaveformEditor.vue`
- Modify: `src-tauri/src/commands.rs` (add get_waveform_data)
- Modify: `src-tauri/src/main.rs` (register command, add module)
- Modify: `src/components/TrackControl.vue` (integrate WaveformEditor)

- [ ] **Step 1: Create src-tauri/src/waveform.rs**

Uses `symphonia` to decode audio files and extract amplitude peaks for waveform display.

```rust
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Extract amplitude peaks from an audio file, downsampled to `width` points.
/// Returns a Vec of peak values in the range [0.0, 1.0].
pub fn extract_peaks(file_path: &str, width: u32) -> Result<Vec<f32>, String> {
    let path = Path::new(file_path);
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open file: {}", e))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Failed to probe audio format: {}", e))?;

    let mut format = probed.format;
    let track = format.default_track()
        .ok_or("No default track found")?;
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    // Collect all samples (mono-mixed)
    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(_) => continue,
        };

        let spec = *decoded.spec();
        let num_channels = spec.channels.count();
        let num_frames = decoded.frames();

        let mut sample_buf = SampleBuffer::<f32>::new(
            num_frames as u64,
            *decoded.spec(),
        );
        sample_buf.copy_interleaved_ref(decoded);
        let samples = sample_buf.samples();

        // Mix to mono by averaging channels
        for frame in 0..num_frames {
            let mut sum = 0.0f32;
            for ch in 0..num_channels {
                sum += samples[frame * num_channels + ch].abs();
            }
            all_samples.push(sum / num_channels as f32);
        }
    }

    if all_samples.is_empty() {
        return Ok(vec![0.0; width as usize]);
    }

    // Downsample to requested width by taking max amplitude in each bucket
    let samples_per_pixel = all_samples.len() as f64 / width as f64;
    let mut peaks = Vec::with_capacity(width as usize);

    // Find global max for normalization
    let global_max = all_samples.iter().cloned().fold(0.0f32, f32::max);
    let normalize = if global_max > 0.0 { global_max } else { 1.0 };

    for i in 0..width {
        let start = (i as f64 * samples_per_pixel) as usize;
        let end = ((i as f64 + 1.0) * samples_per_pixel) as usize;
        let end = end.min(all_samples.len());

        let peak = all_samples[start..end]
            .iter()
            .cloned()
            .fold(0.0f32, f32::max);

        peaks.push(peak / normalize);
    }

    Ok(peaks)
}
```

- [ ] **Step 2: Add waveform command to commands.rs**

```rust
use crate::waveform;

#[tauri::command]
pub fn get_waveform_data(file_path: String, width: u32) -> Result<Vec<f32>, String> {
    waveform::extract_peaks(&file_path, width)
}
```

- [ ] **Step 3: Register in main.rs**

Add `mod waveform;` to the module declarations and add `commands::get_waveform_data` to the `generate_handler!` macro.

- [ ] **Step 4: Create src/components/WaveformEditor.vue**

```vue
<script setup lang="ts">
import { ref, onMounted, watch, nextTick } from 'vue';
import { invoke } from '@tauri-apps/api/core';

const props = defineProps<{
  filePath: string;
  trimStart: number;
  trimEnd: number;
  loopPoint?: number;
  showLoopPoint: boolean;
}>();

const emit = defineEmits<{
  'update:trimStart': [value: number];
  'update:trimEnd': [value: number];
  'update:loopPoint': [value: number];
}>();

const canvas = ref<HTMLCanvasElement | null>(null);
const peaks = ref<number[]>([]);
const duration = ref(0);
const dragging = ref<'trimStart' | 'trimEnd' | 'loopPoint' | null>(null);

const CANVAS_WIDTH = 600;
const CANVAS_HEIGHT = 80;

onMounted(async () => {
  await loadWaveform();
});

watch(() => props.filePath, async () => {
  await loadWaveform();
});

async function loadWaveform(): Promise<void> {
  if (!props.filePath) return;
  try {
    peaks.value = await invoke<number[]>('get_waveform_data', {
      filePath: props.filePath,
      width: CANVAS_WIDTH,
    });
    await nextTick();
    draw();
  } catch (e) {
    console.error('Failed to load waveform:', e);
  }
}

function draw(): void {
  const ctx = canvas.value?.getContext('2d');
  if (!ctx || peaks.value.length === 0) return;

  ctx.clearRect(0, 0, CANVAS_WIDTH, CANVAS_HEIGHT);

  // Draw waveform
  const mid = CANVAS_HEIGHT / 2;
  ctx.fillStyle = '#F0A830'; // primary gold

  for (let i = 0; i < peaks.value.length; i++) {
    const h = peaks.value[i] * mid;
    ctx.fillRect(i, mid - h, 1, h * 2);
  }

  // Draw trim regions (darkened overlay)
  const trimStartX = timeToX(props.trimStart);
  const trimEndX = props.trimEnd > 0 ? timeToX(props.trimEnd) : CANVAS_WIDTH;

  ctx.fillStyle = 'rgba(0, 0, 0, 0.6)';
  if (trimStartX > 0) {
    ctx.fillRect(0, 0, trimStartX, CANVAS_HEIGHT);
  }
  if (trimEndX < CANVAS_WIDTH) {
    ctx.fillRect(trimEndX, 0, CANVAS_WIDTH - trimEndX, CANVAS_HEIGHT);
  }

  // Draw trim markers
  ctx.strokeStyle = '#FF6D00'; // secondary orange
  ctx.lineWidth = 2;
  if (trimStartX > 0) {
    ctx.beginPath();
    ctx.moveTo(trimStartX, 0);
    ctx.lineTo(trimStartX, CANVAS_HEIGHT);
    ctx.stroke();
  }
  if (props.trimEnd > 0) {
    ctx.beginPath();
    ctx.moveTo(trimEndX, 0);
    ctx.lineTo(trimEndX, CANVAS_HEIGHT);
    ctx.stroke();
  }

  // Draw loop point marker
  if (props.showLoopPoint && props.loopPoint !== undefined) {
    const loopX = timeToX(props.loopPoint);
    ctx.strokeStyle = '#4CAF50'; // green
    ctx.lineWidth = 2;
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    ctx.moveTo(loopX, 0);
    ctx.lineTo(loopX, CANVAS_HEIGHT);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}

function timeToX(time: number): number {
  if (duration.value <= 0) return 0;
  return Math.round((time / duration.value) * CANVAS_WIDTH);
}

function xToTime(x: number): number {
  return (x / CANVAS_WIDTH) * duration.value;
}

function onMouseDown(e: MouseEvent): void {
  const rect = canvas.value!.getBoundingClientRect();
  const x = e.clientX - rect.left;
  const time = xToTime(x);

  // Determine which marker is closest
  const trimStartX = timeToX(props.trimStart);
  const trimEndX = props.trimEnd > 0 ? timeToX(props.trimEnd) : CANVAS_WIDTH;
  const loopX = props.loopPoint !== undefined ? timeToX(props.loopPoint) : -1;

  const threshold = 10; // pixels
  const distances = [
    { type: 'trimStart' as const, dist: Math.abs(x - trimStartX) },
    { type: 'trimEnd' as const, dist: Math.abs(x - trimEndX) },
  ];
  if (props.showLoopPoint && loopX >= 0) {
    distances.push({ type: 'loopPoint' as const, dist: Math.abs(x - loopX) });
  }

  distances.sort((a, b) => a.dist - b.dist);
  if (distances[0].dist < threshold) {
    dragging.value = distances[0].type;
  } else {
    // Click to set the nearest marker
    dragging.value = distances[0].type;
    emitMarkerUpdate(distances[0].type, time);
  }
}

function onMouseMove(e: MouseEvent): void {
  if (!dragging.value) return;
  const rect = canvas.value!.getBoundingClientRect();
  const x = Math.max(0, Math.min(CANVAS_WIDTH, e.clientX - rect.left));
  const time = xToTime(x);
  emitMarkerUpdate(dragging.value, time);
}

function onMouseUp(): void {
  dragging.value = null;
}

function emitMarkerUpdate(marker: 'trimStart' | 'trimEnd' | 'loopPoint', time: number): void {
  const rounded = Math.round(time * 100) / 100;
  switch (marker) {
    case 'trimStart': emit('update:trimStart', rounded); break;
    case 'trimEnd': emit('update:trimEnd', rounded); break;
    case 'loopPoint': emit('update:loopPoint', rounded); break;
  }
  nextTick(() => draw());
}

// Redraw when props change
watch([() => props.trimStart, () => props.trimEnd, () => props.loopPoint], () => {
  draw();
});
</script>

<template>
  <div class="waveform-editor" style="position: relative;">
    <canvas
      ref="canvas"
      :width="CANVAS_WIDTH"
      :height="CANVAS_HEIGHT"
      style="width: 100%; height: 80px; cursor: pointer; border-radius: 8px; background: #0A0A0C;"
      @mousedown="onMouseDown"
      @mousemove="onMouseMove"
      @mouseup="onMouseUp"
      @mouseleave="onMouseUp"
    />
    <div class="d-flex ga-2 mt-1 text-caption text-medium-emphasis">
      <span>Trim: {{ props.trimStart.toFixed(2) }}s - {{ props.trimEnd > 0 ? props.trimEnd.toFixed(2) + 's' : 'end' }}</span>
      <span v-if="showLoopPoint">Loop: {{ props.loopPoint?.toFixed(2) ?? 0 }}s</span>
    </div>
  </div>
</template>
```

**Note:** The `duration` ref needs to be set. Either pass it as a prop from the parent, or extract it from the peaks (peaks.length maps to duration). A simpler approach: add a `get_audio_duration` command in Rust. For v1, the waveform component can estimate duration from the file or receive it as a prop. The exact time-to-pixel mapping can be refined.

- [ ] **Step 5: Integrate WaveformEditor into TrackControl.vue**

Add the WaveformEditor inside the expanded section, after the mode settings and before the clips list:

```vue
<!-- Inside the expanded <div>, after the Pan/Fade row, before the Clips section -->
<div v-if="track.clips.length > 0" class="mb-3">
  <WaveformEditor
    v-for="clip in track.clips"
    :key="clip.id"
    :file-path="clip.filePath"
    :trim-start="clip.trimStart"
    :trim-end="clip.trimEnd"
    :loop-point="modeType === 'seamless' ? (track.mode as any).loopPoint : undefined"
    :show-loop-point="modeType === 'seamless'"
    @update:trim-start="(v) => { clip.trimStart = v; store.save(); }"
    @update:trim-end="(v) => { clip.trimEnd = v; store.save(); }"
    @update:loop-point="(v) => updateLoopPoint(v)"
    class="mb-2"
  />
</div>
```

Add the import:
```typescript
import WaveformEditor from './WaveformEditor.vue';
```

- [ ] **Step 6: Verify waveforms display and markers are draggable**

```bash
cargo tauri dev
```

Expected: When you expand a track that has clips, a waveform visualization appears for each clip. You can click and drag to set trim start, trim end, and loop point (for seamless mode) markers. The darkened regions show the trimmed areas. Values update in real time and persist.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: waveform editor with draggable trim and loop markers"
```

---

## Post-Implementation Checklist

After all tasks are complete:

- [ ] Verify the app starts and loads saved data from the Electron version's JSON file (format should be compatible since types match)
- [ ] Create groups, comps, tracks, and clips — verify persistence across restarts
- [ ] Play audio files in all three modes (seamless, sequence, shuffle)
- [ ] Test volume controls (master, track, clip) during playback
- [ ] Test fade in/out on play/stop
- [ ] Test gap and crossfade transitions between clips
- [ ] Test waveform display and marker dragging
- [ ] Test drag reordering of clips
- [ ] Test the file dialog for adding clips
- [ ] Verify no audio on stop/pause, correct resume from pause
