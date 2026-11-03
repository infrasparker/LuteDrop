// LuteDrop Stream: plays the LuteDrop broadcast URL on every Foundry client.

export const MODULE_ID = "lutedrop-stream";

function registerSettings() {
  // World scope: one value for the whole game, and only the GM can edit it.
  game.settings.register(MODULE_ID, "streamUrl", {
    name: "LUTEDROP.StreamUrl.Name",
    hint: "LUTEDROP.StreamUrl.Hint",
    scope: "world",
    config: true,
    type: String,
    default: "",
  });

  // Client scope: stored in this browser only, so each person chooses for themselves.
  game.settings.register(MODULE_ID, "listen", {
    name: "LUTEDROP.Listen.Name",
    hint: "LUTEDROP.Listen.Hint",
    scope: "client",
    config: true,
    type: Boolean,
    default: true,
  });

  game.settings.register(MODULE_ID, "volume", {
    name: "LUTEDROP.Volume.Name",
    hint: "LUTEDROP.Volume.Hint",
    scope: "client",
    config: true,
    type: Number,
    default: 0.8,
    range: { min: 0, max: 1, step: 0.05 },
  });
}

Hooks.once("init", registerSettings);
