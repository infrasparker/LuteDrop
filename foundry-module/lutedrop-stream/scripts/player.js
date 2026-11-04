// Low-latency stream player for the LuteDrop Foundry module.
//
// This file holds the pieces that turn LuteDrop's live broadcast into sound.
// It is plain browser JavaScript (no bundler, no dependencies) so Foundry can
// load it directly, and so the pure parts can be tested under Node.
//
// First piece: OggDemuxer.
//
// What an Ogg page is: the broadcast is an Ogg stream -- a container that
// chops a stream of codec packets (here, Opus audio) into self-delimiting
// chunks called pages. Each page starts with the capture pattern "OggS", then
// a header with a granule position (a timestamp), a serial number, a page
// sequence number, a CRC checksum, and a "lacing table" listing how long the
// pieces of the page body are. The first page of the live stream carries the
// Opus setup packet ("OpusHead"), the second carries the tags ("OpusTags"),
// and every page after that carries audio.
//
// Why packets can straddle pages: the lacing table describes the body in
// segments of at most 255 bytes. A segment of exactly 255 means "this packet
// keeps going in the next segment"; any smaller value (0 included) means "the
// packet ends here". So a long packet is a run of 255s followed by a smaller
// value. If a page runs out of segments while a packet is still open (its last
// lacing value is 255), the packet continues at the start of the next page,
// which is flagged "continued". The demuxer therefore has to carry the
// unfinished bytes over, and only hand out packets that are complete.
//
// Why we check the CRC: a listener can join mid-stream or lose bytes, and a
// stray "OggS" can appear inside audio data by chance. A page whose CRC does
// not match is not trusted; we skip one byte and search for the next real page
// rather than feed corrupt audio to the decoder.

const CAPTURE = [0x4f, 0x67, 0x67, 0x53]; // "OggS"
const HEADER_LEN = 27; // fixed part of a page header, before the lacing table

// Ogg's CRC-32: polynomial 0x04c11db7, bits fed most-significant first (no
// reflection), starting at 0, no final XOR. Built once at load time.
const CRC_TABLE = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let r = i << 24;
  for (let k = 0; k < 8; k++) {
    r = r & 0x80000000 ? (r << 1) ^ 0x04c11db7 : r << 1;
  }
  CRC_TABLE[i] = r >>> 0;
}

/** CRC of `page`, treating its CRC field (bytes 22-25) as zero. */
function pageCrc(page) {
  let crc = 0;
  for (let i = 0; i < page.length; i++) {
    const b = i >= 22 && i < 26 ? 0 : page[i];
    crc = ((crc << 8) ^ CRC_TABLE[((crc >>> 24) ^ b) & 0xff]) >>> 0;
  }
  return crc;
}

