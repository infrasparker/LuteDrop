# Waveform Playhead Design

## Goal

Add a live playback position indicator to the waveform editor, move loop marker handles to the top, and tighten marker grab precision.

## Changes

All changes are in `src/components/WaveformEditor.vue`. No Rust changes needed — `TrackPlaybackState.currentTime` is already emitted every 50ms.

### 1. Live playhead

- Read `currentTime` and `isPlaying`/`isPaused` from the Pinia `playbackStates` map using `trackId` prop.
- Draw a white (`#E8E4DC`) vertical line at the corresponding x position, with a small filled triangle at the top (5px wide) to distinguish from loop markers.
- Animate with `requestAnimationFrame` loop that calls `drawWaveform()` while playing. Cancel the loop when stopped/paused or component unmounts.
- Only visible when the track is playing or paused (not when stopped / no state).

### 2. Loop marker handles to top

- Move the half-box grab handles from `cssH - 8` (bottom) to `y = 0` (top).
- Loop start handle: `fillRect(loopStartX - 5, 0, 5, 8)` (left of line).
- Loop end handle: `fillRect(loopEndX, 0, 5, 8)` (right of line).

### 3. Tighter marker grab threshold

- Reduce `MARKER_GRAB_THRESHOLD_PX` from 12 to 7.
