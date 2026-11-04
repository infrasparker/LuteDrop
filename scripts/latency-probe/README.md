# Latency probe

Developer tool (not shipped) that measures how long a sound takes to get from
"play pressed" to "heard by a Broadcast listener", for several kinds of listener,
and checks the decodeAudioData player's output against a continuous decoder.
Use it for before/after numbers when changing the stream player.

## Run it

1. Start LuteDrop as a dev build and turn Broadcast on.
2. Get the phone-remote pairing token from LuteDrop's log or its Remote dialog.
3. Start the probe, using your own listen key:
   `node scripts/latency-probe/serve.mjs --stream http://127.0.0.1:5883/listen/<key>`
4. Open the printed URL in a Chromium browser (needs WebCodecs `AudioDecoder`).
5. Put the token in the WebSocket URL field (`ws://127.0.0.1:3333/ws?token=<token>`)
   and click **Connect WS**. The track list fills in (tracks already playing are hidden).
6. Pick a short sound that starts loudly, then click **Run trials**.

Each trial stops the track, waits 1.5 s of silence, presses play, and notes when each
listener first hears sound, then stops and waits 2 s. A listener that has not heard
anything after the **Give-up (s)** time (default 8) shows `n/a`. The plain `<audio>`
element can need 10-30 s to start playing a live stream on first use, so raise
Give-up (e.g. 30) when measuring it.

## Trigger mode

- **stop/play** (default): each trial stops the track and plays it from the start. This
  shows the full press-to-hear time, including LuteDrop opening and decoding the file
  (slow on a network drive) and any quiet intro in the track.
- **pause/resume**: before trial 1 the track plays for **Warm-up (s)** (default 8) so it
  is past its intro, then is paused. Each trial resumes it (`play_track` resumes a paused
  track) and pauses again afterwards; the track is stopped at the end. This gives a hard
  onset with no file-open time, so use it for pipeline numbers. Pick a track that is loud
  at the warm-up point.

The browser console also logs how long each listener's start-up took.

## Columns

All times are milliseconds after T0, the moment the page sends `play_track`.

- **arrival**: first decoded audio above the RMS threshold, read straight off the
  network and decoded at once. This is the earliest any listener could hear it.
- **mixing**: LuteDrop reports the track playing with a position above zero.
- **element**: a plain `<audio>` element on the stream (today's Foundry behaviour).
- **webcodecs**: the Foundry module's `WebCodecsPlayer`, loaded from
  `foundry-module/lutedrop-stream/scripts/player.js` and watched by the same detector.
- **held**: the module's `<audio>` fallback (`ElementPlayer`) with its live-edge hold:
  it seeks back to the live edge, or plays at 1.15x until caught up when the seek is
  ignored. This is what players reaching Foundry over plain `http://` hear (WebCodecs
  is unavailable there). The browser console shows `lutedrop-stream | ...` lines when
  it seeks, speeds up, returns to normal speed, or reconnects.
- **decodeaudiodata**: the module's `DecodeAudioDataPlayer`, the low-latency player for
  pages without WebCodecs (Foundry over plain `http://` in Chrome, Edge, Firefox). It
  wraps each network read's Opus packets in a tiny Ogg Opus file, decodes it with
  `decodeAudioData`, and schedules the result with the same 0.2 s cushion as the
  WebCodecs player, so expect numbers close to `webcodecs`.
- `element-arrival`, `webcodecs-arrival`, `held-arrival`, `decodeaudiodata-arrival`: extra
  delay a player adds over the bare minimum. This is the number to compare before and after.

## Continuity check

**Continuity check (10 s)** checks that the decodeAudioData player produces the same
samples as a decoder that runs continuously. Each tiny file starts a fresh decoder, so
the player puts the previous `PREROLL_PACKETS` packets in front to warm it up; if that
were too little, every chunk would come out slightly wrong and the joins would click.

Run it with the probe server pointed at the live stream, and play a track in LuteDrop
while it runs (silence compares equal trivially; the status line says so if the signal
was nearly silent). It reads `/stream` for 10 s and decodes every packet twice: once
through one WebCodecs `AudioDecoder` (the reference) and once per network read through
`OggOpusMuxer` + `decodeAudioData` with pre-roll, exactly as the player does. Both are
lined up by packet index and compared on both channels. The status line and the console
show: the number of chunks, max |difference| over all samples, RMS of the difference,
the worst difference within the first or last 48 samples of any chunk (the joins), how
many joins are above 1e-3, the worst difference away from the joins, and the signal RMS.

Acceptance: **PASS** when max |diff| < 1e-3 everywhere and no join is above 1e-3 (no
audible tick). **FAIL** when anything is above 1e-2. In between is **BORDERLINE**.
Measured in Chrome on an encoded test signal: 4 packets of pre-roll gave 0.14 (fail),
8 gave 0.011, 12 gave 0.0007, 16 (the value used) gave 0.00004. Needs WebCodecs, so
open the probe on `127.0.0.1` / `localhost`.

## Background tabs

Sound is detected in network and audio callbacks (the `<audio>` element is watched by an
AudioWorklet on the audio thread), so the tab need not be visible. Hidden tabs throttle
timers to about once a second; that only makes the page's waits (the quiet check, the
give-up) coarser, not the recorded times. Keeping the tab visible is still the safest.

## Testing the fallback

Two `serve.mjs` switches exercise the WebCodecs player's give-up path:

- `--fail`: `/stream` answers 404. On the probe's own origin the page may read that
  404, so the WebCodecs player keeps retrying with growing delays (1, 2, 4, 8, 15 s...)
  and does NOT give up. That is how LuteDrop's own 404 for a wrong key behaves, since it
  carries the CORS header. It is *not* what a Foundry client sees behind Caddy while
  LuteDrop is not broadcasting: that 502 comes from another origin without the CORS
  header, so the browser refuses it like `--no-cors` below, and it does count toward
  giving up. The other consumers fail too, so watch the console.
- `--no-cors`: `/stream` is sent without `Access-Control-Allow-Origin`. The probe page
  itself is same-origin, so point the webcodecs consumer at another origin:
  open `http://127.0.0.1:8790/?webcodecs=http://localhost:8790/stream`
  (`?decodeaudiodata=<url>` does the same for the decodeaudiodata consumer). The browser
  refuses each fetch; after 3 refusals (about 3 s) the console shows
  `WebCodecs player giving up`. This cross-origin `?webcodecs=` recipe is the realistic
  one for "joined before Broadcast was on" as well: with `--fail` instead of
  `--no-cors`, the same cross-origin URL gets a 404 without the header, which the
  browser refuses in exactly the same way.

## Caveat

T0 includes LuteDrop's own time from receiving the command to mixing the sound, so
`arrival-T0` is never zero. Compare the `-arrival` columns for player overhead.