function concatBytes(a, b) {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

/** Index of the first "OggS" in `buf`, or -1. */
function findCapture(buf) {
  for (let i = 0; i + 4 <= buf.length; i++) {
    if (
      buf[i] === CAPTURE[0] &&
      buf[i + 1] === CAPTURE[1] &&
      buf[i + 2] === CAPTURE[2] &&
      buf[i + 3] === CAPTURE[3]
    ) {
      return i;
    }
  }
  return -1;
}

export class OggDemuxer {
  constructor() {
    this.reset();
  }

  /** Forget everything buffered (call when starting a new connection). */
  reset() {
    this._buf = new Uint8Array(0); // bytes received but not yet consumed
    this._partial = null; // bytes of a packet still open from the previous page
  }

  /**
   * @param {Uint8Array} bytes  Any chunk size, any boundary.
   * @returns {Array<{ data: Uint8Array, granule: number, bos: boolean }>}
   *   Complete packets, in stream order.
   */
  feed(bytes) {
    this._buf = concatBytes(this._buf, bytes);
    const packets = [];

    for (;;) {
      const start = findCapture(this._buf);
      if (start < 0) {
        // No page start anywhere. Keep only the last 3 bytes, which could be
        // the beginning of an "OggS" that the next chunk completes.
        this._buf = this._buf.subarray(Math.max(0, this._buf.length - 3));
        break;
      }
      if (start > 0) this._buf = this._buf.subarray(start); // drop garbage

      const buf = this._buf;
      if (buf.length < HEADER_LEN) break;
      const nsegs = buf[26];
      if (buf.length < HEADER_LEN + nsegs) break;
      let bodyLen = 0;
      for (let i = 0; i < nsegs; i++) bodyLen += buf[HEADER_LEN + i];
      const pageLen = HEADER_LEN + nsegs + bodyLen;
      if (buf.length < pageLen) break; // wait for the rest of the page

      const page = buf.subarray(0, pageLen);
      const view = new DataView(page.buffer, page.byteOffset, page.byteLength);
      if (pageCrc(page) !== view.getUint32(22, true)) {
        // Not a real page (or a damaged one): skip one byte and rescan.
        this._buf = buf.subarray(1);
        continue;
      }

      this._buf = buf.subarray(pageLen);
      this._readPage(page, view, nsegs, packets);
    }
    return packets;
  }

  _readPage(page, view, nsegs, packets) {
    const headerType = page[5];
    const continued = (headerType & 0x01) !== 0;
    const bos = (headerType & 0x02) !== 0;
    const granule = Number(view.getBigInt64(6, true));

    // A packet cannot carry on past a page that is not flagged "continued", so
    // an open partial from the previous page is dropped. Conversely, a
    // "continued" page with no partial open is the tail of a packet whose start
    // we never saw (e.g. we joined mid-stream); its first fragment is skipped.
    if (!continued) this._partial = null;
    let skipFirst = continued && this._partial === null;

    let at = HEADER_LEN + nsegs;
    let pieceStart = at;
    for (let i = 0; i < nsegs; i++) {
      const len = page[HEADER_LEN + i];
      at += len;
      if (len === 255 && i < nsegs - 1) continue; // packet goes on
      const piece = page.slice(pieceStart, at);
      pieceStart = at;
      if (len === 255) {
        // Last segment and still open: carry it into the next page (unless it
        // is the orphaned tail of a lost packet, which the next page will
        // again recognise as orphaned).
        if (skipFirst) continue;
        this._partial = this._partial ? concatBytes(this._partial, piece) : piece;
        continue;
      }
      const data = this._partial ? concatBytes(this._partial, piece) : piece;
      this._partial = null;
      if (skipFirst) {
        skipFirst = false; // orphaned tail of a packet whose start we never saw
        continue;
      }
      packets.push({ data, granule, bos });
    }
  }
}

// Second piece: LiveClock.
//
// The decoder hands us chunks of PCM audio; Web Audio plays a chunk at a time
// we choose. LiveClock is the pure policy for choosing that time. It knows
// nothing about Web Audio: it is given a function that returns "now" in
// seconds (AudioContext.currentTime in production) so tests can use a fake.
//
// Why a cushion (TARGET_LEAD_S = 0.2 s): we never start a chunk exactly "now",
// because by the time the call reaches the speaker it would already be late.
// 0.2 s covers one 40 ms Ogg page (we cannot decode half a page), the decoder's
// own latency, and roughly 100 ms of network jitter. Smaller is lower latency
// but more gaps; larger is safer but laggier.
//
// Why chunks go back to back: TCP cannot lose or reorder packets, so every
// chunk that arrives belongs directly after the previous one. We schedule each
// at the previous chunk's end, computed from its decoded length, which keeps
// the audio gapless with no timestamps needed.
//
// Underruns: if a chunk would have to start in the past, the stream stalled
// and we ran out of audio. Playing a gap is the honest outcome. We restart at
// now + lead, and raise the cushion by LEAD_STEP_S (up to MAX_LEAD_S) so a bad
// link ends up with a bigger cushion rather than repeated gaps.
//
// Catch-up: after a burst of pages (a stall that then delivers everything at
// once) we can end up far ahead of "now", i.e. permanently behind live. When
// the pending position is more than lead + CATCHUP_SLACK_S ahead, we report
// catchUp so the caller stops everything already scheduled, and we place this
// chunk at now + lead. One jump back to live beats staying late forever.
export const TARGET_LEAD_S = 0.2;
export const LEAD_STEP_S = 0.1;
export const MAX_LEAD_S = 0.6;
export const CATCHUP_SLACK_S = 0.4;

export class LiveClock {
  /** @param {() => number} now seconds (AudioContext.currentTime in production) */
  constructor(now) {
    this._now = now;
    this._lead = TARGET_LEAD_S;
    this._next = null; // where the next chunk starts; null means nothing pending
  }

  /** Current cushion in seconds. */
  get lead() {
    return this._lead;
  }

  /**
   * Decide when a chunk of `durationS` seconds should start.
   * @returns {{ at: number, restart: boolean, catchUp: boolean }}
   */
  place(durationS) {
    const now = this._now();
    let at;
    let restart = false;
    let catchUp = false;
    if (this._next === null || this._next < now) {
      // First chunk, or an underrun: start fresh. Only a genuine underrun
      // (we had a position and missed it) grows the cushion.
      if (this._next !== null) {
        this._lead = Math.min(this._lead + LEAD_STEP_S, MAX_LEAD_S);
      }
      restart = true;
      at = now + this._lead;
    } else if (this._next - now > this._lead + CATCHUP_SLACK_S) {
      catchUp = true;
      at = now + this._lead;
    } else {
      at = this._next;
    }
    this._next = at + durationS;
    return { at, restart, catchUp };
  }

  /** New connection: cushion back to the target, nothing pending. */
  reset() {
    this._lead = TARGET_LEAD_S;
    this._next = null;
  }
}

// Shared log prefix. It lives here (not in main.js) because main.js is the
// entry file Foundry loads and imports this one; importing back from main.js
// would make the two files depend on each other. main.js re-exports it.
export const MODULE_ID = "lutedrop-stream";

// Third piece: ReconnectPolicy.
//
// Both players lose the stream sometimes (LuteDrop restarted, Wi-Fi blip) and
// must try again without hammering the server. The policy holds the numbers:
// wait BACKOFF_START_MS, then double each time up to BACKOFF_MAX_MS. Once
// playback has run STABLE_AFTER_MS without trouble, the next failure starts
// from the short delay again. STALL_TIMEOUT_MS is how long the WebCodecs
// player waits with no bytes at all before treating the connection as dead.
//
// Giving up (WebCodecs player only): the WebCodecs player reads the stream with
// fetch(), which the browser only allows when the stream server sends a CORS
// header; the <audio> element needs no such header. If a URL has never
// delivered a single byte and fetch() itself has been refused GIVE_UP_AFTER
// times, the likely cause is a missing CORS header (or no network), so the
// caller switches to the <audio> player. The player only reports those
// refusals as failures here (see WebCodecsPlayer#reconnect): an HTTP error
// such as 502 means the server did answer and we were allowed to read it
// (e.g. Caddy while LuteDrop is not broadcasting yet), so that is a reason
// to wait and retry, not to fall back. Why "never received a byte": once any data has arrived,
// we know fetch() works against this server, so later failures are ordinary
// outages (LuteDrop restarting). Falling back then would downgrade this
// client for the rest of the session over something temporary.
export const BACKOFF_START_MS = 1000;
export const BACKOFF_MAX_MS = 15000;
export const STABLE_AFTER_MS = 10000;
export const GIVE_UP_AFTER = 3;
export const STALL_TIMEOUT_MS = 5000;

export class ReconnectPolicy {
  constructor({ giveUpAfter = GIVE_UP_AFTER } = {}) {
    this._giveUpAfter = giveUpAfter;
    this.reset();
  }

  /** The delay to wait before the next attempt; the one after it is doubled (capped). */
  nextDelayMs() {
    const delay = this._delayMs;
    this._delayMs = Math.min(this._delayMs * 2, BACKOFF_MAX_MS);
    return delay;
  }

  /** Playback has been healthy for a while: forget earlier failures' backoff. */
  markStable() {
    this._delayMs = BACKOFF_START_MS;
  }

  /** This URL has delivered data at least once, so giving up is off the table. */
  markBytesReceived() {
    this._gotBytes = true;
  }

  /** Count a failed connect. Returns true when the caller should give up. */
  markFailure() {
    this._failures++;
    return !this._gotBytes && this._failures >= this._giveUpAfter;
  }

  /** New URL: start over completely. */
  reset() {
    this._delayMs = BACKOFF_START_MS;
    this._failures = 0;
    this._gotBytes = false;
  }
}

/** Append a throwaway query value so the browser never reuses a dead cached response. */
function cacheBusted(url) {
  const separator = url.includes("?") ? "&" : "?";
  return `${url}${separator}t=${Date.now()}`;
}

// Fourth piece: ElementPlayer (the fallback).
//
// Keeps one HTMLAudioElement playing the stream and reconnects when it drops.
// It needs no CORS header from the stream server, which is why it is the
// fallback; but the browser decides how much to buffer, so it runs seconds
// behind live. It never gives up.
//
// "Listen off" sets volume to 0 instead of pausing, so the stream stays live
// and un-muting resumes at the current position.
export class ElementPlayer {
  #audio = new Audio();
  #url = "";
  #listening = true;
  #volume = 0.8;
  #started = false;
  #policy = new ReconnectPolicy();
  #retryTimer = null; // at most one pending reconnect at any time
  #stableTimer = null;

  constructor() {
    this.#audio.preload = "none";
    // Do not set crossOrigin: that would require CORS headers on the stream.
    for (const name of ["error", "stalled", "ended"]) {
      this.#audio.addEventListener(name, () => this.#reconnect());
    }
    this.#audio.addEventListener("playing", () => {
      this.#policy.markBytesReceived();
      this.#watchForStability();
    });
  }

  /** Begin playback. Call only after Foundry's audio-unlock gesture. */
  start() {
    this.#started = true;
    this.#applyVolume();
    this.#connect();
  }

  setUrl(url) {
    this.#url = String(url ?? "").trim();
    this.#policy.reset();
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
    this.#audio.src = cacheBusted(this.#url);
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

  /** Retry after the policy's current backoff delay (it doubles up to the cap). */
  #reconnect() {
    clearTimeout(this.#stableTimer);
    this.#stableTimer = null;
    // Blank URL (an empty src also raises "error") or already waiting: do nothing.
    if (!this.#started || !this.#url || this.#retryTimer !== null) return;
    this.#retryTimer = setTimeout(() => this.#connect(), this.#policy.nextDelayMs());
  }

  /** After STABLE_AFTER_MS of uninterrupted playback, forget earlier failures. */
  #watchForStability() {
    if (this.#stableTimer !== null) return;
    this.#stableTimer = setTimeout(() => {
      this.#stableTimer = null;
      this.#policy.markStable();
    }, STABLE_AFTER_MS);
  }
}

