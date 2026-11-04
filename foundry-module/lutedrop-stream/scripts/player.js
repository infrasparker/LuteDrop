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

// First piece in reverse: OggOpusMuxer.
//
// The decodeAudioData player (further down) needs the opposite of the demuxer:
// it has loose Opus packets and must hand the browser a *complete* Ogg Opus
// file, because decodeAudioData only decodes whole files, never a stream.
// So for every small group of packets we build a tiny file: the OpusHead page,
// the OpusTags page, then the audio packets. That is the smallest thing every
// browser's Ogg reader accepts as "a valid Ogg Opus file".
//
// Granule position for Opus counts 48 kHz samples from the start of the file,
// including the "pre-skip" (encoder warm-up samples the decoder throws away).
// Each LuteDrop packet is 20 ms = 960 samples, so after n packets the granule
// is preSkip + 960 * n. Writing it exactly like that tells the decoder "do not
// trim anything off the end", so every sample of the last packet comes out.
const OGG_SERIAL = 0x4c757465; // any constant works; "Lute" in ASCII
const SAMPLES_PER_PACKET = 960; // 20 ms at 48 kHz
const MAX_LACING = 255; // a page's lacing table holds at most 255 values

/** Lacing values for one packet: a 255 for each full 255 bytes, then the rest (0 included). */
function lacingFor(packet) {
  const values = new Array(Math.floor(packet.length / 255)).fill(255);
  values.push(packet.length % 255);
  return values;
}

/** One finished Ogg page holding whole `packets`, with its CRC filled in. */
function writePage(headerType, granule, seq, packets) {
  const lacing = packets.flatMap(lacingFor);
  const bodyLen = packets.reduce((n, p) => n + p.length, 0);
  const page = new Uint8Array(HEADER_LEN + lacing.length + bodyLen);
  const view = new DataView(page.buffer);
  page.set(CAPTURE, 0);
  page[4] = 0; // version
  page[5] = headerType;
  view.setBigInt64(6, BigInt(granule), true);
  view.setUint32(14, OGG_SERIAL, true);
  view.setUint32(18, seq, true);
  page[26] = lacing.length;
  page.set(lacing, HEADER_LEN);
  let at = HEADER_LEN + lacing.length;
  for (const p of packets) {
    page.set(p, at);
    at += p.length;
  }
  view.setUint32(22, pageCrc(page), true); // pageCrc ignores the (still zero) CRC field
  return page;
}

export class OggOpusMuxer {
  /**
   * @param {Uint8Array} opusHead  the stream's OpusHead packet
   * @param {Uint8Array} opusTags  the stream's OpusTags packet
   */
  constructor(opusHead, opusTags) {
    this._head = opusHead;
    this._tags = opusTags;
    // Bytes 10-11 of OpusHead: pre-skip, little-endian.
    this.preSkip = opusHead.length >= 12 ? opusHead[10] | (opusHead[11] << 8) : 0;
  }

