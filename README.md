# LuteDrop

A soundscape and ambient audio manager for tabletop RPGs, live performances, and immersive environments. Build layered soundscapes with multiple tracks, organize your audio library, and control everything from your phone.

## Getting Started

### Install

Grab the latest build from the [Releases](../../releases) page. Each release ships in two
forms:

- **Installer** — `LuteDrop_<version>_x64-setup.exe` (NSIS) or `LuteDrop_<version>_x64_en-US.msi`.
  Installs to Program Files with Start-menu shortcuts.
- **Portable** — `LuteDrop_<version>_x64_portable.zip`: no install, unzip and run `lutedrop.exe`;
  the `dist-remote/` folder inside enables the phone remote.

Either way you need the **Microsoft Edge WebView2 Runtime** (preinstalled on Windows 11; the
installer fetches it automatically, the portable build assumes it's present).

### Import Audio

1. Go to **Library** (sidebar).
2. Click **Import Files** or **Import Folder** to add audio files (mp3, wav, ogg, flac, aac, m4a, wma, opus).
3. Assign a **type** (Music or Ambiance) and optional **tags** during import.

### Organize Your Library

- **Search** by name using the search bar.
- **Filter by type** using the All / Music / Ambiance toggle.
- **Filter by tags** to narrow down entries.
- Click the **waveform icon** on any entry to open the editor, where you can:
  - Rename the entry
  - Change type and tags
  - Adjust volume/gain
  - Set trim points (start/end)
  - Set loop points (start/end)
- **Fix missing files** (next to Import) walks through entries whose audio file has moved or been deleted, searches a folder for files with a matching name (your library's folder by default), and relinks each with one click. Skipping an entry leaves it for next time.

## Auto-tagging

Auto-tagging suggests a role and tags for music entries by comparing how they sound with entries that were classified by hand. It needs the analyzer service, a separate service that needs a GPU host; see [`tagger/README.md`](tagger/README.md) for how to run it. The **Auto-tagging** button in the Library toolbar opens the settings.

### Role

Every music entry has a role, **Combat** or **Non-combat**; there is no "no role" state. The role shown is the one you set, else the analyzer's suggestion, else **Non-combat** by default. Ambiance entries have no role. The analyzer scores the Combat probability of each entry. Its suggestion is Combat when the probability reaches the Combat threshold, Non-combat when one minus the probability reaches the Non-combat threshold, and otherwise there is none (the default shows). An auto role is suggested only once the role classifier is trusted.

- A role set by hand always wins. Auto-tagging never changes it.
- A suggested role (the analyzer's, or the default) is shown as **✨ Suggested**, with the analyzer's score when there is one. **Confirm** makes it yours; a role you set reads **Set by you**, and **↺** goes back to the suggestion.
- The Library has a Role column and a Role filter (Any role, Combat, Non-combat). The entry editor has a Combat / Non-combat control, and the bulk bar can set the role on several music entries at once or send them back to the suggestion.
- Only entries whose role was set by hand count as examples. Auto roles never do, and neither does the Non-combat default.

### Moods

A mood is a **lens**, not a label: picking a mood ranks the whole music library by how well each track fits it. No track belongs to a mood, and nothing is cut off. The moods are yours to edit (see **Mood vocabulary** below); a new library starts with nine moods and Energy:

- **Peaceful**, **Sorrowful**, **Wondrous**, **Joyful**, **Quirky**, **Mysterious** (enigma and chimes), **Grim** (fear and dread), **Heroic**, **Battle** (combat)
- **Energy** (calm and slow at 0, energetic at 1)

Each music track has a **mood profile**: one value from 0 to 1 per mood, plus one for Energy. Ambiance entries have no moods.

The analyzer suggests every value by comparing the track's sound with a few descriptions of each mood. The bars are measured against a fixed reference set of 300 soundtrack-style tracks (Kevin MacLeod, incompetech.com, CC BY 4.0) that ships with the analyzer, so they mean the same thing in any library: 0.5 is about the top quarter of soundtrack music for that mood, 0.75 the top 10% ("could be heard as this"), and 0.9 the top 3% ("for sure"). Most tracks sit below 0.5 in most moods. Energy is two-sided, so its bar reads 0.5 at the average soundtrack energy. Adding or removing tracks never changes a bar, and editing a mood's words needs no recalibration, because every analysis run recalibrates against the reference set for the current words. With an older analyzer that has no reference set, the bars fall back to a calibration taken once from the first run that covers at least 50 music tracks (until then no moods are suggested): a mood's bar reads 0.5 one standard deviation above your library's average for that mood, and Energy reads 0.5 at the library average. The ranking never depends on this mapping, only the number shown. Upgrading from the pools release clears the old pools; the next analysis run fills in the profiles.

- **The profile.** One bar per mood, each with five levels (0, 0.25, 0.5, 0.75, 1). A bar at the analyzer's value is **Suggested** and drawn muted. Clicking a bar (or using the arrow keys on a focused bar) sets the nearest level; the bar is then **Set by you**, drawn in the accent colour, and hovering it shows "Set by you · suggested 0.62". **↺** goes back to the suggestion. A level you set always wins over the analyzer's.
- The entry editor has a **Role** row and the mood profile under it. The Library's bulk bar has **Set mood levels…**, which opens the profile for the selected music; a click sets that level on every selected track, and a bar whose tracks differ shows "mixed".
- **Lenses.** The Library and Scry have a row of lens chips, one per mood. Each chip is simply **on or off**. Moods that are on rank the list, highest first, by the *weaker* of their scores, so with Grim and Heroic on a track ranks high only when it is high in both. A mood never hides a track; tracks not analyzed yet stay, ranked last. The **Match** column shows the score used for ranking; hovering its header spells out the rule, for example "weakest of Heroic, Grim".
- **Energy bands.** After the mood chips, set a little apart, are three Energy chips: **Low** (below 0.375), **Mid** (0.375 to 0.625) and **High** (above 0.625), using the Energy you set, else the suggested one. So the levels 0 and 0.25 are Low, 0.5 is Mid, 0.75 and 1 are High. Any combination can be on, and a track shows when its Energy is in any of them. With none on there is no Energy filter; with any on, tracks without an Energy value are hidden. The bands only filter; they never change the order.
- Ambiance is hidden while any mood or Energy band is on.
- **Sorting** is done from the column headers only. The Library sorts by status (missing files first), Type, Name, Role, Energy, Duration, and Match while a mood is on; Scry by Name, Energy, Search (how close a track is to the search or seeds) and Match. Click a header to sort by it, again to flip the direction. Turning on the first mood sorts by Match, highest first; turning off the last one goes back to the view's default sort (Name in the Library, Search in Scry). While a mood is on, the Energy header does not sort, so the order never depends on Energy then; use the bands to narrow by Energy. Levels you set are coarse, so tracks set to the same level tie: within a level, Match and Energy order them by the analyzer's suggestion behind the level (the value shown does not change), and any tie left goes by name. The filter bar also has the Role filter.
- Tag chips have three states, **off → include → exclude**, one per click: an entry passes when it has at least one included tag (if any is included) and none of the excluded ones (red outline, struck through). **Untagged** and **Has auto tags** are plain on/off filters on top.

#### Mood vocabulary

**Moods** in the left rail opens the mood vocabulary: the moods the analyzer scores and the text prompts it compares each track with.

- Each mood has an **id** (lowercase letters; it cannot change once saved), a **name** (rename freely), and **prompts**, one per line. Moods can be added, removed and moved up or down; the order is the order of the lens chips and the profile. Energy is always last and has two sides of prompts: energetic and calm.
- The **word bank** above the list holds tag words the analyzer's text model knows well. Clicking one adds it, on a line of its own, to the prompt field you were last in. Single tag words work as well as sentences, but not every word is reliable, so check with Preview.
- **Preview** on a mood lists the 15 tracks its current prompts rank highest, with their album folder and raw similarity, without saving or recalibrating anything. It needs the analyzer. **Preview** under Energy lists the 15 most and the 15 least energetic tracks for the two sides as written (raw difference of similarities), the same way.
- **Save** stores the vocabulary. Removing a mood deletes its values on every track; when that includes levels set by you, Save asks first and says how many.
- **Recalibrate** clears the calibration and every suggested mood value (levels set by you stay) and starts an analysis run, which suggests every mood again from the current prompts. The new calibration comes from the analyzer's reference set when it has one, otherwise from your library. The panel shows which of the two the bars are measured against, and how many tracks it was taken from.
- **Reset to defaults** puts back the shipped mood list and prompts. Levels set by you stay for moods still in the list; moods not in the defaults lose their values, and the confirm dialog says how many. It also works when the stored list is unreadable (when Save is blocked). If prompts changed, it then offers to recalibrate.
- After you change a mood's prompts (or add a mood), Save offers to recalibrate. Until you do, the lens row shows **Needs recalibration**, and the changed moods keep their old suggestions; the others are still suggested as before.

### Auto tags

Auto tags are learned from the tags the user applied by hand.

- A tag's positive examples are music entries that carry it. Its negative examples are music entries with a hand-set role but without the tag, plus entries where it was removed as an auto tag. Auto tags never count as examples.
- A tag is applied only when the analyzer can predict it reliably (trusted): at least 5 positive and 5 negative examples, and good separation when tested on them. The tag quality table in the settings dialog shows each tag's example counts, AUC and, for tags not yet trusted, what is missing.
- Auto tags are marked on their chip. **Keep** turns one into a normal tag. **Remove** takes it off and stops it being applied to that track again.
- Auto-tagging never overwrites or removes a tag applied by hand.
- The Library's tag filter can show only entries with auto tags, and bulk actions can keep or remove auto tags on several entries at once.
- In the entry editor, the **Tag scores** panel shows each tag's score against its threshold and whether the entry's tag was applied by hand or automatically.

### Scry

**Scry** browses analyzed music by sound.

- **Text search.** A description such as "tense strings building to a battle" returns the closest tracks, ranked by score.
- **More like this.** The action on a Library row opens Scry seeded with that track. Seeds are shown as removable chips, more can be added from the results, and the results are the tracks closest to the seeds together.
- **Filters.** The results can be ranked by mood lens chips and filtered by Energy band and Role, and sorted from the column headers by Search, Name, Energy (while no mood is on) or Match.
- **Map.** A scatter plot of every analyzed music track, colored by role. Hovering shows the track name and clicking plays it. Dragging draws a lasso that adds the enclosed tracks to the selection. Seeds and results are highlighted.
- **Playlists.** The selection can become a playlist with **Create playlist**, which adds a composition with one shuffle track holding the selected tracks to a chosen group of the active configuration, or be appended to a track with **Add to existing track**.

### Library search

The Library search box also matches the album and artist folder names of an entry, so a game can be found by its soundtrack folder.

### Settings

- **Enable auto-tagging.** Turning it on starts analyzing the library right away. New imports are analyzed as they arrive.
- **Analyzer URL** and **Token** identify the service. **Test connection** reports the service's model, whether it has finished loading, and how many embeddings it has cached, or the error.
- **Local path prefix** and **Analyzer path prefix** map the library folder on this machine to the same folder as the analyzer sees it. For example, a library opened here as `Z:\media\audio` that the analyzer sees as `/media/audio` uses local prefix `Z:\media\audio` and analyzer prefix `/media/audio`. Entries outside the local prefix are counted as unreachable.
- **Analyze library** analyzes every entry that has no current analysis, with a progress bar and a summary of the last run. The summary lists up to five entries that failed or were unreachable, with the reason for each.
- **Vocabulary** lists the tags that can be applied automatically.

## Building Soundscapes

### Structure

Soundscapes are organized in a hierarchy:

- **Configuration** — A saved state (think of it as a "project" or "session")
- **Group** — A tab (e.g. "Combat", "Town", "Dungeon")
- **Composition** — A scene within a group (e.g. "Tavern Interior", "Boss Fight")
- **Track** — An audio layer within a composition (e.g. "Background Music", "Rain", "Crowd Chatter")
- **Clip** — An audio file assigned to a track

### Creating a Soundscape

1. On the **Soundstage** view, click **+** next to the group tabs to create a group.
2. Click **Add Comp** to create a composition within the group.
3. Select the composition, then click **Add Track**.
4. Expand a track and click **Browse Library** to add clips from your library.
5. Drag clips to reorder them within a track.

### Track Modes

Each track has a playback mode:

| Mode | Behavior |
|------|----------|
| **Seamless** | Loops a single clip continuously with optional loop points |
| **Sequence** | Plays clips in order, optionally repeating |
| **Shuffle** | Plays clips in random order, optionally repeating |

### Transitions (Sequence & Shuffle)

| Transition | Behavior |
|------------|----------|
| **Gapless** | Next clip starts immediately |
| **Gap** | Configurable silence between clips (min/max seconds) |
| **Crossfade** | Clips fade into each other |

Tracks also support **fade in** and **fade out** toggles.

### Playback

- **Play/Pause/Stop** individual tracks using the buttons on each track row.
- **Activate a composition** to play all its tracks at once.
- **Stop All / Pause All** from the transport bar at the bottom.
- Adjust **master volume** and **per-track/clip volume** with sliders.
- **Mute** individual tracks, clips, or entire compositions.

## Remote Control

LuteDrop runs a local web server so you can control playback from any device on your network (phone, tablet, another computer).

1. The server starts automatically on **port 3333**.
2. In the desktop app, click the **Phone remote** icon at the bottom of the sidebar to
   open a dialog with the pairing link, e.g. `http://<your-computer-ip>:3333/?token=<code>`.
3. Open that link in a browser on any device connected to the same network.
4. The link includes a pairing code required to connect; it's regenerated each time
   LuteDrop starts, so reopen the dialog for a fresh link after a restart.

The remote provides:
- Group tabs and composition selection
- Play/pause/stop for tracks and compositions
- Master, track, and clip volume control
- Real-time status updates via WebSocket

## Broadcast

Broadcast streams what LuteDrop is playing to listeners over the internet, so remote
players hear the same soundscape you do. It is **off until you switch it on**.

**What is streamed:** the live mix of your tracks, taken *before* master volume. **What
is not:** the library preview (auditioning an entry in the editor) is never streamed.
Master volume (and master mute) only changes what comes out of your speakers; listeners
hear the full-level mix regardless. The stream is Ogg/Opus at 128 kbps.

1. Click the **Broadcast** entry in the sidebar to open the dialog and switch it on.
2. The dialog shows two links:
   - **Same Wi-Fi** - `http://<your-computer-ip>:5883/listen/<key>`, built from your
     computer's local address. Works for anyone on your network with no extra setup.
   - **Players on the internet** - your **Public address** plus `/listen/<key>`. This
     stays empty until you fill in the Public address field (see below).
3. Add `/page` to either link (`.../listen/<key>/page`) for a bare listening page with a
   play button, handy for a phone browser. The Foundry module plays the raw link instead.
4. The badge in the transport bar and the dialog show how many people are listening.

The stream route (`/listen/<key>`) sends `Access-Control-Allow-Origin: *` so the Foundry module
can read it with `fetch()` from its own page. The key in the URL stays the only secret, and no
cookies are involved. The `/page` listening page does not send it. The server also sends 40 ms
Ogg pages with `TCP_NODELAY` to keep the stream low-latency.

The stream is served on **port 5883** by default. You can change the port in the dialog,
but only while Broadcast is off. Settings live in the database's `meta` table under
`broadcast_enabled`, `broadcast_port`, `broadcast_key` and `broadcast_public_base`; if
Broadcast was on when you quit, it turns itself back on at the next start.

**Regenerate key** (in the dialog) makes a new listener key. The old link stops working
immediately, so everyone needs the new one.

### Public access through Caddy

The public link goes through the same Caddy reverse proxy that fronts the phone remote.
On gargantua, add this block to `~/stacks/caddy/Caddyfile`, beside the existing
`lutedrop.irslandia.duckdns.org` one:

```
stream.lutedrop.irslandia.duckdns.org {
	import tls_duckdns
	reverse_proxy 192.168.86.42:5883 {
		flush_interval -1
	}
}
```

`flush_interval -1` tells Caddy to pass stream bytes through immediately instead of
buffering the response, which would otherwise make the audio stutter or never start.

Caddy runs from `~/stacks/caddy/docker-compose.yml` as the `caddy` container, with the
Caddyfile mounted read-only, so reload it with:

```sh
docker compose -f ~/stacks/caddy/docker-compose.yml exec caddy caddy reload --config /etc/caddy/Caddyfile
```

Then, in the Broadcast dialog, set **Public address** to
`https://stream.lutedrop.irslandia.duckdns.org` and press Save.

> If the two-level name `stream.lutedrop.irslandia.duckdns.org` does not resolve, use
> `lutedrop-stream.irslandia.duckdns.org` instead, in both the Caddy block and the
> dialog's Public address.

To check it from the desktop (with Broadcast on), run:

```sh
curl -sI https://stream.lutedrop.irslandia.duckdns.org/listen/<key>
```

Expect `HTTP/2 200` and `content-type: audio/ogg`. With Broadcast off you get `502`. For
the real test, open `https://stream.lutedrop.irslandia.duckdns.org/listen/<key>/page` on
a phone using mobile data and press play.

### Notes

- Safari may not play Ogg/Opus; use Chrome or Firefox for listeners where possible.
- The first time Broadcast binds port 5883, Windows Firewall may ask whether to allow
  LuteDrop (dev builds ask again from each new exe path); allow it, because dismissing
  the prompt blocks other machines, so Caddy returns `502`.
- To play the stream inside Foundry VTT for every player, install the module from the
  manifest URL `https://github.com/infrasparker/LuteDrop/releases/latest/download/module.json`
  (Foundry: Add-on Modules > Install Module); details in
  [`foundry-module/README.md`](foundry-module/README.md).

## Configurations

Configurations save your entire soundstage state (groups, compositions, tracks, clips, and settings). Use the configuration picker in the top bar to switch between saved configurations — useful for different campaigns or sessions.

## Shortcuts

Use the transport bar at the bottom of the screen for global controls:
- **Pause/Resume All** — Pause or resume every playing track
- **Stop All** — Stop all playback

## Development

LuteDrop is a [Tauri 2](https://tauri.app) app: a Vue 3 + Vite frontend with a Rust backend.

### Prerequisites

- **Node.js** 18+ and npm
- **Rust** (stable, `x86_64-pc-windows-msvc` toolchain) via [rustup](https://rustup.rs)
- **Visual Studio C++ Build Tools** — the MSVC compiler/linker + Windows SDK that the
  Rust `msvc` toolchain links against. Install the "Desktop development with C++" workload
  (e.g. `winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"`).
- **The Opus C libraries** (libogg / libopus / opusfile). The audio engine is
  [miniaudio](https://miniaud.io) (vendored in `src-tauri/vendor/`); its `ma_libopus`
  decoding backend links the xiph Opus stack **statically**. Get them via
  [vcpkg](https://github.com/microsoft/vcpkg):
  ```sh
  git clone https://github.com/microsoft/vcpkg
  vcpkg\bootstrap-vcpkg.bat
  vcpkg\vcpkg install opusfile:x64-windows-static
  ```
  `src-tauri/build.rs` links these from the `x64-windows-static` triplet. The vcpkg root is
  resolved from the **`VCPKG_ROOT`** environment variable if set, otherwise it falls back to
  `%USERPROFILE%\vcpkg` (i.e. `~\vcpkg`). No path is hardcoded in `build.rs`; set `VCPKG_ROOT`
  if your vcpkg install is elsewhere.
- **CMake** (`winget install Kitware.CMake`) — only used *by vcpkg* to build the Opus libs
  above; the app itself compiles its C shim with the `cc` crate (no CMake at app build time).

### Setup

```sh
npm install
```

### Run (dev)

```sh
cargo tauri dev      # or: npm run tauri dev
```

This launches Vite and the Rust backend with hot-reload and opens the app window. To also run
the phone-remote dev server (port 3333) alongside it:

```sh
npm run dev:all
```

### Tests

```sh
cd src-tauri && cargo test
```

## Building a Release

Produces an optimized binary and a Windows installer:

```sh
cargo tauri build    # or: npm run tauri build
```

This runs the frontend build (`npm run build` + `npm run build:remote`), compiles the Rust
backend in release mode, and bundles installers. Output lands in:

```
src-tauri/target/release/bundle/
  nsis/LuteDrop_<version>_x64-setup.exe   # NSIS installer
  msi/LuteDrop_<version>_x64_en-US.msi    # MSI installer
```

Before building a new release, bump the version in `src-tauri/tauri.conf.json`,
`package.json`, `src-tauri/Cargo.toml`, and `foundry-module/lutedrop-stream/module.json`
(its `version` and the `/v<version>/` in its `download` URL). `npm run package:release`
refuses to run if they disagree.

### Packaging the release assets

After `cargo tauri build`, stage everything for GitHub in one step:

```sh
npm run package:release
```

This fills the git-ignored `release/` folder with:

```
release/
  LuteDrop_<version>_x64-setup.exe        # NSIS installer
  LuteDrop_<version>_x64_en-US.msi        # MSI installer
  LuteDrop_<version>_x64_portable.zip     # lutedrop.exe + dist-remote/ (no install needed)
  lutedrop-stream.zip                     # the Foundry module
  module.json                             # the Foundry manifest
  portable/                               # the unzipped portable build, kept for running locally
```

The `portable/` folder is for this machine: point a shortcut at `release/portable/lutedrop.exe`
and every release refreshes it. Close LuteDrop before packaging, or the folder cannot be replaced.

The portable zip works because `lutedrop.exe` runs by itself, and the `dist-remote/`
folder beside it enables the port-3333 phone remote.

**Every release must attach `lutedrop-stream.zip` and `module.json`.** Foundry's manifest
URL points at the *latest* release, so a release without them breaks module installs and
updates. Upload with the five files listed explicitly (PowerShell does not expand `release/*` for
`gh`; in bash, use `\` instead of the backtick line continuations):

```powershell
gh release create v<version> `
  release/LuteDrop_<version>_x64-setup.exe release/LuteDrop_<version>_x64_en-US.msi `
  release/LuteDrop_<version>_x64_portable.zip release/lutedrop-stream.zip release/module.json `
  --title "LuteDrop v<version>" --notes-file <notes>
```

### Audio engine

Playback and mixing are handled by [miniaudio](https://miniaud.io) (vendored, called through
a small C shim in `src-tauri/vendor/` plus `src-tauri/src/audio/ffi.rs`). Every supported
format — mp3, wav, ogg, flac, aac, m4a, wma, **and opus** — is **streamed directly from
disk**: no whole-file decode, no transcoding, no cache. Opus is decoded by miniaudio's
`ma_libopus` backend (xiph libogg/libopus/opusfile, linked statically). Waveform peaks and
durations come from the same miniaudio decoder. Files on disk are never modified or
duplicated, so a library can be shared with other tools (e.g. Navidrome) without conflict.

The mixer maps directly onto miniaudio's node hierarchy: master volume on the engine,
per-track volume on a sound group, per-clip volume on the sound, and mute by zeroing the
relevant node. Seamless loops use miniaudio's sample-accurate scheduling for crossfades.
