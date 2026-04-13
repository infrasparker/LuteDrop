# Navigation, Library Waveform Editor, and Trim Points Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add vue-router with nav rail, restructure into Soundstage/Library routes with persistent transport bar, add waveform editor dialog to Library with trim+loop editing, and make the audio engine respect trim points.

**Architecture:** App.vue becomes a shell (nav rail + route-specific app bar + router-view + bottom transport bar). Soundstage and Library are separate routes. The WaveformEditor gains trim markers with drag constraints. Trim points flow from library entries through the Clip model to the audio engine, which uses them for start position, end boundary, and crossfade timing. Data clamping ensures loop points always fit within trim bounds.

**Tech Stack:** vue-router 4, Vue 3, Vuetify 4, Rust (rusqlite, kira)

**Spec:** `docs/superpowers/specs/2026-04-12-navigation-library-trim-design.md`

---

## File Structure

### New files

| File | Responsibility |
|---|---|
| `src/router.ts` | Vue Router setup with Soundstage and Library routes |
| `src/views/SoundstageView.vue` | Main playback view (groups, comps, tracks) — absorbs PlaybackView + group management from App.vue |
| `src/components/TransportBar.vue` | Persistent bottom bar: master volume, stop/pause/resume all, track count |
| `src/components/WaveformEditorDialog.vue` | Full-width dialog for editing library entry audio properties (name, tags, gain, trim, loop) |

### Modified files

| File | What changes |
|---|---|
| `package.json` | Add `vue-router` dependency |
| `src/main.ts` | Register router |
| `src/App.vue` | Slim shell: nav rail + per-route app bar + router-view + TransportBar |
| `src/views/LibraryView.vue` | Add waveform dialog trigger, move search/import to inline header |
| `src/components/TrackSettings.vue` | Remove waveform editor and loop point numeric inputs for seamless mode |
| `src/components/WaveformEditor.vue` | Add trim markers, dim outside trim region, enforce drag constraints between all 4 markers |
| `src/composables/useTrackUpdates.ts` | Remove loop point update methods (no longer called from TrackSettings) |
| `src-tauri/src/models.rs` | Add `trim_start: Option<f64>`, `trim_end: Option<f64>` to Clip struct |
| `src-tauri/src/database/mod.rs` | Resolve trim points in `load_clips` |
| `src-tauri/src/database/library.rs` | Add clamping logic to `update_library_entry_trim` and `update_library_entry_loop` |
| `src-tauri/src/audio/engine.rs` | Respect trim_start/trim_end in playback, crossfade timing, and duration |
| `generated/` | Regenerated types |

### Deleted files

| File | Why |
|---|---|
| `src/views/PlaybackView.vue` | Replaced by `SoundstageView.vue` |

---

## Task 1: Add vue-router and create route shell

**Files:**
- Modify: `package.json`
- Create: `src/router.ts`
- Modify: `src/main.ts`

- [ ] **Step 1: Install vue-router**

Run: `npm install vue-router@4`

- [ ] **Step 2: Create router.ts**

Create `src/router.ts`:

```typescript
import { createRouter, createWebHistory } from 'vue-router';

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      name: 'soundstage',
      component: () => import('./views/SoundstageView.vue'),
    },
    {
      path: '/library',
      name: 'library',
      component: () => import('./views/LibraryView.vue'),
    },
  ],
});

export default router;
```

- [ ] **Step 3: Register router in main.ts**

Replace `src/main.ts`:

```typescript
import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './App.vue';
import { vuetify } from './plugins/vuetify';
import router from './router';

const app = createApp(App);
app.use(createPinia());
app.use(vuetify);
app.use(router);
app.mount('#app');
```

- [ ] **Step 4: Commit**

```bash
git add package.json package-lock.json src/router.ts src/main.ts
git commit -m "feat: add vue-router with soundstage and library routes"
```

---

## Task 2: Create SoundstageView (extract from App.vue)

**Files:**
- Create: `src/views/SoundstageView.vue`
- Delete: `src/views/PlaybackView.vue`

- [ ] **Step 1: Create SoundstageView.vue**

This combines PlaybackView's content with the group management logic currently in App.vue (group tabs, new group dialog, context menu, rename/delete dialogs, new comp dialog). It receives no props — it reads from the store directly.

Create `src/views/SoundstageView.vue`:

```vue
<script setup lang="ts">
import { ref } from 'vue';
import { storeToRefs } from 'pinia';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../stores/appStore';
import { useContextMenuState } from '../composables/useContextMenu';
import type { Group } from '@generated/index';
import GroupTab from '../components/GroupTab.vue';

const store = useAppStore();
const { groups } = storeToRefs(store);

const activeTab = ref(0);
const serverUrl = ref('');

// Server info
(async () => {
  try {
    const info = await invoke<{ ip: string; port: number }>('get_server_info');
    serverUrl.value = `http://${info.ip}:${info.port}`;
  } catch (e) {
    console.warn('Could not get server info:', e);
  }
})();

// New group dialog
const showNewGroupDialog = ref(false);
const newGroupName = ref('');