// Fifth piece: WebCodecsPlayer (the low-latency player).
//
// Why not just an <audio> element: the element owns its buffer and fills it as
// it sees fit, typically several seconds on a live stream, and it reacts
// unpredictably to a stream that never "ends". Here we do every step
// ourselves: fetch() hands us the raw bytes as they arrive, OggDemuxer cuts
// them into Opus packets, the browser's WebCodecs AudioDecoder turns each
// packet into PCM samples, and Web Audio plays each chunk at a time LiveClock
// chooses. We control the buffer, so we hold it at ~0.2 s instead of seconds.
//
// The cost: fetch() of another origin needs the server's CORS header (LuteDrop
// sends one), and WebCodecs only exists in secure contexts (https, or
// localhost). createPlayer() checks support; giving up falls back.
//
// Every connection gets a "generation" number. Anything asynchronous (a read,
// a decoder callback, a timer) remembers the generation it belongs to and does
// nothing if a newer connection (or a stop) has happened since. That keeps a
// dying connection from interfering with its replacement.
const OPUS_HEAD = "OpusHead";
const OPUS_TAGS = "OpusTags";
const OPUS_SAMPLE_RATE = 48000;
const PACKET_US = 20000; // each Opus packet LuteDrop sends is 20 ms (in microseconds)