  /**
   * A complete Ogg Opus file holding `packets` (20 ms each).
   * Pages: OpusHead (bos, seq 0), OpusTags (seq 1), then the audio (eos on the
   * last page). The audio normally fits one page; only a burst large enough to
   * overflow a page's 255 lacing values spills onto more pages.
   * @param {Uint8Array[]} packets
   * @returns {Uint8Array}
   */
  file(packets) {
    const pages = [writePage(0x02, 0, 0, [this._head]), writePage(0x00, 0, 1, [this._tags])];
    let seq = 2;
    let done = 0; // packets already written, for the granule
    let group = [];
    let lacingCount = 0;
    const flush = (last) => {
      done += group.length;
      const granule = this.preSkip + SAMPLES_PER_PACKET * done;
      pages.push(writePage(last ? 0x04 : 0x00, granule, seq++, group));
      group = [];
      lacingCount = 0;
    };
    for (const packet of packets) {
      const n = Math.floor(packet.length / 255) + 1;
      if (group.length > 0 && lacingCount + n > MAX_LACING) flush(false);
      group.push(packet);
      lacingCount += n;
    }
    flush(true);

    const out = new Uint8Array(pages.reduce((n, p) => n + p.length, 0));
    let at = 0;
    for (const p of pages) {
      out.set(p, at);
      at += p.length;
    }
    return out;
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
//
// Shrinking back (decay): underruns are not always the network's fault.
// Foundry itself freezes the page for a moment when it loads a scene, and
// that starves us too. Without a way down, each such freeze would leave a
// bigger cushion (more delay) for the rest of the night. So after
// LEAD_DECAY_AFTER_S (60 s) with no underrun, the cushion drops by
// LEAD_STEP_S, and again after every further quiet minute, never below
// TARGET_LEAD_S. A link that really needs the bigger cushion will underrun
// again and grow it back; one that does not stays fast.
//
// Slow drift: LuteDrop's sound card and the listener's sound card each keep
// their own time, and no two run at exactly the same speed. If ours runs a
// touch slow, audio piles up a few milliseconds per minute: never enough for
// the big catch-up above, but enough to leave us late for good. Lowering the
// cushion (decay) has the same effect: the audio already queued stays where
// it is. So if the pending audio has stayed more than DRIFT_SLACK_S (0.1 s)
// above the cushion for DRIFT_AFTER_S (5 s) in a row, we report catchUp just
// like the big jump. The 5 s wait means a brief burst (which drains by
// itself) never causes a jump; only lateness that sticks does.
export const TARGET_LEAD_S = 0.2;
export const LEAD_STEP_S = 0.1;
export const MAX_LEAD_S = 0.6;
export const CATCHUP_SLACK_S = 0.4;
export const LEAD_DECAY_AFTER_S = 60;
export const DRIFT_SLACK_S = 0.1;
export const DRIFT_AFTER_S = 5;

export class LiveClock {
  /** @param {() => number} now seconds (AudioContext.currentTime in production) */
  constructor(now) {
    this._now = now;
    this.reset();
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
      this._quietSince = now; // the quiet minute (decay) counts from here
      this._driftSince = null;
      restart = true;
      at = now + this._lead;
    } else {
      this.#decay(now);
      const pending = this._next - now;
      if (pending > this._lead + CATCHUP_SLACK_S || this.#driftedLongEnough(pending, now)) {
        catchUp = true;
        this._driftSince = null;
        at = now + this._lead;
      } else {
        at = this._next;
      }
    }
    this._next = at + durationS;
    return { at, restart, catchUp };
  }

  /** After a quiet minute, lower the cushion one step (not below the target). */
  #decay(now) {
    if (now - this._quietSince < LEAD_DECAY_AFTER_S) return;
    this._quietSince = now; // the next step needs another quiet minute
    this._lead = Math.max(this._lead - LEAD_STEP_S, TARGET_LEAD_S);
  }