async function createGroup(): Promise<void> {
  if (!newGroupName.value.trim()) return;
  await store.command('add_group', { name: newGroupName.value.trim() });
  newGroupName.value = '';
  showNewGroupDialog.value = false;
  activeTab.value = groups.value.length - 1;
}

// Group context menu
const { menuVisible: contextMenu, menuX: contextMenuX, menuY: contextMenuY, openMenu } = useContextMenuState();
const contextGroupId = ref('');
const contextGroupName = ref('');

function openContextMenu(event: MouseEvent, group: Group): void {
  contextGroupId.value = group.id;
  contextGroupName.value = group.name;
  openMenu(event);
}

// Rename
const showRenameDialog = ref(false);
const renameGroupName = ref('');

function openRenameDialog(): void {
  renameGroupName.value = contextGroupName.value;
  showRenameDialog.value = true;
}

async function renameGroup(): Promise<void> {
  if (!renameGroupName.value.trim()) return;
  await store.command('rename_group', { groupId: contextGroupId.value, name: renameGroupName.value.trim() });
  renameGroupName.value = '';
  showRenameDialog.value = false;
}

// Delete
const showDeleteConfirm = ref(false);

function confirmOrDeleteGroup(): void {
  const group = groups.value.find((g) => g.id === contextGroupId.value);
  if (group && group.comps.length === 0) {
    deleteGroup();
  } else {
    showDeleteConfirm.value = true;
  }
}

async function deleteGroup(): Promise<void> {
  showDeleteConfirm.value = false;
  await store.command('delete_group', { groupId: contextGroupId.value });
  if (activeTab.value >= groups.value.length) {
    activeTab.value = Math.max(0, groups.value.length - 1);
  }
}

// New comp dialog
const showNewCompDialog = ref(false);
const newCompName = ref('');
const newCompGroupId = ref('');

function openNewCompDialog(groupId: string): void {
  newCompGroupId.value = groupId;
  newCompName.value = '';
  showNewCompDialog.value = true;
}

async function createComp(): Promise<void> {
  if (!newCompName.value.trim()) return;
  await store.command('add_comp', { groupId: newCompGroupId.value, name: newCompName.value.trim() });
  newCompName.value = '';
  showNewCompDialog.value = false;
}

function isGroupActive(group: Group): boolean {
  return group.comps.some((comp) =>
    comp.tracks.some((track) => {
      const s = store.playbackStates.get(track.id);
      return s?.isPlaying || s?.isPaused;
    })
  );
}
</script>

<template>
  <div class="d-flex flex-column" style="height: 100%;">
    <!-- Soundstage app bar content: group tabs -->
    <div class="px-2 pt-1" style="background: rgb(var(--v-theme-surface)); border-bottom: 1px solid rgba(255,255,255,0.05);">
      <div class="d-flex align-center mb-1">
        <span v-if="serverUrl" class="text-caption text-medium-emphasis">
          Remote: {{ serverUrl }}
        </span>
      </div>
      <VTabs v-model="activeTab" color="primary">
        <VTab
          v-for="(group, i) in groups"
          :key="group.id"
          :value="i"
          @contextmenu.prevent="openContextMenu($event, group)"
          class="tab-truncate"
          :style="isGroupActive(group) ? { color: '#FF6D00' } : {}"
        >
          {{ group.name }}
        </VTab>
        <VTab :value="activeTab" @click.stop.prevent="showNewGroupDialog = true" style="min-width: 0;" class="px-2">
          <VIcon>mdi-plus</VIcon>
        </VTab>
      </VTabs>
    </div>

    <!-- Main content -->
    <div class="flex-grow-1" style="overflow: auto;">
      <VContainer fluid class="d-flex flex-column" style="height: 100%;">
        <GroupTab
          v-if="groups.length > 0 && groups[activeTab]"
          :group="groups[activeTab]"
          @add-comp="openNewCompDialog"
          class="flex-grow-1"
          style="overflow: hidden;"
        />
        <div v-else class="text-center text-medium-emphasis pa-8">
          No groups yet. Click + to create one.
        </div>
      </VContainer>
    </div>

    <!-- Dialogs -->
    <VDialog v-model="showNewGroupDialog" max-width="400">
      <VCard title="New Group">
        <VCardText>
          <VTextField v-model="newGroupName" label="Group Name" variant="outlined" autofocus @keydown.enter="createGroup" />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showNewGroupDialog = false" />
          <VBtn text="Create" color="primary" variant="tonal" @click="createGroup" />
        </VCardActions>
      </VCard>
    </VDialog>

    <VDialog v-model="showNewCompDialog" max-width="400">
      <VCard title="New Composition">
        <VCardText>
          <VTextField v-model="newCompName" label="Composition Name" variant="outlined" autofocus @keydown.enter="createComp" />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showNewCompDialog = false" />
          <VBtn text="Create" color="primary" variant="tonal" @click="createComp" />
        </VCardActions>
      </VCard>
    </VDialog>

    <VMenu v-model="contextMenu" :target="[contextMenuX, contextMenuY]" close-on-content-click>
      <VList density="compact">
        <VListItem @click="openRenameDialog">
          <template #prepend><VIcon>mdi-pencil</VIcon></template>
          <VListItemTitle>Rename</VListItemTitle>
        </VListItem>
        <VListItem @click="confirmOrDeleteGroup" class="text-error">
          <template #prepend><VIcon color="error">mdi-delete</VIcon></template>
          <VListItemTitle>Delete</VListItemTitle>
        </VListItem>
      </VList>
    </VMenu>

    <VDialog v-model="showRenameDialog" max-width="400">
      <VCard title="Rename Group">
        <VCardText>
          <VTextField v-model="renameGroupName" label="Group Name" variant="outlined" autofocus @keydown.enter="renameGroup" />
        </VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showRenameDialog = false" />
          <VBtn text="Rename" color="primary" variant="tonal" @click="renameGroup" />
        </VCardActions>
      </VCard>
    </VDialog>

    <VDialog v-model="showDeleteConfirm" max-width="450">
      <VCard title="Delete Group">
        <VCardText>Are you sure you want to delete group "{{ contextGroupName }}"? This will remove all compositions and tracks in it.</VCardText>
        <VDivider />
        <VCardActions>
          <VSpacer />
          <VBtn text="Cancel" variant="plain" @click="showDeleteConfirm = false" />
          <VBtn text="Delete" color="error" variant="tonal" @click="deleteGroup" />
        </VCardActions>
      </VCard>
    </VDialog>
  </div>
