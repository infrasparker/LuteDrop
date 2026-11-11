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

## Analyzer

The analyzer listens to music entries: it suggests their mood values and powers Scry. It needs the analyzer service, a separate service that needs a GPU host; see [`tagger/README.md`](tagger/README.md) for how to run it. The **Analyzer** button in the Library toolbar opens the settings. Tags and roles are user-managed; the analyzer never changes them. Click a Role icon in the Library or Scry to switch between Combat and Non-combat.

### Role

Every music entry has a role, **Combat** or **Non-combat**; there is no "no role" state. An entry is **Non-combat** until you set it to Combat. Ambiance entries have no role.

- The Library has a Role column and a Role filter (Any role, Combat, Non-combat). The entry editor has a Combat / Non-combat control, and the bulk bar can set the role on several music entries at once.

### Moods

A mood is a **lens**, not a label: picking a mood ranks the whole music library by how well each track fits it. No track belongs to a mood, and nothing is cut off. The moods are yours to edit (see **Mood vocabulary** below); a new library starts with nine moods and Energy:

- **Peaceful**, **Sorrowful**, **Wondrous**, **Joyful**, **Quirky**, **Mysterious** (enigma and chimes), **Grim** (fear and dread), **Heroic**, **Battle** (combat)
- **Energy** (calm and slow at 0, energetic at 1)

Each music track has a **mood profile**: one value from 0 to 1 per mood, plus one for Energy. Ambiance entries have no moods.

The analyzer suggests every value by comparing the track's sound with a few descriptions of each mood. The bars are measured against a fixed reference set of 300 soundtrack-style tracks (Kevin MacLeod, incompetech.com, CC BY 4.0) that ships with the analyzer, so they mean the same thing in any library: 0.5 is about the top quarter of soundtrack music for that mood, 0.75 the top 10% ("could be heard as this"), and 0.9 the top 3% ("for sure"). Most tracks sit below 0.5 in most moods. Energy is two-sided, so its bar reads 0.5 at the average soundtrack energy. Adding or removing tracks never changes a bar. After you edit a mood's words, the lens row shows **Needs recalibration** until the next analysis run, which recalibrates against the reference set for the current words and clears the marker by itself (this needs the analyzer on); **Recalibrate** just does that run now. The ranking never depends on this mapping, only the number shown.

With an older analyzer that has no reference set, the bars fall back to your library: a calibration taken once, from the first analysis run that covers at least 50 music tracks (until then no moods are suggested). A mood's bar then reads 0.5 half a standard deviation above the library's average for that mood, and Energy reads 0.5 at the library average. Edited words then need a Recalibrate.

Upgrading from the pools release clears the old pools; the next analysis run fills in the profiles.

- **The profile.** One bar per mood, each with five levels (0, 0.25, 0.5, 0.75, 1). A bar at the analyzer's value is **Suggested** and drawn muted. Clicking a bar (or using the arrow keys on a focused bar) sets the nearest level; the bar is then **Set by you**, drawn in the accent colour, and hovering it shows "Set by you · suggested 0.62". **↺** goes back to the suggestion. A level you set always wins over the analyzer's.
- The entry editor has a **Role** row and the mood profile under it. The Library's bulk bar has **Set mood levels…**, which opens the profile for the selected music; a click sets that level on every selected track, and a bar whose tracks differ shows "mixed".
- **Mood ranking.** The Library and Scry mood chips cycle **Off → High → Low**; right-click cycles backwards. High uses the mood score and Low uses `1 − score`. **Match** is their geometric mean: multiply those fits and take the Nth root for N preferences. This rewards a balanced fit across every chosen direction without a hard cutoff. Tracks with missing scores stay, ranked last. Manual levels take priority, with analyzer suggestions breaking ties. Arrows on the chips show the requested direction; hovering Match lists them.
- **Energy bands.** After the mood chips, set a little apart, are three Energy chips: **Low** (below 0.375), **Mid** (0.375 to 0.625) and **High** (above 0.625), using the Energy you set, else the suggested one. So the levels 0 and 0.25 are Low, 0.5 is Mid, 0.75 and 1 are High. Any combination can be on, and a track shows when its Energy is in any of them. With none on there is no Energy filter; with any on, tracks without an Energy value are hidden. The bands only filter; they never change the order.
- Ambiance is hidden while any mood or Energy band is on.
- **Sorting** is done from the column headers only. The Library sorts by status (missing files first), Type, Name, Role, Energy, Duration, and Match while a mood is on; Scry by Name, Energy, Search (how close a track is to the search or seeds) and Match. Click a header to sort by it, again to flip the direction. Turning on the first mood sorts by Match, highest first; turning off the last one goes back to the view's default sort (Name in the Library, Search in Scry). While a mood is on, the Energy header does not sort, so the order never depends on Energy then; use the bands to narrow by Energy. Levels you set are coarse, so tracks set to the same level tie: within a level, Match and Energy order them by the analyzer's suggestion behind the level (the value shown does not change), and any tie left goes by name. The filter bar also has the Role filter.
- Tag chips have three states, **off → include → exclude**, one per click (right-click reverses): an entry passes when it has at least one included tag (if any is included) and none of the excluded ones (red outline, struck through). **Untagged** is a plain on/off filter on top.