  /** True once `pending` has sat above lead + DRIFT_SLACK_S for DRIFT_AFTER_S. */
  #driftedLongEnough(pending, now) {
    if (pending <= this._lead + DRIFT_SLACK_S) {
      this._driftSince = null; // back within the slack: the clock starts over
      return false;
    }
    this._driftSince ??= now;
    return now - this._driftSince >= DRIFT_AFTER_S;
  }

  /** New connection: cushion back to the target, nothing pending. */
  reset() {
    this._lead = TARGET_LEAD_S;
    this._next = null; // where the next chunk starts; null means nothing pending
    this._quietSince = 0; // `now` of the last underrun (or the first chunk)
    this._driftSince = null; // `now` when pending first went above lead + slack
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
// from the short delay again. STALL_TIMEOUT_MS is how long the low-latency
// players wait with no bytes at all before treating the connection as dead.
//
// Giving up (low-latency players only): those players read the stream with
// fetch(), which the browser only allows when the response carries a CORS
// header; the <audio> element needs no such header. If a URL has never
// delivered a single byte and fetch() itself has been refused GIVE_UP_AFTER
// times since the last URL change (not necessarily in a row), the caller
// switches to the <audio> player. The player only reports those refusals as
// failures here (see StreamPlayerBase _reconnect).
//
// What counts as "refused": the browser rejects a cross-origin fetch() of
// any response without the CORS header, whatever its status. So besides a
// LuteDrop too old to send the header and a network that is down, a server
// that answers but whose error page has no header also counts: Caddy's own
// 502 while LuteDrop is not broadcasting yet, or LuteDrop closed on the LAN
// (connection refused). LuteDrop's own 404 for a wrong key does carry the
// header, so that one is a plain HTTP error: retry, never give up.
//
// That is why falling back is not final: main.js lets the <audio> player
// check, each time it starts playing, whether the stream can be fetched
// again, and if so swaps the low-latency player back in. A client that opens
// Foundry before the GM switches Broadcast on starts on the <audio> player
// and upgrades a few seconds after the stream goes live.
//
// Why "never received a byte": once any data has arrived, we know fetch()
// works against this server, so later failures are ordinary outages
// (LuteDrop restarting). Falling back then would downgrade this client over
// something temporary.
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
export function cacheBusted(url) {
  const separator = url.includes("?") ? "&" : "?";
  return `${url}${separator}t=${Date.now()}`;
}

// Fourth piece: LiveEdgeHold (keeps the <audio> fallback near live).
//
// Why the element drifts: an <audio> element decides on its own how much of a
// stream to download ahead of what it plays. On a live stream that "ahead" is
// pure delay: everything downloaded but not yet played is audio the players
// hear late. The browser never tries to catch up, so the delay only grows
// (measured: 2 s right after starting, then 7-28 s).
//
// What we can see from outside: currentTime (where the element is playing) and
// the end of its `buffered` range (how far it has downloaded, i.e. the live
// edge as far as this element knows). The difference is the delay we add.
//
// What we do about it, gently first:
// 1. Seek: set currentTime to just behind the buffered end. One jump, done.
// 2. If the next look shows the gap is still too big, the seek was ignored
//    (a chunked live stream often has no seekable range). Then play slightly
//    faster (HOLD_CATCHUP_RATE = 1.15) until caught up, and back to normal.
//    The browser keeps the pitch the same, so a few seconds at 1.15x is
//    barely audible.
// 3. If currentTime has not moved for HOLD_STALL_S seconds, the element is
//    stuck; the only cure is to open the stream again.
//
// Why 0.5 s behind the edge (HOLD_TARGET_S): the element needs some audio in
// hand or it will run dry and stutter whenever the network hiccups. Half a
// second is enough for that while still being close to live. We only act when
// the gap passes HOLD_MAX_GAP_S (1 s), so normal jitter does not cause
// constant seeking.
//
// Like LiveClock, this is pure: no DOM, just numbers in, a decision out, so it
// can be tested under Node. ElementPlayer polls it every HOLD_POLL_MS.
export const HOLD_TARGET_S = 0.5;
export const HOLD_MAX_GAP_S = 1.0;
export const HOLD_CATCHUP_RATE = 1.15;
export const HOLD_POLL_MS = 250;
export const HOLD_STALL_S = 3;

export class LiveEdgeHold {
  constructor() {
    this.reset();
  }

  /**
   * Look at the element once and decide what to do.
   * @param {{ currentTime: number, bufferedEnd: number, now: number }} view  seconds
   * @returns {{ action: "none" | "seek" | "rate" | "normal" | "reconnect", to?: number, rate?: number }}
   */
  poll({ currentTime, bufferedEnd, now }) {
    // Stall check first: if playback is stuck, catching up is pointless.
    if (this._lastTime === null || currentTime !== this._lastTime) {
      this._lastTime = currentTime;
      this._movedAt = now;
    } else if (now - this._movedAt >= HOLD_STALL_S) {
      this.reset(); // the caller opens a new connection
      return { action: "reconnect" };
    }

    const gap = bufferedEnd - currentTime;
    if (this._mode === "rate") {
      if (gap > HOLD_TARGET_S) return { action: "rate", rate: HOLD_CATCHUP_RATE };
      this._mode = "idle";
      return { action: "normal" };
    }
    if (this._mode === "seeked") {
      if (gap > HOLD_MAX_GAP_S) {
        // The seek did not take; speed up instead.
        this._mode = "rate";
        return { action: "rate", rate: HOLD_CATCHUP_RATE };
      }
      this._mode = "idle"; // the seek worked
      return { action: "none" };
    }
    if (gap > HOLD_MAX_GAP_S) {
      this._mode = "seeked";
      return { action: "seek", to: bufferedEnd - HOLD_TARGET_S };
    }
    return { action: "none" };
  }

  /** New connection: forget everything. */
  reset() {
    this._mode = "idle"; // "idle" | "seeked" (just tried a seek) | "rate" (catching up)
    this._lastTime = null; // currentTime at the last poll
    this._movedAt = 0; // `now` when currentTime last changed
  }
}

// Fifth piece: ElementPlayer (the fallback).
//
// Keeps one HTMLAudioElement playing the stream and reconnects when it drops.
// It needs no CORS header from the stream server, which is why it is the
// fallback; but the browser decides how much to buffer, so left alone it runs
// seconds behind live. LiveEdgeHold (above) pulls it back: while playing we
// look at the element every HOLD_POLL_MS and seek, speed up, or reconnect as
// the policy says. It never gives up.
//
// `onPlaying` (optional) is called once per connection, the first time the
// element reaches "playing" (bytes are flowing). main.js uses it to check
// whether the faster player can take over again (see ReconnectPolicy).
//
// "Listen off" sets volume to 0 instead of pausing, so the stream stays live
// and un-muting resumes at the current position.
export class ElementPlayer {
  #audio;
  #url = "";
  #listening = true;
  #volume = 0.8;
  #started = false;
  #policy = new ReconnectPolicy();
  #retryTimer = null; // at most one pending reconnect at any time
  #stableTimer = null;
  #hold = new LiveEdgeHold();
  #holdTimer = null; // the live-edge poll; at most one per connection
  #catchingUp = false; // playing at HOLD_CATCHUP_RATE right now (so we log it once)
  #onPlaying;
  #announcedPlaying = false; // has this connection called onPlaying yet?

  /**
   * @param {object} [opts]
   * @param {HTMLAudioElement} [opts.audio]  only the latency probe passes one (it
   *   needs crossOrigin to listen to the element); the module passes nothing.
   * @param {() => void} [opts.onPlaying]  called once per connection when playback
   *   first starts
   */
  constructor({ audio = new Audio(), onPlaying } = {}) {
    this.#audio = audio;
    this.#onPlaying = onPlaying;
    this.#audio.preload = "none";
    // Do not set crossOrigin: that would require CORS headers on the stream.
    for (const name of ["error", "stalled", "ended"]) {
      this.#audio.addEventListener(name, () => this.#reconnect());
    }
    this.#audio.addEventListener("playing", () => {
      this.#policy.markBytesReceived();
      this.#watchForStability();
      this.#startHold();
      // "playing" also fires after every seek or short buffering pause; only
      // the first one of a connection is news.
      if (!this.#announcedPlaying) {
        this.#announcedPlaying = true;
        this.#onPlaying?.();
      }
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
    this.#stopHold();
    this.#hold.reset(); // a new connection has its own clock and its own buffer
    this.#announcedPlaying = false;
    if (!this.#url) {
      this.#stop();
      return;
    }
    this.#audio.src = cacheBusted(this.#url);
    this.#audio.play().catch((err) => this.#onPlayRejected(err));
  }

  #stop() {
    this.#stopHold();
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
    // Covers error, stalled, ended, a rejected play() and the hold's own stall
    // check: there is no point steering a connection that is being replaced.
    this.#stopHold();
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

  /**
   * Start polling the live-edge hold. "playing" fires again after every seek,
   * rate change or short buffering pause, so only the first one per
   * connection creates the interval.
   */
  #startHold() {
    if (this.#holdTimer !== null) return;
    this.#holdTimer = setInterval(() => this.#pollHold(), HOLD_POLL_MS);
  }

  #stopHold() {
    clearInterval(this.#holdTimer);
    this.#holdTimer = null;
    this.#catchingUp = false;
    // Back to normal speed. (Loading a new src does this too; being explicit
    // means a stopped or reconnecting player never keeps the catch-up rate.)
    this.#audio.playbackRate = 1;
  }

  /** One look at the element; act on what LiveEdgeHold decides. */
  #pollHold() {
    const audio = this.#audio;
    const buffered = audio.buffered;
    if (buffered.length === 0) return; // nothing downloaded yet, nothing to judge
    const bufferedEnd = buffered.end(buffered.length - 1);
    const decision = this.#hold.poll({
      currentTime: audio.currentTime,
      bufferedEnd,
      now: performance.now() / 1000,
    });
    switch (decision.action) {
      case "seek": {
        const behindS = (bufferedEnd - audio.currentTime).toFixed(1);
        console.info(`${MODULE_ID} | ${behindS} s behind live; jumping to the live edge`);
        audio.currentTime = decision.to;
        break;
      }
      case "rate":
        if (!this.#catchingUp) {
          this.#catchingUp = true;
          console.info(`${MODULE_ID} | the jump did not take; playing at ${decision.rate}x to catch up`);
        }
        audio.playbackRate = decision.rate;
        break;
      case "normal":
        this.#catchingUp = false;
        audio.playbackRate = 1;
        console.info(`${MODULE_ID} | caught up with live; back to normal speed`);
        break;
      case "reconnect":
        console.info(`${MODULE_ID} | playback stuck for ${HOLD_STALL_S} s; reconnecting`);
        this.#reconnect();
        break;
      default:
        break; // "none": close enough to live
    }
  }
}