</template>

<style scoped>
.tab-truncate {
  max-width: 150px !important;
}
.tab-truncate :deep(.v-btn__content) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  display: block !important;
}
</style>
```

- [ ] **Step 2: Delete PlaybackView.vue**

Delete `src/views/PlaybackView.vue`.

- [ ] **Step 3: Commit**

```bash
git add src/views/SoundstageView.vue
git rm src/views/PlaybackView.vue
git commit -m "feat: create SoundstageView, replace PlaybackView"
```

---

## Task 3: Create TransportBar and rewrite App.vue as shell

**Files:**
- Create: `src/components/TransportBar.vue`
- Modify: `src/App.vue`

- [ ] **Step 1: Create TransportBar.vue**

```vue
<script setup lang="ts">
import { ref, computed } from 'vue';
import { useAudio } from '../composables/useAudio';
import { useAppStore } from '../stores/appStore';

const audio = useAudio();
const store = useAppStore();
const paused = ref(false);

const playingCount = computed(() => {
  let count = 0;
  for (const [, state] of store.playbackStates) {
    if (state.isPlaying) count++;
  }
  return count;
});

function setMasterVolume(v: number): void {
  audio.masterVolume.value = v;
  audio.setMasterVolume(v);
}

function stopAll(): void {
  paused.value = false;
  audio.stopAll();
}

function togglePause(): void {
  if (paused.value) {
    audio.resumeAll();
    paused.value = false;
  } else {
    audio.pauseAll();
    paused.value = true;
  }
}
</script>

<template>
  <VFooter app class="px-4" style="background: rgb(var(--v-theme-surface)); border-top: 1px solid rgba(255,255,255,0.08);">
    <div class="d-flex align-center w-100 ga-3">
      <VBtn
        :icon="paused ? 'mdi-play' : 'mdi-pause'"
        variant="tonal"
        size="small"
        @click="togglePause"
      />
      <VBtn
        icon="mdi-stop"
        variant="tonal"
        size="small"
        @click="stopAll"
      />
      <span v-if="playingCount > 0" class="text-caption text-primary">
        {{ playingCount }} track{{ playingCount === 1 ? '' : 's' }} playing
      </span>
      <VSpacer />
      <VIcon size="small" class="text-medium-emphasis">mdi-volume-high</VIcon>
      <VSlider
        :model-value="audio.masterVolume.value"
        @update:model-value="setMasterVolume"
        :min="0"
        :max="1"
        :step="0.01"
        hide-details
        density="compact"
        style="max-width: 200px;"
      />
    </div>
  </VFooter>
</template>
```

- [ ] **Step 2: Rewrite App.vue as shell**

Replace `src/App.vue` entirely. The shell has: nav rail (left), router-view (center), transport bar (bottom), snackbar, and the initial data load in onMounted.

```vue
<script setup lang="ts">
import { onMounted } from 'vue';
import { useRouter } from 'vue-router';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from './stores/appStore';
import { useAudio } from './composables/useAudio';
import { useToast } from './composables/useToast';
import type { AppState } from '@generated/index';
import TransportBar from './components/TransportBar.vue';

const store = useAppStore();
const router = useRouter();
const audio = useAudio();
const toast = useToast();

onMounted(async () => {
  const state = await invoke<AppState>('load_data');
  store.hydrate(state);
  await store.checkMissingFiles();

  if (store.missingEntryIds.size > 0) {
    toast.showError(`${store.missingEntryIds.size} audio file(s) not found. Check the Library view.`);
  }
});

const navItems = [
  { title: 'Soundstage', icon: 'mdi-speaker-group', to: '/' },
  { title: 'Library', icon: 'mdi-bookshelf', to: '/library' },
];
</script>

