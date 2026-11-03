// LuteDrop Stream: plays the LuteDrop broadcast URL on every Foundry client.

export const MODULE_ID = "lutedrop-stream";

const BACKOFF_START_MS = 1000;
const BACKOFF_MAX_MS = 15000;
// Playback must run this long without a failure before we trust the connection again.
const STABLE_AFTER_MS = 10000;

/**
 * Keeps one HTMLAudioElement playing the stream and reconnects when it drops.
 *
 * A plain <audio> element (not Web Audio) is used on purpose: it needs no CORS
 * headers from the stream server. "Listen off" sets volume to 0 instead of
 * pausing, so the stream stays live and un-muting resumes at the current position.
 */
class StreamPlayer {
  #audio = new Audio();
  #url = "";
  #listening = true;
  #volume = 0.8;
  #started = false;
  #backoffMs = BACKOFF_START_MS;
  #retryTimer = null; // at most one pending reconnect at any time
  #stableTimer = null;

  constructor() {
    this.#audio.preload = "none";
    // Do not set crossOrigin: that would require CORS headers on the stream.
    for (const name of ["error", "stalled", "ended"]) {
      this.#audio.addEventListener(name, () => this.#reconnect());
    }
    this.#audio.addEventListener("playing", () => this.#watchForStability());
  }

  /** Begin playback. Call only after Foundry's audio-unlock gesture. */
  start() {
    this.#started = true;
    this.#applyVolume();
    this.#connect();
  }

  setUrl(url) {
    this.#url = String(url ?? "").trim();
    this.#backoffMs = BACKOFF_START_MS;
    if (this.#started) this.#connect();
  }

  setListening(on) {
    this.#listening = Boolean(on);
    this.#applyVolume();
  }

  setVolume(v) {
    this.#volume = Math.min(1, Math.max(0, Number(v) || 0));
    this.#applyVolume();
  }

  #applyVolume() {
    this.#audio.volume = this.#listening ? this.#volume : 0;
  }

  #clearTimers() {
    clearTimeout(this.#retryTimer);
    clearTimeout(this.#stableTimer);
    this.#retryTimer = null;
    this.#stableTimer = null;
  }

  /** Point the element at the stream (fresh connection) and play. */
  #connect() {
    this.#clearTimers();
    if (!this.#url) {
      this.#stop();
      return;
    }
    // The throwaway query value stops the browser reusing a dead cached response.
    const separator = this.#url.includes("?") ? "&" : "?";
    this.#audio.src = `${this.#url}${separator}t=${Date.now()}`;
    this.#audio.play().catch((err) => this.#onPlayRejected(err));
  }

  #stop() {
    this.#audio.src = "";
    this.#audio.pause();
    console.info(`${MODULE_ID} | no stream URL set; staying silent`);
  }

  #onPlayRejected(err) {
    // AbortError: a newer connect() replaced this one, which is expected.
    if (err?.name === "AbortError") return;
    console.warn(`${MODULE_ID} | play() rejected`, err);
    // NotAllowedError means the browser still blocks autoplay; retrying cannot help.
    if (err?.name === "NotAllowedError") return;
    this.#reconnect();
  }

  /** Retry after the current backoff delay, doubling it up to the cap. */
  #reconnect() {
    clearTimeout(this.#stableTimer);
    this.#stableTimer = null;
    // Blank URL (an empty src also raises "error") or already waiting: do nothing.
    if (!this.#started || !this.#url || this.#retryTimer !== null) return;
    this.#retryTimer = setTimeout(() => this.#connect(), this.#backoffMs);
    this.#backoffMs = Math.min(this.#backoffMs * 2, BACKOFF_MAX_MS);
  }

  /** After STABLE_AFTER_MS of uninterrupted playback, forget earlier failures. */
  #watchForStability() {
    if (this.#stableTimer !== null) return;
    this.#stableTimer = setTimeout(() => {
      this.#stableTimer = null;
      this.#backoffMs = BACKOFF_START_MS;
    }, STABLE_AFTER_MS);
  }
}

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
    onChange: (on) => player?.setListening(on),
  });

  game.settings.register(MODULE_ID, "volume", {
    name: "LUTEDROP.Volume.Name",
    hint: "LUTEDROP.Volume.Hint",
    scope: "client",
    config: true,
    type: Number,
    default: 0.8,
    range: { min: 0, max: 1, step: 0.05 },
    onChange: (v) => player?.setVolume(v),
  });
}

Hooks.once("init", registerSettings);

Hooks.once("ready", async () => {
  player = new StreamPlayer();
  player.setUrl(game.settings.get(MODULE_ID, "streamUrl"));
  player.setListening(game.settings.get(MODULE_ID, "listen"));
  player.setVolume(game.settings.get(MODULE_ID, "volume"));
  // game.audio.unlock is a Promise (not a function) that resolves on the user's first click/key.
  await game.audio.unlock;
  player.start();
});