// Sixth piece: the low-latency players.
//
// Why not just an <audio> element: the element owns its buffer and fills it as
// it sees fit, typically several seconds on a live stream, and it reacts
// unpredictably to a stream that never "ends". Here we do every step
// ourselves: fetch() hands us the raw bytes as they arrive, OggDemuxer cuts
// them into Opus packets, a decoder turns the packets into PCM samples, and Web
// Audio plays each chunk at a time LiveClock chooses. We control the buffer, so
// we hold it at ~0.2 s instead of seconds.
//
// There are two of these players, which differ only in the decoder:
// - WebCodecsPlayer uses the browser's AudioDecoder (WebCodecs). It decodes a
//   continuous stream packet by packet, but only exists in secure contexts
//   (https, or localhost).
// - DecodeAudioDataPlayer uses AudioContext.decodeAudioData, which also works
//   on plain http. It decodes whole files only, so it wraps packets in tiny
//   files (see OggOpusMuxer and the player's own notes below).
// Everything else (fetching, demuxing, reconnecting, giving up, the stall
// timer, scheduling with LiveClock, volume) is the same, so it lives once in
// StreamPlayerBase and each player only supplies its decoder.
//
// The cost of both: fetch() of another origin needs the server's CORS header
// (LuteDrop sends one). createPlayer() checks what the browser supports;
// giving up falls back to the <audio> element until the stream can be
// fetched again (see ReconnectPolicy).
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

