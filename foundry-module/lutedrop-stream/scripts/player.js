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
  if (a.length === 0) return b;
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

    // If the previous page left a packet open but this page doesn't continue
    // it (or vice versa), a page was lost; the half-packet is unusable.
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
