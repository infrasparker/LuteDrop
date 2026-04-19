# LuteDrop

A soundscape and ambient audio manager for tabletop RPGs, live performances, and immersive environments. Build layered soundscapes with multiple tracks, organize your audio library, and control everything from your phone.

## Getting Started

### Install

Download and run the installer from the [Releases](../../releases) page.

### Import Audio

1. Go to **Library** (sidebar).
2. Click **Import Files** or **Import Folder** to add audio files (mp3, wav, ogg, flac, aac, m4a, wma).
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
2. Open a browser on any device connected to the same network.
3. Navigate to `http://<your-computer-ip>:3333`.
4. You can find your computer's IP by running `ipconfig` in a terminal.

The remote provides:
- Group tabs and composition selection
- Play/pause/stop for tracks and compositions
- Master, track, and clip volume control
- Real-time status updates via WebSocket

## Configurations

Configurations save your entire soundstage state (groups, compositions, tracks, clips, and settings). Use the configuration picker in the top bar to switch between saved configurations — useful for different campaigns or sessions.

## Shortcuts

Use the transport bar at the bottom of the screen for global controls:
- **Pause/Resume All** — Pause or resume every playing track
- **Stop All** — Stop all playback
