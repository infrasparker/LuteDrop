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

Auto-tagging suggests a role and tags for music entries by comparing how they sound with entries the user has already classified by hand. For each entry it scores the Combat probability, which sets the role (Combat or Non-combat), and scores the entry against each tag in the vocabulary. A role or tag that passes its threshold is applied automatically.

- The role is Combat when the probability reaches the Combat threshold, Non-combat when one minus the probability reaches the Non-combat threshold, and otherwise left unset. Its examples are entries whose role was set by hand. A role set or cleared by hand is never changed by auto-tagging.
- Auto tags are marked on their chip. Each one offers **Keep** (it becomes a normal tag) or **Remove** (it is taken off and not suggested again for that entry). Auto-tagging never overwrites or removes a tag applied by hand, and removing an auto tag stops it being applied to that entry again.
- A role or tag is applied only once the analyzer can predict it reliably. A tag's positive examples are music entries that carry it; its negative examples are music entries with a hand-set role but without the tag, plus entries where it was removed as an auto tag. It needs at least 5 of each, and the tag must also separate them well enough when tested on these examples. Auto roles and auto tags never count as examples. The tag quality table in the dialog shows each tag's example counts, AUC and, for tags not yet trusted, what is missing.
- The library's tag filter can show only entries with auto tags. Bulk actions can keep or remove the auto tags on several entries at once.
- In the entry editor, the **Tag scores** panel shows each tag's score against its threshold and whether the entry's tag was applied by hand or by auto-tagging.

Audio is analyzed by a separate service that needs a GPU host; see [`tagger/README.md`](tagger/README.md) for how to run it. The **Auto-tagging** button in the Library toolbar opens the settings:

- **Enable auto-tagging.** Turning it on starts analyzing the library right away. New imports are analyzed as they arrive.
- **Analyzer URL** and **Token** identify the service. **Test connection** reports the service's model, whether it has finished loading, and how many embeddings it has cached, or the error.
- **Local path prefix** and **Analyzer path prefix** map the library folder on this machine to the same folder as the analyzer sees it. For example, a library shared from the analyzer host and opened here as `Z:\media\audio` uses local prefix `Z:\media\audio` and analyzer prefix `/media/audio`, matching the compose mount `/srv/media/audio:/media/audio:ro`. Entries outside the local prefix are counted as unreachable.
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
