# Latency probe

Developer tool (not shipped) that measures how long a sound takes to get from
"play pressed" to "heard by a Broadcast listener", for several kinds of listener.
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
listener first hears sound (8 s timeout), then stops and waits 2 s.

## Columns

All times are milliseconds after T0, the moment the page sends `play_track`.

- **arrival**: first decoded audio above the RMS threshold, read straight off the
  network and decoded at once. This is the earliest any listener could hear it.
- **mixing**: LuteDrop reports the track playing with a position above zero.
- **element**: a plain `<audio>` element on the stream (today's Foundry behaviour).
- **webcodecs**: the new player (appears once that consumer is added).
- `element-arrival`, `webcodecs-arrival`: extra delay a player adds over the bare minimum.
  This is the number to compare before and after.

## Caveat

T0 includes LuteDrop's own time from receiving the command to mixing the sound, so
`arrival-T0` is never zero. Compare the `-arrival` columns for player overhead.
