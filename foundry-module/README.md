# LuteDrop Stream (Foundry VTT module)

Plays the LuteDrop broadcast stream on every connected Foundry client. The GM
sets the stream URL once; each player can turn the stream off or change its
volume for themselves.

The module only plays a URL. It never talks to LuteDrop's control API.

## Install

**Option 1: manifest URL (recommended).** In Foundry, open Add-on Modules, click
Install Module, paste this into the "Manifest URL" box at the bottom, and click Install:

```
https://github.com/infrasparker/LuteDrop/releases/latest/download/module.json
```

Foundry downloads and unpacks the module for you, and later updates show up in its
module list.

**Option 2: copy the folder** (if you have the repo). Copy the `lutedrop-stream` folder
(the one containing `module.json`) into Foundry's `Data/modules` folder, so you end up
with `Data/modules/lutedrop-stream/module.json`.

Either way, restart Foundry if it was running, open your world, then Settings > Manage
Modules and enable "LuteDrop Stream".

## Set up

1. In LuteDrop, switch Broadcast on and copy the public stream URL from the
   Broadcast dialog.
2. In Foundry, open Settings > Configure Settings > Module Settings and paste it
   into "LuteDrop stream URL" (GM only). Leave it blank to stop playback.

Playback starts after Foundry's usual "click to enable audio" gesture. If the
stream drops (for example LuteDrop restarts), the module reconnects by itself.

## Controls

- **Playlists sidebar:** a "LuteDrop" slider sits under Foundry's own Music,
  Environment and Interface sliders. The speaker icon next to it mutes or
  unmutes the stream on this device.
- The same "Listen to LuteDrop" and "LuteDrop volume" options are in Module
  Settings. They are per device, so the GM can mute the stream locally without
  affecting players.

## Latency

Expect to hear a change about a quarter of a second after LuteDrop makes it (measured
with LuteDrop and the browser on one PC). Over the internet add a little for network
jitter.

Most of that is a cushion of audio the player keeps in hand so a late network packet
does not cause a gap. It starts at 0.2 s, grows by 0.1 s each time the stream runs dry
(up to 0.6 s on a bad link), and jumps forward if it ever falls more than 0.4 s behind.
The cushion shrinks back by 0.1 s after each quiet minute without a gap, so one bad
moment (such as a slow scene load) does not leave the stream late for the whole night.

### How players should join

**Recommended: the `https://` address**, e.g. `https://foundry.irslandia.duckdns.org`
(Foundry behind a reverse proxy such as Caddy, with Foundry's `proxySSL` and `proxyPort`
options set). An `https://` page gets the fastest player, which decodes with the
browser's `AudioDecoder`. That only exists on `https://` pages or on `localhost` /
`127.0.0.1`. Browsers that have it: Chrome and Edge 94+, recent Safari (the module
checks for Opus support at start-up and falls back otherwise), Firefox 130+ on desktop,
and the Foundry desktop app.

The plain `http://<ip>:5884` address still works. There Chrome, Edge and Firefox use a
second low-latency player (built on `decodeAudioData`, nearly as fast), and a browser
that cannot do that either gets the `<audio>` player, which is held close to live but
still runs a second or more behind. Nobody has to choose: the module picks the best
player the browser supports, and falls back automatically.

To see which player a client got, open the browser console (F12) and look for one of
`lutedrop-stream | using the low-latency WebCodecs player`,
`lutedrop-stream | WebCodecs Opus unavailable; using the low-latency decodeAudioData player` or
`lutedrop-stream | WebCodecs Opus and decodeAudioData Ogg Opus unavailable; using the <audio> player`.

### When the fast player falls back

If the stream server never sends a byte and the browser refuses three connections, the
client switches to the `<audio>` player and logs
`lutedrop-stream | falling back to the <audio> player (stream server sent no CORS header, or network down)`.
The browser refuses any reply from another address that lacks a CORS header, whatever
its status. So besides a LuteDrop older than this change (it sends no CORS header), a
player who joins before the GM switches Broadcast on also gets there: Caddy's own 502
"LuteDrop is not running" page carries no such header. A LAN stream URL while LuteDrop
is closed counts too (the connection is refused outright).

That is why the fallback only lasts *until the stream becomes reachable*: each time the
`<audio>` player starts playing, the module tries one direct read of the stream, and as
soon as that works (the stream is live) it switches back to the fast player and logs
`lutedrop-stream | stream reachable again with CORS; upgrading to the WebCodecs player`
(or `... decodeAudioData player` on plain `http://`).

One fallback is for the whole session: if the decodeAudioData player's first three
decodes all fail, the browser cannot decode the stream that way, so it logs
`lutedrop-stream | decodeAudioData player giving up: 3 decodes failed in a row and none ever succeeded`
and the `<audio>` player stays. Reloading the page starts over.

## Mixed content

Browsers block insecure content on secure pages. A Foundry served over
`http://` can load an `https://` stream, but a Foundry served over `https://`
cannot load an `http://` stream. Use the `https://` public URL.