<template>
  <VApp @contextmenu.prevent>
    <VNavigationDrawer rail expand-on-hover permanent color="surface">
      <VList nav density="compact" class="mt-2">
        <VListItem
          v-for="item in navItems"
          :key="item.to"
          :to="item.to"
          :prepend-icon="item.icon"
          :title="item.title"
          :active="router.currentRoute.value.path === item.to"
          color="primary"
        />
      </VList>
    </VNavigationDrawer>

    <VMain>
      <router-view />
    </VMain>

    <TransportBar />

    <VSnackbar v-model="toast.visible.value" :color="toast.color.value" :timeout="3000">
      {{ toast.message.value }}
    </VSnackbar>
  </VApp>
</template>
```

- [ ] **Step 3: Verify TypeScript compilation**

Run: `npx vue-tsc --noEmit`

- [ ] **Step 4: Run dev server and verify both routes work**

Run: `npm run tauri dev`

Test:
- Nav rail shows two icons, expands on hover
- Clicking Soundstage shows group tabs, comps, tracks
- Clicking Library shows library entries, search, import
- Transport bar at bottom with master volume, pause/stop, track count
- Playback works from Soundstage, transport bar controls work from Library

- [ ] **Step 5: Commit**

```bash
git add src/components/TransportBar.vue src/App.vue
git commit -m "feat: app shell with nav rail, per-route views, persistent transport bar"
```

---

## Task 4: Data clamping for trim and loop points

**Files:**
- Modify: `src-tauri/src/database/library.rs`

- [ ] **Step 1: Write tests for clamping behavior**

Add to the test module in `src-tauri/src/database/library.rs`:

```rust
#[test]
fn trim_clamps_loop_points() {
    let conn = open_memory_db();
    let id = insert_library_entry(&conn, "/a.mp3", Some("a"), "A", None, None, None).unwrap();

    // Set loop points first
    update_library_entry_loop(&conn, &id, Some(10.0), Some(100.0)).unwrap();

    // Set trim that's tighter than loop — loop should be clamped
    update_library_entry_trim(&conn, &id, Some(20.0), Some(80.0)).unwrap();

    let (ls, le): (Option<f64>, Option<f64>) = conn.query_row(
        "SELECT loop_start, loop_end FROM library_entries WHERE id = ?1",
        params![id], |r| Ok((r.get(0)?, r.get(1)?))
    ).unwrap();
    assert!((ls.unwrap() - 20.0).abs() < 0.001); // clamped to trim_start
    assert!((le.unwrap() - 80.0).abs() < 0.001); // clamped to trim_end
}

#[test]
fn trim_clears_loop_when_invalid() {
    let conn = open_memory_db();
    let id = insert_library_entry(&conn, "/a.mp3", Some("a"), "A", None, None, None).unwrap();

    update_library_entry_loop(&conn, &id, Some(40.0), Some(60.0)).unwrap();

    // Trim range is entirely after the loop range — loop becomes invalid
    update_library_entry_trim(&conn, &id, Some(70.0), Some(100.0)).unwrap();

    let (ls, le): (Option<f64>, Option<f64>) = conn.query_row(
        "SELECT loop_start, loop_end FROM library_entries WHERE id = ?1",
        params![id], |r| Ok((r.get(0)?, r.get(1)?))
    ).unwrap();
    assert!(ls.is_none()); // cleared
    assert!(le.is_none()); // cleared
}

#[test]
fn loop_clamped_to_trim_bounds() {
    let conn = open_memory_db();
    let id = insert_library_entry(&conn, "/a.mp3", Some("a"), "A", None, None, None).unwrap();

    // Set trim first
    update_library_entry_trim(&conn, &id, Some(10.0), Some(50.0)).unwrap();

    // Try to set loop outside trim — should be clamped
    update_library_entry_loop(&conn, &id, Some(5.0), Some(60.0)).unwrap();

    let (ls, le): (Option<f64>, Option<f64>) = conn.query_row(
        "SELECT loop_start, loop_end FROM library_entries WHERE id = ?1",
        params![id], |r| Ok((r.get(0)?, r.get(1)?))
    ).unwrap();
    assert!((ls.unwrap() - 10.0).abs() < 0.001); // clamped to trim_start
    assert!((le.unwrap() - 50.0).abs() < 0.001); // clamped to trim_end
}

#[test]
fn trim_start_must_be_less_than_end() {
    let conn = open_memory_db();
    let id = insert_library_entry(&conn, "/a.mp3", Some("a"), "A", None, None, None).unwrap();
    let result = update_library_entry_trim(&conn, &id, Some(50.0), Some(10.0));
    assert!(result.is_err());
}