/**
 * Shared plumbing of the two low-latency players. Not exported: a subclass
 * supplies the decoder through these hooks:
 * - `_onHeader(opusHead, opusTags, gen)`: the stream's two setup packets
 *   arrived (opusTags is null if the stream skipped it). Set up the decoder.
 * - `_onPackets(packets, gen)`: the audio packets from one network read, in
 *   order. Decode them and pass the PCM to `_schedule`.
 * - `_onHalt()`: the connection ended; drop all decoder state.
 * And uses these helpers: `_ctx`, `_isCurrent(gen)`, `_reconnect(gen, reason)`,
 * `_schedule(planes, sampleRate, gen)`.
 */
class StreamPlayerBase {
  #ctx;
  #gain; // every chunk plays through this; it carries volume and listen-off
  #name; // "WebCodecs player" etc., for the give-up message
  #demuxer = new OggDemuxer();
  #clock = new LiveClock(() => this.#ctx.currentTime);
  #policy = new ReconnectPolicy();
  #abort = null; // AbortController of the current fetch
  #sources = new Set(); // scheduled AudioBufferSourceNodes not yet finished
  #stallTimer = null;
  #retryTimer = null;
  #stableTimer = null;
  #headerState = "head"; // "head" (want OpusHead) | "tags" (want OpusTags) | "audio"
  #opusHead = null;
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
   * @param {string} name  how log lines name this player
   */
  constructor({ audioContext, onGiveUp, output = audioContext.destination }, name) {
    this.#ctx = audioContext;
    this.#onGiveUp = onGiveUp;
    this.#name = name;
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

  // ---- for subclasses ----

  get _ctx() {
    return this.#ctx;
  }

  /** True while `gen` is still the live connection. */
  _isCurrent(gen) {
    return gen === this.#generation;
  }

  // Hooks; the subclasses replace these.
  _onHeader() {}
  _onPackets() {}
  _onHalt() {}

  // Listen-off is gain 0, not "stop reading". If we stopped, un-muting would
  // need a fresh connection and a fresh decoder (a noticeable pause); keeping
  // the stream flowing silently means un-muting is instant and already at the
  // live edge. The <audio> player does the same, so both behave alike.
  #applyVolume() {
    this.#gain.gain.value = this.#listening ? this.#volume : 0;
  }

  /**
   * End the current connection: invalidate its callbacks, clear every timer,
   * abort the fetch, drop the decoder. With `stopSources`, also silence audio
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
    this._onHalt();
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
    this.#headerState = "head";
    this.#opusHead = null;
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
      // fetch() itself refused (a TypeError: CORS blocked, including an
      // error page sent without the CORS header, DNS failure, connection
      // refused). This is the only kind of failure that counts toward giving
      // up; see ReconnectPolicy.
      if (gen !== this.#generation || err?.name === "AbortError") return;
      this._reconnect(gen, err?.message ?? String(err), { countsTowardGiveUp: true });
      return;
    }
    try {
      // A non-OK status we are allowed to see (LuteDrop's own 404 for a wrong
      // key carries the CORS header) proves the server answered: retry, never
      // give up. One without the header never gets here; see ReconnectPolicy.
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
        const audio = [];
        for (const packet of this.#demuxer.feed(value)) {
          if (gen !== this.#generation) return; // a decoder setup failure already reconnected
          this.#sortPacket(packet, gen, audio);
        }
        if (gen !== this.#generation) return;
        if (audio.length > 0) this._onPackets(audio, gen);
      }
    } catch (err) {
      // Our own abort (setUrl, reconnect, stall) lands here too; ignore it.
      if (gen !== this.#generation || err?.name === "AbortError") return;
      this._reconnect(gen, err?.message ?? String(err));
    }
  }

  #armStallTimer(gen) {
    clearTimeout(this.#stallTimer);
    this.#stallTimer = setTimeout(
      () => this._reconnect(gen, `no data for ${STALL_TIMEOUT_MS / 1000} s`),
      STALL_TIMEOUT_MS,
    );
  }

  /** Header packets go to `_onHeader`; audio packets are collected in `audio`. */
  #sortPacket(packet, gen, audio) {
    if (this.#headerState === "head") {
      // Until OpusHead arrives there is nothing we can decode; anything
      // before it (there should be nothing) is dropped.
      if (packet.bos && startsWithAscii(packet.data, OPUS_HEAD)) {
        this.#opusHead = packet.data;
        this.#headerState = "tags";
      }
      return;
    }
    if (this.#headerState === "tags") {
      // OpusTags is metadata (encoder name, comments), not audio. It always
      // follows OpusHead; if it is missing, this packet is already audio.
      this.#headerState = "audio";
      const isTags = startsWithAscii(packet.data, OPUS_TAGS);
      this._onHeader(this.#opusHead, isTags ? packet.data : null, gen);
      if (isTags) return;
    }
    audio.push(packet.data);
  }