#### Mood vocabulary

**Moods** in the left rail opens the mood vocabulary: the moods the analyzer scores and the text prompts it compares each track with.

The current mood editor uses one workflow for every mood: edit **More like / Less like**,
add song **Examples**, choose **Preview**, then **Apply**. Saved and proposed
versions have separate tabs; editing a saved version creates a draft. Wording
search produces independent proposals and never replaces the source editor. Examples express direction and conviction: **Definitely** is a strong yes,
**Understandable** a yes, **Stretch** a tentative no, and **No** a strong no.
They do not request exact values. A fixed 300-recording reference safeguards the
word meaning against broad changes inferred from a handful of selected examples.
One-sided feedback uses a more conservative prior; additional feedback accumulates
without treating the selected examples as the prevalence of a mood in the library.
Agreeing more continues to reduce disagreement;
a score above .9 is never pulled back to .9 merely because of a flag.
**Comparisons** optionally order one song above another. All moods use the same
rules. Earlier scoring versions are labeled when viewed beside a new preview.
Wording search helpers are optional and produce
previews through the same scorer. Scores remain fixed until an update is applied;
changing feedback or importing songs does not retrain them. The smooth mapping
preserves ranking near the endpoints. See [the analyzer's scoring details](tagger/README.md#saved-mood-profiles)
for the exact 512D/1024D feature paths, fitting objective and compatibility rules.

Each mood keeps a stable internal id, so renaming or reordering it preserves its
scores and feedback. The mood actions menu manages names, ordering and removal.
The Saved and Preview tabs show their own descriptions and fit summaries;
**Previous** can restore the last scoring version. The editor and results share
one page scroller. Technical details are collapsed by default. Energy remains a separate
energetic-versus-calm measurement with its own existing editor.

### Scry

**Scry** browses analyzed music by sound.

- **Text search.** A description such as "tense strings building to a battle" returns the closest tracks, ranked by score.
- **More like this.** The action on a Library row opens Scry seeded with that track. Seeds are shown as removable chips, more can be added from the results, and the results are the tracks closest to the seeds together.
- **Filters.** The results can be ranked by mood lens chips and filtered by Energy band and Role, and sorted from the column headers by Search, Name, Energy (while no mood is on) or Match.
- **Map.** A scatter plot of every analyzed music track, colored by role. Hovering shows the track name and clicking plays it. Dragging draws a lasso that adds the enclosed tracks to the selection. Seeds and results are highlighted.
- **Playlists.** The selection can become a playlist with **Create playlist**, which creates a reusable shuffle track holding the selected audio files, optionally added to a board, or be appended to a track with **Add to existing track**.

### Library search

The Library search box also matches the album and artist folder names of an entry, so a game can be found by its soundtrack folder.

### Settings

- **Enable the analyzer.** Turning it on starts analyzing the library right away. New imports are analyzed as they arrive.
- **Analyzer URL** and **Token** identify the service. **Test connection** reports the service's model, whether it has finished loading, and how many embeddings it has cached, or the error.
- **Local path prefix** and **Analyzer path prefix** map the library folder on this machine to the same folder as the analyzer sees it. For example, a library opened here as `Z:\media\audio` that the analyzer sees as `/media/audio` uses local prefix `Z:\media\audio` and analyzer prefix `/media/audio`. Entries outside the local prefix are counted as unreachable.
- **Analyze library** analyzes every entry that has no current analysis, with a progress bar and a summary of the last run. The summary lists up to five entries that failed or were unreachable, with the reason for each.

## Building Soundscapes

### Structure

- **Configuration** — A campaign or session's collection of boards and scenes.
- **Board** — A location or situation tab, such as City, Wilderness, or Battle.
- **Scene** — A saved mix: which tracks play, their volumes, and their starting clips.
- **Track** — A reusable sound layer with playback settings, such as Weather or Music.
- **Clip** — An audio choice in a track, referencing an audio file in the Library.

### Creating a Soundscape

1. Create a board with **+** beside the tabs.
2. Use **All tracks** as a freeform workspace, or create an empty scene with **New scene**.
3. Choose **Add track** to create or reuse a track in the current view. In a scene, new cards start switched off.
4. Toggle tracks and choose clips directly on their cards. Choose **Save mix as scene** to capture the current mix.
5. Tracks use full-width expandable rows with wide volume sliders. Click a track’s name or chevron to expand its editor. **Shared track settings** lists affected scenes; **Make independent copy for this scene** replaces just that scene’s reference with its own definition.
6. Drag the grip at the left of a scene or track to reorder it. Scene track order belongs to that scene; All tracks has its own board order. Move up/down remains available in the action menus.
7. Use **Tracks** in the sidebar to manage tracks even when no scene uses them.

Track names, clips, order, playback mode, gaps, and fades are shared wherever that track
is used. A single shared Music track can hold many songs, with each scene selecting its own clip. Split tracks when you need different playback settings, separate choice lists, or simultaneous layers. Names such as Travel themes or Tavern songs describe reusable collections. The inventory and track pickers show clip names and where each track is used in the current configuration; search also matches that context. Each scene saves its own on/off states, volumes, and clip selections. Duplicate
a track to make an independent variation. Removing a scene or board keeps the tracks. Track actions use **Add to…** for scenes or boards and **Duplicate…** for an independent scene replacement or inventory copy. Existing destinations remain visible as **Already added**. Scene **Duplicate scene…** accepts a name and any destination board.
Removing a clip used by scene selections requires choosing a replacement or disabling
that track in the affected scenes. Audio files stay in the Library.

### Track Modes

- **Seamless** — Choose among multiple alternatives; the chosen clip loops with its Library loop points.
- **One shot** — Play a chosen clip once, useful for cues.
- **Sequence** — Play clips in order, optionally repeating.
- **Shuffle** — Play clips randomly, optionally repeating; random gaps support effects such as varying thunder.

### Transitions (Sequence & Shuffle)

| Transition | Behavior |
|------------|----------|
| **Gapless** | Next clip starts immediately |
| **Gap** | Configurable silence between clips (min/max seconds) |
| **Crossfade** | Clips fade into each other |

Tracks also support **fade in** and **fade out** toggles.

### Playback

- Each scene has its own card layout, including optional cards saved switched off. **All tracks** shows the complete board without changing playback.
- Click a scene’s name to view its cards without changing playback. Its separate **Play** button loads the scene and replaces the mix. Shared tracks already playing the same clip continue without restarting, including pending random gaps.
- Use **Add scene to mix** in the scene menu to keep other tracks playing.
- Shuffle picks a random starting clip when loaded unless a specific starting clip was chosen in scene settings. Continuing an already-playing shared track preserves its current clip and random-gap timing.
- Toggle tracks and choose clips directly from their cards; use **Save changes** to update the loaded scene.
- Use **Stop All / Pause All** in the transport bar and adjust master, track, or clip volume.

Existing libraries migrate automatically from Groups → Boards and Compositions → Scenes.
Track and clip identities, playback settings, ordering, and Library references are retained.
The app backs up the database before upgrading it. Tracks with the same name remain separate.

## Remote Control

LuteDrop runs a local web server so you can control playback from any device on your network (phone, tablet, another computer).

1. The server starts automatically on **port 3333**.
2. In the desktop app, click the **Phone remote** icon at the bottom of the sidebar to
   open a dialog with the pairing link, e.g. `http://<your-computer-ip>:3333/?token=<code>`.
3. Open that link in a browser on any device connected to the same network.
4. The link includes a pairing code required to connect; it's regenerated each time
   LuteDrop starts, so reopen the dialog for a fresh link after a restart.

The remote provides:
- Board tabs, scene loading, and direct track controls
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

Configurations organize boards and scene presets for different campaigns or sessions. The Tracks inventory and audio Library are shared across configurations. Duplicating a configuration copies its scene presets while reusing its tracks.

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

### UI preview (no backend)

```sh
npm run preview:ui   # http://localhost:1440
```

Runs the real frontend in a plain browser with Tauri's IPC mocked (`vite.preview.config.ts`,
`ui-preview/main.ts`), for looking at and screenshotting screens without the Rust side. Commands
answer from `ui-preview/fixtures/`, exported from a real library; flags, levels and accepted Tune
lines change in memory until a reload. `tune_mood` compares any lines through a stub
(`ui-preview/stubTune.ts`: the library's bars nudged by a hash of the lines), and answers Arrange /
Suggest with the recorded analyzer runs for Sorrowful and Horror (suggest), or a stub proposal for
other moods.
There is no audio. It does not affect `npm run build` or the Tauri build.

To refresh the fixtures from your library (the app may stay open):

```sh
python ui-preview/export_fixtures.py             # add --no-tune to skip the analyzer calls
```

The script opens `%APPDATA%\com.lutedrop\lutedrop.db` read-only and works on a backup-API copy,
cuts file paths down to artist / album / file, and leaves out the analyzer token and the
broadcast key. The Tune runs it records are listed in `TUNE_RUNS` at its top.

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


### Read-only prompt experiments

`python scripts/compare-mood-prompts.py --out results.json` compares the saved Battle prompts and the named alternatives in `scripts/mood-prompt-experiments.json` using the live analyzer. It reads the app DB and credentials without printing credentials, uses cached embeddings, and never changes saved vocabulary, calibration, flags, or scores. The report includes ranked tracks, overlap/rank correlation with Heroic, and ranks of existing Battle flags. Edit the candidate JSON to try other wording or another mood. Lower Heroic overlap alone does not make a better mood; review the tracks and flags too. Raw scores from different prompt sets are not calibrated bars.

`python scripts/tag-game-osts.py` previews a curated, exact-album OST tagging pass using embedded metadata (requires `mutagen`). With LuteDrop closed, `--apply` first backs up the database, then adds `game ost` and `ost: <series/game>` tags in one verified transaction. Existing tags and audio files are preserved; reruns do not duplicate memberships. Unknown albums are left for review.


`python scripts/mood-spread-study.py --out <report.json>` runs a read-only library-wide vocabulary study. The candidate words/ordinary-language definitions and semantic synonym sets live in `scripts/mood-spread-words.json` and `scripts/mood-spread-phrases.json`. It tests each bare word, “X music,” the definition, and (where supplied) a synonym set, using the analyzer's frozen reference statistics to put all candidates on the same scale. Paired fit is the lower of the musical-name and semantic-anchor scores: a conservative diagnostic for wording sensitivity, not the app's combined mood ranking. Reports include every score, top tracks, rank agreement, overlap with existing moods, tracks below the strong band, and tracks absent from every current top-20 list. No names or album metadata enter the scoring request; it uses cached audio hashes only. No live vocabulary or library data is edited.


`python scripts/refine-mood-sets.py --out <current-set-tests.json>` snapshots the **current** app vocabulary and flags, scores line/subword ablations and curated complete alternatives from `scripts/mood-set-alternatives.json`, then sends final comparisons to the live `/tune` endpoint. It also requests arrangements from each mood's own semantic word pool. The objective includes stability, so a flag-perfect current set often wins even when another set would offer a different useful profile. No proposal is accepted automatically.

`python scripts/test-qualifier-sets.py --baseline <current-set-tests.json> --out <qualifiers.json>` tests additions from `scripts/qualifier-set-alternatives.json` against that same captured baseline. It sends three complete formulations per qualifier plus their actual three-line combination through `/score`. Combined scores use the app's real mean-embedding scoring and reference calibration, not an invented maximum or a renamed single-word score. The per-form range and agreement remain available to spot wording-sensitive results. Character descriptors such as Atmospheric or Choral are identified separately from feelings in the report; nothing is added to the app by running the study.


For a focused follow-up, pass `--sets scripts/battle-identity-sets.json` to `scripts/test-qualifier-sets.py`. This tests alternative battle identities (Ferocious, Defiant, Menacing, and others) across the same frozen library. Inspect each target song's independent-form range as well as the combined bar: a qualifier can look stable library-wide while being a one-phrase match for a particular song. Keep broad Combat/Battle usefulness separate from more specific expressive identity, and avoid promoting a higher score alone into a label.


`python scripts/audit-mood-vocabulary.py --base <battle-tests.json> --extra <qualifier-tests.json> --out <audit.json>` evaluates the complete candidate vocabulary jointly using the already-scored full library. It validates that both source studies share identical baseline scores/vocabulary and reference data, counts consistent support across albums and game/series tags, measures ranking correlation and high-match Jaccard overlap, and checks key duplicate pairs after leaving out each source group. It enumerates small one-to-three-addition trade-offs with explicit breadth and redundancy rules. Positive coactivation is treated separately from opposite rankings; semantic nesting and known vocal-proxy cases remain review constraints. These are diagnostic rules, not automatic edits or proof of emotional meaning.
