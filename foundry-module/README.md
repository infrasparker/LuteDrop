# LuteDrop Stream (Foundry VTT module)

Plays the LuteDrop broadcast stream on every connected Foundry client. The GM
sets the stream URL once; each player can turn the stream off or change its
volume for themselves.

The module only plays a URL. It never talks to LuteDrop's control API.

## Install

1. Copy the `lutedrop-stream` folder (the one containing `module.json`) into
   Foundry's `Data/modules` folder, so you end up with
   `Data/modules/lutedrop-stream/module.json`.
2. Restart Foundry, open your world, then Settings > Manage Modules and enable
   "LuteDrop Stream".

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

## Mixed content

Browsers block insecure content on secure pages. A Foundry served over
`http://` can load an `https://` stream, but a Foundry served over `https://`
cannot load an `http://` stream. Use the `https://` public URL.