function startsWithAscii(bytes, text) {
  if (bytes.length < text.length) return false;
  for (let i = 0; i < text.length; i++) {
    if (bytes[i] !== text.charCodeAt(i)) return false;
  }
  return true;
}

export class WebCodecsPlayer {
  #ctx;
  #gain; // every chunk plays through this; it carries volume and listen-off
  #demuxer = new OggDemuxer();
  #clock = new LiveClock(() => this.#ctx.currentTime);
  #policy = new ReconnectPolicy();
  #decoder = null;
  #abort = null; // AbortController of the current fetch
  #sources = new Set(); // scheduled AudioBufferSourceNodes not yet finished
  #stallTimer = null;
  #retryTimer = null;
  #stableTimer = null;
  #packetIndex = 0;
  #skipTags = false; // true between OpusHead and the OpusTags packet after it
  #scheduledAny = false; // has this connection scheduled a chunk yet?
  #generation = 0;
  #onGiveUp;
  #url = "";
  #listening = true;
  #volume = 0.8;
  #started = false;
  #gaveUp = false; // once true this player stays silent; the caller replaces it

  /**
   * @param {object} opts
   * @param {AudioContext} opts.audioContext
   * @param {() => void} [opts.onGiveUp]  called once if the URL never works (see ReconnectPolicy)
   * @param {AudioNode} [opts.output]  where the sound goes; the latency probe passes a detector here
   */
  constructor({ audioContext, onGiveUp, output = audioContext.destination }) {
    this.#ctx = audioContext;
    this.#onGiveUp = onGiveUp;
    this.#gain = audioContext.createGain();
    this.#gain.connect(output);
    this.#applyVolume();
  }

