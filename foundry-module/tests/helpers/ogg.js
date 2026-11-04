// Test-only helper: builds real Ogg pages so the demuxer's CRC check is
// exercised against genuine bytes rather than a stand-in.

// Ogg's CRC-32: polynomial 0x04c11db7, bits fed most-significant first (no
// reflection), starting at 0, no final XOR. Not the same as zip's CRC-32.
const CRC_TABLE = new Uint32Array(256);
for (let i = 0; i < 256; i++) {
  let r = i << 24;
  for (let k = 0; k < 8; k++) {
    r = r & 0x80000000 ? (r << 1) ^ 0x04c11db7 : r << 1;
  }
  CRC_TABLE[i] = r >>> 0;
}

function crc32(bytes) {
  let crc = 0;
  for (const b of bytes) {
    crc = ((crc << 8) ^ CRC_TABLE[((crc >>> 24) ^ b) & 0xff]) >>> 0;
  }
  return crc;
}

export function concat(...arrays) {
  const out = new Uint8Array(arrays.reduce((n, a) => n + a.length, 0));
  let at = 0;
  for (const a of arrays) {
    out.set(a, at);
    at += a.length;
  }
  return out;
}

// `segments` are packet pieces. Each piece is laid out as lacing values of 255
// for every full 255 bytes, then one final value (< 255) that ends the packet.
// To leave the last piece open so the packet continues on the next page, pass
// `leaveLastOpen: true`; its length must then be a multiple of 255 and no
// terminating lacing value is written.
export function buildPage({
  serial = 1,
  seq,
  granule,
  bos = false,
  continued = false,
  segments,
  leaveLastOpen = false,
}) {
  const lacing = [];
  segments.forEach((seg, idx) => {
    let left = seg.length;
    while (left >= 255) {
      lacing.push(255);
      left -= 255;
    }
    const isLast = idx === segments.length - 1;
    if (!(isLast && leaveLastOpen)) lacing.push(left);
  });
  const body = concat(...segments);
  const header = new Uint8Array(27 + lacing.length);
  const view = new DataView(header.buffer);
  header.set([0x4f, 0x67, 0x67, 0x53], 0); // "OggS"
  header[4] = 0; // version
  header[5] = (continued ? 0x01 : 0) | (bos ? 0x02 : 0);
  view.setBigInt64(6, BigInt(granule), true);
  view.setUint32(14, serial, true);
  view.setUint32(18, seq, true);
  // CRC at 22..25 stays zero while we compute it.
  header[26] = lacing.length;
  header.set(lacing, 27);
  const page = concat(header, body);
  new DataView(page.buffer).setUint32(22, crc32(page), true);
  return page;
}