#[test]
fn loop_start_must_be_less_than_end() {
    let conn = open_memory_db();
    let id = insert_library_entry(&conn, "/a.mp3", Some("a"), "A", None, None, None).unwrap();
    let result = update_library_entry_loop(&conn, &id, Some(50.0), Some(10.0));
    assert!(result.is_err());
}
```

- [ ] **Step 2: Update update_library_entry_trim with clamping**

Replace the existing `update_library_entry_trim` in `src-tauri/src/database/library.rs`:

```rust
pub fn update_library_entry_trim(
    conn: &Connection,
    id: &str,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
) -> Result<(), String> {
    // Validate: start < end when both set
    if let (Some(ts), Some(te)) = (trim_start, trim_end) {
        if ts >= te {
            return Err("trim_start must be less than trim_end".to_string());
        }
    }

    // Write trim values
    conn.execute(
        "UPDATE library_entries SET trim_start = ?1, trim_end = ?2 WHERE id = ?3",
        params![trim_start, trim_end, id],
    ).map_err(|e| e.to_string())?;

    // Clamp loop points to fit within new trim bounds
    let (loop_start, loop_end): (Option<f64>, Option<f64>) = conn.query_row(
        "SELECT loop_start, loop_end FROM library_entries WHERE id = ?1",
        params![id], |r| Ok((r.get(0)?, r.get(1)?))
    ).map_err(|e| e.to_string())?;

    let mut ls = loop_start;
    let mut le = loop_end;
    let mut changed = false;

    if let (Some(ref mut loop_s), Some(trim_s)) = (&mut ls, trim_start) {
        if *loop_s < trim_s {
            *loop_s = trim_s;
            changed = true;
        }
    }
    if let (Some(ref mut loop_e), Some(trim_e)) = (&mut le, trim_end) {
        if *loop_e > trim_e {
            *loop_e = trim_e;
            changed = true;
        }
    }

    // If after clamping loop_start >= loop_end, clear both
    if let (Some(loop_s), Some(loop_e)) = (ls, le) {
        if loop_s >= loop_e {
            ls = None;
            le = None;
            changed = true;
        }
    }

    if changed {
        conn.execute(
            "UPDATE library_entries SET loop_start = ?1, loop_end = ?2 WHERE id = ?3",
            params![ls, le, id],
        ).map_err(|e| e.to_string())?;
    }

    Ok(())
}
```

- [ ] **Step 3: Update update_library_entry_loop with clamping**

Replace the existing `update_library_entry_loop` in `src-tauri/src/database/library.rs`:

```rust
pub fn update_library_entry_loop(
    conn: &Connection,
    id: &str,
    loop_start: Option<f64>,
    loop_end: Option<f64>,
) -> Result<(), String> {
    // Read current trim bounds
    let (trim_start, trim_end): (Option<f64>, Option<f64>) = conn.query_row(
        "SELECT trim_start, trim_end FROM library_entries WHERE id = ?1",
        params![id], |r| Ok((r.get(0)?, r.get(1)?))
    ).map_err(|e| e.to_string())?;

    // Clamp to trim bounds
    let mut ls = loop_start;
    let mut le = loop_end;

    if let (Some(ref mut loop_s), Some(trim_s)) = (&mut ls, trim_start) {
        if *loop_s < trim_s {
            *loop_s = trim_s;
        }
    }
    if let (Some(ref mut loop_e), Some(trim_e)) = (&mut le, trim_end) {
        if *loop_e > trim_e {
            *loop_e = trim_e;
        }
    }

    // Validate: start < end when both set
    if let (Some(loop_s), Some(loop_e)) = (ls, le) {
        if loop_s >= loop_e {
            return Err("loop_start must be less than loop_end".to_string());
        }
    }

    conn.execute(
        "UPDATE library_entries SET loop_start = ?1, loop_end = ?2 WHERE id = ?3",
        params![ls, le, id],
    ).map_err(|e| e.to_string())?;

    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cd src-tauri && cargo test database::library::tests -v`

Expected: All tests pass including the 5 new clamping tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/database/library.rs
git commit -m "feat: trim/loop data clamping with validation"
```

---

## Task 5: Add trim points to Clip model and engine

**Files:**
- Modify: `src-tauri/src/models.rs`
- Modify: `src-tauri/src/database/mod.rs`
- Modify: `src-tauri/src/audio/engine.rs`

- [ ] **Step 1: Add trim fields to Clip struct**

In `src-tauri/src/models.rs`, add to the `Clip` struct:

```rust
pub struct Clip {
    pub id: String,
    pub name: String,
    pub file_path: String,
    pub volume: f64,
    pub library_entry_id: String,
    pub library_volume: f64,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
}
```

- [ ] **Step 2: Resolve trim points in load_clips**

In `src-tauri/src/database/mod.rs`, update the `load_clips` function's map closure to resolve trim_start and trim_end from the library entry:

```rust
.map(|(clip_id, entry_id, volume)| {
    let (name, file_path, library_volume, trim_start, trim_end) = entry_map
        .get(&entry_id)
        .map(|e| (e.name.clone(), e.file_path.clone(), e.volume, e.trim_start, e.trim_end))
        .unwrap_or_else(|| ("[missing]".to_string(), String::new(), 1.0, None, None));

    Clip {
        id: clip_id,
        library_entry_id: entry_id,
        name,
        file_path,
        volume,
        library_volume,
        trim_start,
        trim_end,
    }
})
```

- [ ] **Step 3: Update engine to use trim_start for start position**

In `src-tauri/src/audio/engine.rs`, in `start_clip_inner`, after the `sound_data` is loaded and before the start_position logic, add trim_start handling:

Find:
```rust
if let Some(pos) = start_position {
    sound_data = sound_data.start_position(pos);
}
```

Replace with:
```rust
// Apply trim_start as default start position (overridden by explicit seek)
let effective_start = start_position.or(clip.trim_start);
if let Some(pos) = effective_start {
    sound_data = sound_data.start_position(pos);
}
```

- [ ] **Step 4: Update engine to use trim_end for loop region and clip duration**

In the same function, update the Seamless loop region to respect trim bounds:

Find the loop region code and update:
```rust
TrackMode::Seamless { loop_start, loop_end } => {
    let ls = clip.trim_start.map(|ts| (*loop_start).max(ts)).unwrap_or(*loop_start);
    let raw_le = if *loop_end > 0.0 { *loop_end } else { clip.trim_end.unwrap_or(f64::MAX) };
    let le = clip.trim_end.map(|te| raw_le.min(te)).unwrap_or(raw_le);

    let region_start = PlaybackPosition::Seconds(ls);
    let region_end = if le < f64::MAX {
        EndPosition::Custom(PlaybackPosition::Seconds(le))
    } else {
        EndPosition::EndOfAudio
    };
    sound_data = sound_data.loop_region(Region {
        start: region_start,
        end: region_end,
    });
}
```

- [ ] **Step 5: Update clip duration tracking for crossfade/advancement**

After the sound is started, where `current_clip_duration` is set:

Find:
```rust
active.current_clip_duration = probe_duration(&clip.file_path);
```

Replace with:
```rust
// Effective duration accounts for trim points
let raw_duration = probe_duration(&clip.file_path);
active.current_clip_duration = match (clip.trim_start, clip.trim_end, raw_duration) {
    (_, Some(te), _) => Some(te),   // trim_end is the effective end
    (_, None, Some(d)) => Some(d),  // no trim, use file duration
    _ => None,
};
```

- [ ] **Step 6: Update crossfade trigger to use effective end**

In `check_crossfade_triggers`, the comparison `position >= clip_duration - crossfade_duration` already uses `current_clip_duration` which we've set to `trim_end` when applicable. This should work without changes since `trim_end` represents the absolute time position in the file, and `handle.position()` returns absolute position.

Verify this is correct by reading the `check_crossfade_triggers` function.

- [ ] **Step 7: Verify compilation and tests**

Run: `cd src-tauri && cargo check && cargo test`

Expected: Clean compilation, all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/models.rs src-tauri/src/database/mod.rs src-tauri/src/audio/engine.rs
git commit -m "feat: trim point support in clip model and audio engine"
```

---

## Task 6: Waveform editor trim markers and constraints

**Files:**
- Modify: `src/components/WaveformEditor.vue`

- [ ] **Step 1: Add trim marker props and emit**

Update the props and emits:

```typescript
const props = defineProps<{
  filePath: string;
  loopStart: number;
  loopEnd: number;
  trimStart: number | null;
  trimEnd: number | null;
  trackId?: string;  // optional — not needed in library context
}>();

const emit = defineEmits<{
  'update:loopStart': [value: number];
  'update:loopEnd': [value: number];
  'update:trimStart': [value: number | null];
  'update:trimEnd': [value: number | null];
}>();
```

- [ ] **Step 2: Add trim marker types and interaction**

Extend the `MarkerType` to include trim markers:

```typescript
type MarkerType = 'loopStart' | 'loopEnd' | 'trimStart' | 'trimEnd' | null;
```

Update `findNearestMarker` to include trim markers (use a distinct color — orange for trim, green for loop).

Update `clampAndEmit` to enforce all four constraints:
- trimStart: `0 <= trimStart < trimEnd`, if dragged past loopStart → push loopStart
- trimEnd: `trimStart < trimEnd <= duration`, if dragged before loopEnd → push loopEnd
- loopStart: `trimStart <= loopStart < loopEnd`
- loopEnd: `loopStart < loopEnd <= trimEnd`

- [ ] **Step 3: Draw trim markers and dim trimmed regions**

In `drawWaveform`, after drawing the waveform peaks but before loop markers:
1. Draw dimmed overlay (semi-transparent dark) over the regions before trim_start and after trim_end
2. Draw trim markers as orange lines with small tab indicators (similar style to loop markers but orange)
3. Draw loop markers (green, existing code) — constrained within trim region

- [ ] **Step 4: Verify it works**

Run: `npm run tauri dev`

Test in the Library waveform dialog (Task 7 builds the dialog, but you can temporarily test by adding trim props to the existing TrackSettings waveform if needed).

- [ ] **Step 5: Commit**

```bash
git add src/components/WaveformEditor.vue
git commit -m "feat: trim markers in waveform editor with drag constraints"
```

---

## Task 7: Waveform editor dialog in Library

**Files:**
- Create: `src/components/WaveformEditorDialog.vue`
- Modify: `src/views/LibraryView.vue`
- Modify: `src/components/TrackSettings.vue`
- Modify: `src/composables/useTrackUpdates.ts`

- [ ] **Step 1: Create WaveformEditorDialog.vue**

```vue
<script setup lang="ts">
import { ref, watch, computed } from 'vue';
import type { LibraryEntry } from '@generated/index';
import { useAppStore } from '../stores/appStore';
import WaveformEditor from './WaveformEditor.vue';

const props = defineProps<{
  modelValue: boolean;
  entryId: string;
}>();

const emit = defineEmits<{
  'update:modelValue': [value: boolean];
}>();

const store = useAppStore();

const entry = computed(() => store.findLibraryEntry(props.entryId));

// Local state for name editing
const editingName = ref('');
const editingTags = ref('');
watch(() => entry.value, (e) => {
  if (e) {
    editingName.value = e.name;
    editingTags.value = e.tags.join(', ');
  }
}, { immediate: true });

async function saveName(): Promise<void> {
  if (!editingName.value.trim() || !entry.value) return;
  await store.command('rename_library_entry', { entryId: props.entryId, name: editingName.value.trim() });
}

async function saveTags(): Promise<void> {
  if (!entry.value) return;
  const tags = editingTags.value.split(',').map((t) => t.trim()).filter((t) => t.length > 0);
  await store.command('update_library_entry_tags', { entryId: props.entryId, tags });
}

async function updateVolume(v: number): Promise<void> {
  await store.command('update_library_entry_volume', { entryId: props.entryId, volume: v });
}

async function updateLoopStart(v: number): Promise<void> {
  await store.command('update_library_entry_loop_points', {
    entryId: props.entryId,
    loopStart: v,
    loopEnd: entry.value?.loopEnd ?? null,
  });
}

async function updateLoopEnd(v: number): Promise<void> {
  await store.command('update_library_entry_loop_points', {
    entryId: props.entryId,
    loopStart: entry.value?.loopStart ?? null,
    loopEnd: v,
  });
}

async function updateTrimStart(v: number | null): Promise<void> {
  await store.command('update_library_entry_trim', {
    entryId: props.entryId,
    trimStart: v,
    trimEnd: entry.value?.trimEnd ?? null,
  });
}

async function updateTrimEnd(v: number | null): Promise<void> {
  await store.command('update_library_entry_trim', {
    entryId: props.entryId,
    trimStart: entry.value?.trimStart ?? null,
    trimEnd: v,
  });
}

function close(): void {
  emit('update:modelValue', false);
}
</script>

<template>
  <VDialog :model-value="modelValue" max-width="95%" @update:model-value="close">
    <VCard v-if="entry">
      <VCardTitle class="d-flex align-center">
        <VTextField
          v-model="editingName"
          variant="plain"
          density="compact"
          hide-details
          class="text-h6 flex-grow-1"
          @blur="saveName"
          @keydown.enter="($event.target as HTMLInputElement)?.blur()"
        />
        <VBtn icon="mdi-close" variant="text" @click="close" />
      </VCardTitle>

      <VCardText>
        <!-- Tags -->
        <div class="d-flex align-center ga-2 mb-4">
          <VIcon size="small">mdi-tag</VIcon>
          <VTextField
            v-model="editingTags"
            variant="outlined"
            density="compact"
            hide-details
            placeholder="Tags (comma-separated)"
            style="max-width: 400px;"
            @blur="saveTags"
            @keydown.enter="($event.target as HTMLInputElement)?.blur()"
          />
        </div>

        <!-- Gain volume -->
        <div class="d-flex align-center ga-2 mb-4">
          <VIcon size="small">mdi-volume-high</VIcon>
          <span class="text-caption text-medium-emphasis" style="min-width: 40px;">Gain</span>
          <VSlider
            :model-value="entry.volume"
            @update:model-value="updateVolume"
            :min="0"
            :max="2"
            :step="0.01"
            hide-details
            density="compact"
            style="max-width: 300px;"
          />
          <span class="text-caption text-medium-emphasis">{{ (entry.volume * 100).toFixed(0) }}%</span>
        </div>

        <!-- Waveform editor -->
        <WaveformEditor
          :file-path="entry.filePath"
          :loop-start="entry.loopStart ?? 0"
          :loop-end="entry.loopEnd ?? 0"
          :trim-start="entry.trimStart"
          :trim-end="entry.trimEnd"
          @update:loop-start="updateLoopStart"
          @update:loop-end="updateLoopEnd"
          @update:trim-start="updateTrimStart"
          @update:trim-end="updateTrimEnd"
        />

        <!-- Numeric inputs for precise editing -->
        <div class="d-flex ga-3 mt-3">
          <VTextField
            :model-value="entry.trimStart ?? ''"
            @update:model-value="(v: any) => updateTrimStart(v === '' ? null : Number(v))"
            label="Trim Start (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
            style="max-width: 140px;"
          />
          <VTextField
            :model-value="entry.trimEnd ?? ''"
            @update:model-value="(v: any) => updateTrimEnd(v === '' ? null : Number(v))"
            label="Trim End (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
            style="max-width: 140px;"
          />
          <VTextField
            :model-value="entry.loopStart ?? ''"
            @update:model-value="(v: any) => updateLoopStart(v === '' ? 0 : Number(v))"
            label="Loop Start (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
            style="max-width: 140px;"
          />
          <VTextField
            :model-value="entry.loopEnd ?? ''"
            @update:model-value="(v: any) => updateLoopEnd(v === '' ? 0 : Number(v))"
            label="Loop End (s)"
            type="number"
            variant="outlined"
            density="compact"
            hide-details
            style="max-width: 140px;"
          />
        </div>
      </VCardText>
    </VCard>
  </VDialog>
</template>
```

- [ ] **Step 2: Add a trim command to commands.rs and actions.rs**

In `src-tauri/src/commands.rs`, add:

```rust
#[tauri::command]
pub fn update_library_entry_trim(
    state: State<'_, Arc<AppStateContainer>>,
    entry_id: String,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
) -> Result<AppState, String> {
    actions::update_library_entry_trim_action(&state, &entry_id, trim_start, trim_end)
}
```

In `src-tauri/src/actions.rs`, add:

```rust
pub fn update_library_entry_trim_action(
    container: &AppStateContainer,
    entry_id: &str,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
) -> Result<AppState, String> {
    with_db(container, |conn| {
        library::update_library_entry_trim(conn, entry_id, trim_start, trim_end)?;
        database::load_full_state(conn)
    })
}
```

Register in `lib.rs` invoke_handler: `commands::update_library_entry_trim,`

- [ ] **Step 3: Wire dialog into LibraryView**

In `src/views/LibraryView.vue`, add dialog state and the component:

```typescript
import WaveformEditorDialog from '../components/WaveformEditorDialog.vue';

const showEditorDialog = ref(false);
const editorEntryId = ref('');

function openEditor(entryId: string): void {
  editorEntryId.value = entryId;
  showEditorDialog.value = true;
}
```

Add an edit button to each `LibraryEntryRow` in the actions slot, and add the dialog to the template:

```vue
<VBtn icon="mdi-waveform" size="small" variant="text" title="Edit audio" @click.stop="openEditor(entry.id)" />

<!-- At the bottom of the template -->
<WaveformEditorDialog v-model="showEditorDialog" :entry-id="editorEntryId" />
```

- [ ] **Step 4: Remove waveform from TrackSettings**

In `src/components/TrackSettings.vue`:
- Remove the `WaveformEditor` import
- Remove the `seamlessClip` and `showWaveform` computed properties
- Remove the waveform section from the template (the `v-if="showWaveform"` block)
- Remove the loop start/end numeric inputs (lines 117-139)
- Remove the `updateLoopStart`, `updateLoopEnd`, `updateLoopStartFromWaveform`, `updateLoopEndFromWaveform` imports from `useTrackUpdates`

- [ ] **Step 5: Clean up useTrackUpdates**

In `src/composables/useTrackUpdates.ts`, remove the loop point methods and helpers that are no longer needed:
- `updateLoopStart`, `updateLoopEnd`, `updateLoopStartFromWaveform`, `updateLoopEndFromWaveform`
- `getSeamlessClipEntryId`, `getCurrentLoopStart`, `getCurrentLoopEnd`

These are now handled by the `WaveformEditorDialog` directly.

- [ ] **Step 6: Verify everything compiles and works**

Run: `npx vue-tsc --noEmit`
Run: `cd src-tauri && cargo check && cargo test`
Run: `npm run tauri dev`

Test:
- Library view: click edit button on an entry → full-width waveform dialog opens
- Drag trim markers → dimmed regions update, loop markers get pushed if needed
- Drag loop markers → constrained within trim bounds
- Numeric inputs work for precise values
- Name and tag editing works
- Gain slider works
- TrackSettings no longer shows waveform or loop inputs for Seamless mode
- Playback respects trim_start (starts at trim point) and trim_end (stops/loops within trim)

- [ ] **Step 7: Commit**

```bash
git add src/components/WaveformEditorDialog.vue src/views/LibraryView.vue src/components/TrackSettings.vue src/composables/useTrackUpdates.ts src-tauri/src/commands.rs src-tauri/src/actions.rs src-tauri/src/lib.rs
git commit -m "feat: library waveform editor dialog with trim/loop editing"
```

---

## Task 8: Regenerate types and final cleanup

**Files:**
- Regenerate: `generated/`
- Various cleanup

- [ ] **Step 1: Regenerate TypeScript types**

Run: `cd src-tauri && cargo test export_bindings` or `cargo build`

Verify `generated/Clip.ts` now includes `trimStart` and `trimEnd` fields.
Update `generated/index.ts` if needed.

- [ ] **Step 2: Final type check and test run**

Run: `npx vue-tsc --noEmit`
Run: `cd src-tauri && cargo test`

- [ ] **Step 3: Full integration test**

Run: `npm run tauri dev`

Test checklist:
- Nav rail works, routes switch correctly
- Soundstage: groups, comps, tracks, playback all work
- Library: search, filter, import, tag editing all work
- Library waveform dialog: trim markers, loop markers, drag constraints, numeric inputs
- Trim points: audio starts at trim_start, stops/loops at trim_end
- Crossfade respects trim_end
- Transport bar: master volume, pause/stop/resume all, track count indicator
- Remote control still works (Soundstage functionality only)

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "cleanup: regenerate types, final integration"
```