  /** Begin playback. Call only after Foundry's audio-unlock gesture. */
  start() {
    this.#started = true;
    this.#applyVolume();
    // A context created before the user's first click starts "suspended".
    this.#ctx.resume().catch(() => {});
    if (this.#url) this.#connect();
  }

  setUrl(url) {
    this.#url = String(url ?? "").trim();
    this.#policy.reset();
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

  // Listen-off is gain 0, not "stop reading". If we stopped, un-muting would
  // need a fresh connection and a fresh decoder (a noticeable pause); keeping
  // the stream flowing silently means un-muting is instant and already at the
  // live edge. The <audio> player does the same, so both behave alike.
  #applyVolume() {
    this.#gain.gain.value = this.#listening ? this.#volume : 0;
  }

  /**
   * End the current connection: invalidate its callbacks, clear every timer,
   * abort the fetch, close the decoder. With `stopSources`, also silence audio
   * already scheduled (otherwise its last ~0.2 s is left to play out).
   */
  #halt(stopSources) {
    this.#generation++;
    clearTimeout(this.#stallTimer);
    clearTimeout(this.#retryTimer);
    clearTimeout(this.#stableTimer);
    this.#stallTimer = null;
    this.#retryTimer = null;
    this.#stableTimer = null;
    this.#abort?.abort();
    this.#abort = null;
    if (this.#decoder && this.#decoder.state !== "closed") this.#decoder.close();
    this.#decoder = null;
    if (stopSources) this.#stopSources();
  }

  #stopSources() {
    for (const source of this.#sources) source.stop();
    this.#sources.clear();
  }

  /** Open a fresh connection to the current URL (or go silent if there is none). */
  #connect() {
    this.#halt(true);
    this.#demuxer.reset();
    this.#clock.reset();
    this.#packetIndex = 0;
    this.#skipTags = false;
    this.#scheduledAny = false;
    if (!this.#url) {
      console.info(`${MODULE_ID} | no stream URL set; staying silent`);
      return;
    }
    if (this.#gaveUp) return;
    const gen = this.#generation;
    this.#abort = new AbortController();
    this.#armStallTimer(gen); // also covers a server that accepts but never answers
    this.#read(gen, this.#abort.signal);
  }

  async #read(gen, signal) {
    let res;
    try {
      res = await fetch(cacheBusted(this.#url), { cache: "no-store", signal });
    } catch (err) {
      // fetch() itself refused (a TypeError: CORS blocked, DNS failure,
      // connection refused). This is the only kind of failure that counts
      // toward giving up; see ReconnectPolicy.
      if (gen !== this.#generation || err?.name === "AbortError") return;
      this.#reconnect(gen, err?.message ?? String(err), { countsTowardGiveUp: true });
      return;
    }
    try {
      // A non-OK status (e.g. Caddy's 502 while LuteDrop is not broadcasting
      // yet) proves the server answered and we may read it: retry, never give up.
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const reader = res.body.getReader();
      let first = true;
      for (;;) {
        const { done, value } = await reader.read();
        if (gen !== this.#generation) return;
        if (done) throw new Error("stream ended");
        if (first) {
          first = false;
          console.info(`${MODULE_ID} | connected; receiving the stream`);
          this.#stableTimer = setTimeout(() => {
            this.#stableTimer = null;
            this.#policy.markStable();
          }, STABLE_AFTER_MS);
        }
        this.#policy.markBytesReceived();
        this.#armStallTimer(gen);
        for (const packet of this.#demuxer.feed(value)) {
          if (gen !== this.#generation) return; // a decode failure already reconnected
          this.#onPacket(packet, gen);
        }
      }
    } catch (err) {
      // Our own abort (setUrl, reconnect, stall) lands here too; ignore it.
      if (gen !== this.#generation || err?.name === "AbortError") return;
      this.#reconnect(gen, err?.message ?? String(err));
    }
  }

  #armStallTimer(gen) {
    clearTimeout(this.#stallTimer);
    this.#stallTimer = setTimeout(
      () => this.#reconnect(gen, `no data for ${STALL_TIMEOUT_MS / 1000} s`),
      STALL_TIMEOUT_MS,
    );
  }

  #onPacket(packet, gen) {
    if (!this.#decoder) {
      // Until OpusHead arrives there is nothing we can decode; anything
      // before it (there should be nothing) is dropped.
      if (packet.bos && startsWithAscii(packet.data, OPUS_HEAD)) this.#openDecoder(packet.data, gen);
      return;
    }
    if (this.#skipTags) {
      // OpusTags is metadata (encoder name, comments), not audio.
      this.#skipTags = false;
      if (startsWithAscii(packet.data, OPUS_TAGS)) return;
    }
    if (this.#decoder.state !== "configured") return;
    // The decoder insists on a timestamp, so we give it a simple count of
    // 20 ms packets. We do NOT use these timestamps (or the Ogg granule
    // positions) to decide when to play: over TCP no packet can be lost or
    // reordered, so each decoded chunk belongs right after the previous one,
    // and its decoded length is the truth about how long it lasts. LiveClock
    // schedules from that length.
    const chunk = new EncodedAudioChunk({
      type: "key",
      timestamp: this.#packetIndex * PACKET_US,
      data: packet.data,
    });
    this.#packetIndex++;
    try {
      this.#decoder.decode(chunk);
    } catch (err) {
      this.#reconnect(gen, `decode failed: ${err?.message ?? err}`);
    }
  }

  #openDecoder(opusHead, gen) {
    const decoder = new AudioDecoder({
      output: (audioData) => this.#onDecoded(audioData, gen),
      error: (err) => this.#reconnect(gen, `decoder error: ${err?.message ?? err}`),
    });
    try {
      // `description` is the OpusHead packet itself, as WebCodecs' Opus rules
      // require. It tells the decoder the channel count, the channel layout,
      // and the "pre-skip" (encoder warm-up samples at the stream's start).
      decoder.configure({
        codec: "opus",
        sampleRate: OPUS_SAMPLE_RATE,
        numberOfChannels: 2,
        description: opusHead,
      });
    } catch (err) {
      decoder.close();
      this.#reconnect(gen, `decoder setup failed: ${err?.message ?? err}`);
      return;
    }
    this.#decoder = decoder;
    this.#skipTags = true;
  }

  /** One decoded chunk (20 ms of PCM): copy it into an AudioBuffer and schedule it. */
  #onDecoded(audioData, gen) {
    try {
      if (gen !== this.#generation) return;
      this.#copyAndSchedule(audioData);
    } catch (err) {
      // An exception here would otherwise escape the decoder callback as an
      // uncaught error and leave the stream silently broken; reconnect instead.
      this.#reconnect(gen, `playing decoded audio failed: ${err?.message ?? err}`);
    } finally {
      // AudioData holds memory outside JavaScript; release it once copied.
      audioData.close();
    }
  }

  /** Copy one decoded chunk (20 ms of PCM) into an AudioBuffer and schedule it. */
  #copyAndSchedule(audioData) {
    const channels = audioData.numberOfChannels;
    const frames = audioData.numberOfFrames;
    const buffer = this.#ctx.createBuffer(channels, frames, audioData.sampleRate);
    if (audioData.format === "f32") {
      // Interleaved (L R L R ...): copy once, then split into channels.
      const interleaved = new Float32Array(frames * channels);
      audioData.copyTo(interleaved, { planeIndex: 0, format: "f32" });
      for (let ch = 0; ch < channels; ch++) {
        const out = buffer.getChannelData(ch);
        for (let i = 0; i < frames; i++) out[i] = interleaved[i * channels + ch];
      }
    } else {
      // Planar, or another sample format the browser converts for us.
      for (let ch = 0; ch < channels; ch++) {
        audioData.copyTo(buffer.getChannelData(ch), { planeIndex: ch, format: "f32-planar" });
      }
    }
    this.#schedule(buffer, audioData.duration / 1e6);
  }

  #schedule(buffer, durationS) {
    const { at, restart, catchUp } = this.#clock.place(durationS);
    const leadMs = Math.round(this.#clock.lead * 1000);
    if (restart && this.#scheduledAny) {
      console.info(`${MODULE_ID} | ran out of audio (underrun); restarting with a ${leadMs} ms cushion`);
    }
    if (catchUp) {
      // Far behind live: drop what is queued and jump back to the live edge.
      this.#stopSources();
      console.info(`${MODULE_ID} | fell behind live; jumped ahead (cushion ${leadMs} ms)`);
    }
    this.#scheduledAny = true;
    const source = this.#ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(this.#gain);
    source.addEventListener("ended", () => {
      this.#sources.delete(source);
      source.disconnect();
    });
    source.start(at);
    this.#sources.add(source);
  }

  /**
   * The connection failed (fetch refused, HTTP error, stream ended, decoder
   * error, or stall). Retry later, or give up (see ReconnectPolicy). Only a
   * refused fetch() passes `countsTowardGiveUp`: everything else means the
   * server is reachable and readable, so we keep retrying with backoff.
   */
  #reconnect(gen, reason, { countsTowardGiveUp = false } = {}) {
    if (gen !== this.#generation) return; // this connection is already gone
    this.#halt(false); // let the last scheduled audio play out while we wait
    if (countsTowardGiveUp && this.#policy.markFailure()) {
      this.#gaveUp = true;
      this.#stopSources();
      // Detach from the output so nothing of this player stays in the graph.
      this.#gain.disconnect();
      console.warn(
        `${MODULE_ID} | WebCodecs player giving up: fetch refused ${GIVE_UP_AFTER} times and no data ever arrived (${reason})`,
      );
      this.#onGiveUp?.();
      return;
    }
    const delay = this.#policy.nextDelayMs();
    console.info(`${MODULE_ID} | stream lost (${reason}); reconnecting in ${delay} ms`);
    this.#retryTimer = setTimeout(() => {
      this.#retryTimer = null;
      this.#connect();
    }, delay);
  }
}

/** True when this browser can decode Opus with WebCodecs (needs a secure context). */
export async function isWebCodecsOpusSupported() {
  if (typeof AudioDecoder === "undefined" || typeof EncodedAudioChunk === "undefined") return false;
  try {
    const { supported } = await AudioDecoder.isConfigSupported({
      codec: "opus",
      sampleRate: OPUS_SAMPLE_RATE,
      numberOfChannels: 2,
    });
    return supported === true;
  } catch {
    return false;
  }
}

/** The WebCodecs player when this browser supports it, else the <audio> player. */
export async function createPlayer({ audioContext, onGiveUp, output }) {
  if (await isWebCodecsOpusSupported()) {
    console.info(`${MODULE_ID} | using the low-latency WebCodecs player`);
    return new WebCodecsPlayer({ audioContext, onGiveUp, output });
  }
  console.info(`${MODULE_ID} | WebCodecs Opus unavailable; using the <audio> player`);
  return new ElementPlayer();
}