  /**
   * Play one chunk of decoded audio: `planes` holds one Float32Array per
   * channel, all the same length. It starts where LiveClock says.
   */
  _schedule(planes, sampleRate, gen) {
    if (gen !== this.#generation) return;
    const frames = planes[0]?.length ?? 0;
    if (frames === 0) return;
    const buffer = this.#ctx.createBuffer(planes.length, frames, sampleRate);
    planes.forEach((plane, ch) => buffer.copyToChannel(plane, ch));
    const { at, restart, catchUp } = this.#clock.place(frames / sampleRate);
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
  _reconnect(gen, reason, { countsTowardGiveUp = false } = {}) {
    if (gen !== this.#generation) return; // this connection is already gone
    this.#halt(false); // let the last scheduled audio play out while we wait
    if (countsTowardGiveUp && this.#policy.markFailure()) {
      this.#gaveUp = true;
      this.#stopSources();
      // Detach from the output so nothing of this player stays in the graph.
      this.#gain.disconnect();
      console.warn(
        `${MODULE_ID} | ${this.#name} giving up: fetch refused ${GIVE_UP_AFTER} times and no data ever arrived (${reason})`,
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

// WebCodecsPlayer: the stream player for secure pages (https or localhost),
// decoding with the browser's AudioDecoder one packet at a time.
export class WebCodecsPlayer extends StreamPlayerBase {
  #decoder = null;
  #packetIndex = 0;

  /**
   * @param {object} opts
   * @param {AudioContext} opts.audioContext
   * @param {() => void} [opts.onGiveUp]  called once if the URL never works (see ReconnectPolicy)
   * @param {AudioNode} [opts.output]  where the sound goes; the latency probe passes a detector here
   */
  constructor(opts) {
    super(opts, "WebCodecs player");
  }

  _onHalt() {
    if (this.#decoder && this.#decoder.state !== "closed") this.#decoder.close();
    this.#decoder = null;
    this.#packetIndex = 0;
  }

  _onHeader(opusHead, _opusTags, gen) {
    const decoder = new AudioDecoder({
      output: (audioData) => this.#onDecoded(audioData, gen),
      error: (err) => this._reconnect(gen, `decoder error: ${err?.message ?? err}`),
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
      this._reconnect(gen, `decoder setup failed: ${err?.message ?? err}`);
      return;
    }
    this.#decoder = decoder;
  }

  _onPackets(packets, gen) {
    for (const data of packets) {
      if (!this._isCurrent(gen)) return; // a decode failure already reconnected
      if (!this.#decoder || this.#decoder.state !== "configured") return;
      // The decoder insists on a timestamp, so we give it a simple count of
      // 20 ms packets. We do NOT use these timestamps (or the Ogg granule
      // positions) to decide when to play: over TCP no packet can be lost or
      // reordered, so each decoded chunk belongs right after the previous one,
      // and its decoded length is the truth about how long it lasts. LiveClock
      // schedules from that length.
      const chunk = new EncodedAudioChunk({
        type: "key",
        timestamp: this.#packetIndex * PACKET_US,
        data,
      });
      this.#packetIndex++;
      try {
        this.#decoder.decode(chunk);
      } catch (err) {
        this._reconnect(gen, `decode failed: ${err?.message ?? err}`);
      }
    }
  }

  /** One decoded chunk (20 ms of PCM): copy it out and schedule it. */
  #onDecoded(audioData, gen) {
    try {
      if (!this._isCurrent(gen)) return;
      this._schedule(copyPlanes(audioData), audioData.sampleRate, gen);
    } catch (err) {
      // An exception here would otherwise escape the decoder callback as an
      // uncaught error and leave the stream silently broken; reconnect instead.
      this._reconnect(gen, `playing decoded audio failed: ${err?.message ?? err}`);
    } finally {
      // AudioData holds memory outside JavaScript; release it once copied.
      audioData.close();
    }
  }
}

/** Copy an AudioData's samples into one Float32Array per channel. */
function copyPlanes(audioData) {
  const channels = audioData.numberOfChannels;
  const frames = audioData.numberOfFrames;
  const planes = [];
  for (let ch = 0; ch < channels; ch++) planes.push(new Float32Array(frames));
  if (audioData.format === "f32") {
    // Interleaved (L R L R ...): copy once, then split into channels.
    const interleaved = new Float32Array(frames * channels);
    audioData.copyTo(interleaved, { planeIndex: 0, format: "f32" });
    for (let ch = 0; ch < channels; ch++) {
      const out = planes[ch];
      for (let i = 0; i < frames; i++) out[i] = interleaved[i * channels + ch];
    }
  } else {
    // Planar, or another sample format the browser converts for us.
    for (let ch = 0; ch < channels; ch++) {
      audioData.copyTo(planes[ch], { planeIndex: ch, format: "f32-planar" });
    }
  }
  return planes;
}

// DecodeAudioDataPlayer: the stream player for plain-http pages.
//
// decodeAudioData decodes a complete file and nothing smaller. So for every
// network read (normally one Ogg page = two 20 ms packets) we build a tiny but
// complete Ogg Opus file with OggOpusMuxer, decode it, and schedule the result
// exactly like the WebCodecs player schedules its chunks.
//
// Why pre-roll: every tiny file starts a brand-new decoder with no memory of
// the audio before it, and Opus leans on that memory. Besides blending the end
// of one frame into the next, it sends each frequency band's loudness as a
// *change* from the previous frame. A cold decoder starts from a wrong
// guess, and the error only halves or so with each frame. Played as is, every
// chunk would be a little too loud or too quiet in some bands, 25 times a
// second: a rough, warbling sound and clicks at the joins. So each file starts
// with the last PREROLL_PACKETS packets we already played: the decoder "warms
// up" on them, and by the time it reaches the new packets its state matches a
// decoder that had been running all along.
//
// Why 16 packets (320 ms): measured in Chrome against a continuous decoder,
// the largest sample error was 0.14 with 4 packets of pre-roll (clearly
// audible), 0.011 with 8, 0.0007 with 12, and 0.00004 with 16. 16 leaves a
// wide margin under the 0.001 we accept. It costs only decoding time (under
// 1 ms per chunk), never latency: the pre-roll is audio we already have.
// At the start of a connection there is less history than that; then the
// file simply holds every packet so far, which is exactly what a continuous
// decoder started at the same moment would have seen.
//
// Why the tail slice: the decoded file holds the pre-roll audio (already
// played) followed by the new packets. Each packet is 960 samples, so the new
// audio is exactly the last 960 * (new packets) samples of every channel. We
// keep those and throw the rest away. (The decoder also drops the OpusHead
// "pre-skip" from the start of each file; with pre-roll in front, that only
// eats into pre-roll we discard anyway.)
//
// Why an OfflineAudioContext at 48 kHz: decodeAudioData resamples its output
// to its context's sample rate. The main context may run at 44.1 kHz, and a
// resampled chunk would no longer be exactly 960 samples per packet, so the
// slice would land in the wrong place. A private 48 kHz context decodes at
// Opus's own rate; the source nodes resample when they play, if they must.
//
// Decoding is asynchronous and two decodes could finish out of order, so each
// connection keeps a promise chain: a chunk is scheduled only after the one
// before it. Every step checks the generation, so audio from a connection
// that has since ended is dropped, and nothing is decoded after setUrl("").
export const PREROLL_PACKETS = 16;

// An OpusTags packet with an empty vendor string and no comments, for streams
// that skip OpusTags (the file we hand the decoder must have one).
const EMPTY_OPUS_TAGS = new Uint8Array([
  0x4f, 0x70, 0x75, 0x73, 0x54, 0x61, 0x67, 0x73, 0, 0, 0, 0, 0, 0, 0, 0,
]);

export class DecodeAudioDataPlayer extends StreamPlayerBase {
  #offline = null; // private 48 kHz context, used only for decoding
  #muxer = null;
  #preroll = []; // the last PREROLL_PACKETS audio packets of this connection
  #chain = Promise.resolve(); // decodes of this connection, in order
  #warnedShort = false;

  /**
   * @param {object} opts
   * @param {AudioContext} opts.audioContext
   * @param {() => void} [opts.onGiveUp]  called once if the URL never works (see ReconnectPolicy)
   * @param {AudioNode} [opts.output]  where the sound goes; the latency probe passes a detector here
   */
  constructor(opts) {
    super(opts, "decodeAudioData player");
  }

  _onHalt() {
    this.#muxer = null;
    this.#preroll = [];
    this.#chain = Promise.resolve(); // the old chain's steps see a stale generation and stop
    this.#warnedShort = false;
  }

  _onHeader(opusHead, opusTags) {
    this.#muxer = new OggOpusMuxer(opusHead, opusTags ?? EMPTY_OPUS_TAGS);
  }

  _onPackets(packets, gen) {
    if (!this.#muxer) return;
    const prerollCount = this.#preroll.length;
    const file = this.#muxer.file([...this.#preroll, ...packets]);
    const preSkip = this.#muxer.preSkip;
    this.#preroll = [...this.#preroll, ...packets].slice(-PREROLL_PACKETS);
    this.#offline ??= new OfflineAudioContext(2, 1, OPUS_SAMPLE_RATE);
    // Start decoding at once (several files may decode side by side), but
    // schedule strictly in arrival order through the chain. decodeAudioData
    // takes ownership of (detaches) the buffer; `file` was built for this one
    // call, so nothing else needs it. The result is wrapped so a failure is a
    // value the chain handles, never an unhandled rejection.
    const decoding = this.#offline.decodeAudioData(file.buffer).then(
      (audio) => ({ audio }),
      (error) => ({ error }),
    );
    this.#chain = this.#chain.then(() => this.#play(decoding, prerollCount, packets.length, preSkip, gen));
  }

  /** Wait for one file's decode, cut out the new packets' samples, schedule them. */
  async #play(decoding, prerollCount, count, preSkip, gen) {
    const { audio, error } = await decoding;
    if (!this._isCurrent(gen)) return; // this connection ended meanwhile
    if (error) {
      this._reconnect(gen, `decodeAudioData failed: ${error?.message ?? error}`);
      return;
    }
    try {
      const wanted = SAMPLES_PER_PACKET * count;
      // Without enough pre-roll (the first chunk of a connection) the
      // pre-skip comes off the new packets, so a shorter result is expected
      // then; anything shorter than that is worth a warning.
      const expected = Math.max(0, Math.min(wanted, SAMPLES_PER_PACKET * (prerollCount + count) - preSkip));
      if (audio.length < expected && !this.#warnedShort) {
        this.#warnedShort = true;
        console.warn(
          `${MODULE_ID} | decodeAudioData returned ${audio.length} samples, expected at least ${expected}; playing what there is`,
        );
      }
      const take = Math.min(wanted, audio.length);
      const planes = [];
      for (let ch = 0; ch < audio.numberOfChannels; ch++) {
        planes.push(audio.getChannelData(ch).subarray(audio.length - take));
      }
      this._schedule(planes, audio.sampleRate, gen);
    } catch (err) {
      this._reconnect(gen, `playing decoded audio failed: ${err?.message ?? err}`);
    }
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

// One 20 ms stereo Opus packet of silence (CELT, fullband): a TOC byte 0xFC
// then two bytes that decode to silence.
const SILENT_OPUS_PACKET = new Uint8Array([0xfc, 0xff, 0xfe]);
const DECODE_PROBE_TIMEOUT_MS = 2000;

/**
 * True when decodeAudioData can decode an Ogg Opus file in this browser
 * (Chrome, Edge and Firefox can; Safari cannot). We simply try: mux one silent
 * packet into a tiny file and decode it. Never throws; any failure is "no".
 */
export async function isOggOpusDecodable(audioContext) {
  let timer = null;
  try {
    if (typeof OfflineAudioContext === "undefined") return false;
    if (typeof audioContext?.decodeAudioData !== "function") return false;
    // OpusHead: version 1, 2 channels, pre-skip 0, 48 kHz, gain 0, mapping 0.
    const head = new Uint8Array(19);
    for (let i = 0; i < OPUS_HEAD.length; i++) head[i] = OPUS_HEAD.charCodeAt(i);
    head[8] = 1;
    head[9] = 2;
    new DataView(head.buffer).setUint32(12, OPUS_SAMPLE_RATE, true);
    const file = new OggOpusMuxer(head, EMPTY_OPUS_TAGS).file([SILENT_OPUS_PACKET]);
    const ctx = new OfflineAudioContext(2, 1, OPUS_SAMPLE_RATE);
    // A browser that neither resolves nor rejects must not hold up startup.
    const timeout = new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error("timed out")), DECODE_PROBE_TIMEOUT_MS);
    });
    const decoded = await Promise.race([ctx.decodeAudioData(file.buffer), timeout]);
    return decoded.length > 0;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

/**
 * The best player this browser supports: WebCodecs (secure pages), else
 * decodeAudioData (plain http in Chrome, Edge, Firefox), else the <audio> player.
 */
export async function createPlayer({ audioContext, onGiveUp, output }) {
  if (await isWebCodecsOpusSupported()) {
    console.info(`${MODULE_ID} | using the low-latency WebCodecs player`);
    return new WebCodecsPlayer({ audioContext, onGiveUp, output });
  }
  if (await isOggOpusDecodable(audioContext)) {
    console.info(`${MODULE_ID} | WebCodecs Opus unavailable; using the low-latency decodeAudioData player`);
    return new DecodeAudioDataPlayer({ audioContext, onGiveUp, output });
  }
  console.info(`${MODULE_ID} | WebCodecs Opus and decodeAudioData Ogg Opus unavailable; using the <audio> player`);
  return new ElementPlayer();
}
