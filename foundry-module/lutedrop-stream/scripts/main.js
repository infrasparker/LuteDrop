// LuteDrop Stream: plays the LuteDrop broadcast URL on every Foundry client.

// MODULE_ID is defined in player.js (which must not import this entry file)
// and re-exported here so existing imports of it from main.js keep working.
import { MODULE_ID, createPlayer, ElementPlayer } from "./player.js";

export { MODULE_ID };

let player = null;

function registerSettings() {
  // World scope: one value for the whole game, and only the GM can edit it.
  game.settings.register(MODULE_ID, "streamUrl", {
    name: "LUTEDROP.StreamUrl.Name",
    hint: "LUTEDROP.StreamUrl.Hint",
    scope: "world",
    config: true,
    type: String,
    default: "",
    onChange: (value) => player?.setUrl(value),
  });

  // Client scope: stored in this browser only, so each person chooses for themselves.
  game.settings.register(MODULE_ID, "listen", {
    name: "LUTEDROP.Listen.Name",
    hint: "LUTEDROP.Listen.Hint",
    scope: "client",
    config: true,
    type: Boolean,
    default: true,
    onChange: (on) => {
      player?.setListening(on);
      refreshSliderRows();
    },
  });

  game.settings.register(MODULE_ID, "volume", {
    name: "LUTEDROP.Volume.Name",
    hint: "LUTEDROP.Volume.Hint",
    scope: "client",
    config: true,
    type: Number,
    default: 0.8,
    range: { min: 0, max: 1, step: 0.05 },
    onChange: (v) => {
      player?.setVolume(v);
      refreshSliderRows();
    },
  });
}

Hooks.once("init", registerSettings);

/** Give a player the current URL / listen / volume settings. */
function applySettings(target) {
  target.setUrl(game.settings.get(MODULE_ID, "streamUrl"));
  target.setListening(game.settings.get(MODULE_ID, "listen"));
  target.setVolume(game.settings.get(MODULE_ID, "volume"));
}

/**
 * The WebCodecs player could never reach the stream (see ReconnectPolicy in
 * player.js), so use the <audio> player for the rest of this session. Settings
 * handlers call `player?.setX(...)`, so swapping the variable is enough.
 */
function switchToElementPlayer() {
  const fallback = new ElementPlayer();
  applySettings(fallback);
  fallback.start();
  player = fallback;
  console.info(
    `${MODULE_ID} | falling back to the <audio> player (stream server sent no CORS header, or network down)`,
  );
}

Hooks.once("ready", async () => {
  // game.audio.unlock is a Promise (not a function) that resolves on the user's first click/key.
  await game.audio.unlock;
  // Reuse Foundry's AudioContext when it has one (already unlocked), else make our own.
  const ctx = game.audio?.context ?? new AudioContext();
  player = await createPlayer({ audioContext: ctx, onGiveUp: switchToElementPlayer });
  applySettings(player);
  player.start();
});

// ---- Playlists sidebar slider ----

const ROW_CLASS = "lutedrop-volume";

// Ears hear loudness on a roughly logarithmic scale, so Foundry's own volume
// sliders pass the slider position through a curve (position ^ 1.5) before
// using it as the volume; a straight 1:1 slider puts nearly all the audible
// change in its bottom third. We use the same curve so our row feels like the
// Music / Environment / Interface rows beside it. The saved "volume" setting
// holds the real volume (as Foundry's do); only the slider position is curved.
// AudioHelper lives at foundry.audio.AudioHelper (v12+; v12 also has a global
// AudioHelper). If neither is found, fall back to a straight 1:1 slider.
function audioHelper() {
  return globalThis.foundry?.audio?.AudioHelper ?? globalThis.AudioHelper;
}

function sliderToVolume(position) {
  const helper = audioHelper();
  return typeof helper?.inputToVolume === "function" ? helper.inputToVolume(position) : position;
}

function volumeToSlider(volume) {
  const helper = audioHelper();
  return typeof helper?.volumeToInput === "function" ? helper.volumeToInput(volume) : volume;
}

function buildSliderRow() {
  const label = game.i18n.localize("LUTEDROP.SliderLabel");
  const row = document.createElement("li");
  // "flexrow" and "volume-icon" are Foundry's own classes, so the row picks up their styling.
  row.className = `${ROW_CLASS} flexrow`;
  row.innerHTML = `
    <label>${label}</label>
    <button type="button" class="volume-icon fa-fw fa-solid" aria-label="${label}"></button>
    <input type="range" min="0" max="1" step="0.05" aria-label="${label}">`;

  const slider = row.querySelector("input");
  // Drag: change the live audio at once; save the setting only on release.
  slider.addEventListener("input", () => player?.setVolume(sliderToVolume(slider.valueAsNumber)));
  slider.addEventListener("change", () =>
    game.settings.set(MODULE_ID, "volume", sliderToVolume(slider.valueAsNumber)),
  );
  row.querySelector("button").addEventListener("click", () => {
    game.settings.set(MODULE_ID, "listen", !game.settings.get(MODULE_ID, "listen"));
  });
  return row;
}

/** Make a row show the current settings (slider position and mute icon). */
function syncSliderRow(row) {
  row.querySelector("input").value = volumeToSlider(game.settings.get(MODULE_ID, "volume"));
  const listening = game.settings.get(MODULE_ID, "listen");
  // Foundry dims .volume-icon.fa-volume-xmark, which suits a muted state.
  row.querySelector("button").classList.toggle("fa-volume-xmark", !listening);
  row.querySelector("button").classList.toggle("fa-volume-low", listening);
}

function refreshSliderRows() {
  document.querySelectorAll(`.${ROW_CLASS}`).forEach(syncSliderRow);
}

// v14 passes (application, HTMLElement); v12 passes a jQuery object, so unwrap it.
Hooks.on("renderPlaylistDirectory", (_app, element) => {
  const root = element instanceof HTMLElement ? element : element?.[0];
  if (!root) return;
  // ".global-volume ol" holds the Music / Environment / Interface <li> rows.
  const list = root.querySelector(".global-volume ol");
  if (!list) return;
  // This hook fires on every re-render, so only add the row once.
  if (list.querySelector(`.${ROW_CLASS}`)) return;
  const row = buildSliderRow();
  syncSliderRow(row);
  list.append(row);
});
