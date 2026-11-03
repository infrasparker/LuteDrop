# LuteDrop

A soundscape and ambient audio manager for tabletop RPGs, live performances, and immersive environments. Build layered soundscapes with multiple tracks, organize your audio library, and control everything from your phone.

## Getting Started

### Install

Grab the latest build from the [Releases](../../releases) page. Each release ships in two
forms:

- **Installer** — `LuteDrop_<version>_x64-setup.exe` (NSIS) or `LuteDrop_<version>_x64_en-US.msi`.
  Installs to Program Files with Start-menu shortcuts.
- **Portable** — `lutedrop.7z` (recommended) or the bare `lutedrop.exe`. No install; unzip and
  run `lutedrop.exe`. Use the **`.7z`**, not the bare exe, if you want the phone remote: the
  archive includes the `dist-remote/` folder the exe needs beside it. The bare `lutedrop.exe`
  runs the desktop app fine on its own.

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
   - **Same Wi-Fi** - `http://<your-computer-ip>:3334/listen/<key>`, built from your
     computer's local address. Works for anyone on your network with no extra setup.
   - **Players on the internet** - your **Public address** plus `/listen/<key>`. This
     stays empty until you fill in the Public address field (see below).
3. Add `/page` to either link (`.../listen/<key>/page`) for a bare listening page with a
   play button, handy for a phone browser. The Foundry module plays the raw link instead.
4. The badge in the transport bar and the dialog show how many people are listening.

The stream is served on **port 3334** by default. You can change the port in the dialog,
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
	reverse_proxy 192.168.86.42:3334 {
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
- The first time Broadcast binds port 3334, Windows Firewall may ask whether to allow
  LuteDrop (dev builds ask again from each new exe path); allow it, because dismissing
  the prompt blocks other machines, so Caddy returns `502`.
- To play the stream inside Foundry VTT for every player, see
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

Bump the version in `src-tauri/tauri.conf.json`, `package.json`, and `src-tauri/Cargo.toml`
before building a new release.

### Portable build

For a no-install **portable** download, package the standalone binary with the remote web
assets it expects beside it:

```sh
mkdir portable
cp src-tauri/target/release/lutedrop.exe portable/
cp -r dist-remote portable/dist-remote      # so the phone remote works
cd portable && 7z a -mx=9 lutedrop.7z lutedrop.exe dist-remote
```

`lutedrop.exe` runs the desktop app by itself; the `dist-remote/` folder beside it enables the
port-3333 phone remote. Attach `lutedrop.7z` (and optionally the bare `lutedrop.exe`) to the
GitHub [Release](../../releases) alongside the installers.

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
