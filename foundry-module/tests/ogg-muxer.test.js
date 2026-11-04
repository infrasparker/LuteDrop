import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  OggDemuxer,
  OggOpusMuxer,
  isOggOpusDecodable,
  PREROLL_PACKETS,
} from '../lutedrop-stream/scripts/player.js';
import { concat } from './helpers/ogg.js';

// A realistic OpusHead: "OpusHead", version 1, 2 channels, pre-skip 312,
// input rate 48000, gain 0, channel mapping family 0.
const PRE_SKIP = 312;
function opusHead() {
  const head = new Uint8Array(19);
  head.set(new TextEncoder().encode('OpusHead'), 0);
  const view = new DataView(head.buffer);
  head[8] = 1;
  head[9] = 2;
  view.setUint16(10, PRE_SKIP, true);
  view.setUint32(12, 48000, true);
  return head;
}

// "OpusTags", vendor string "test", no comments.
function opusTags() {
  const vendor = new TextEncoder().encode('test');
  const counts = new Uint8Array(8);
  new DataView(counts.buffer).setUint32(0, vendor.length, true);
  const tail = new Uint8Array(4); // comment count 0
  return concat(new TextEncoder().encode('OpusTags'), counts.subarray(0, 4), vendor, tail);
}

// Sizes chosen to hit every lacing edge: < 255, > 255, exactly 255 (needs a
// trailing 0 lacing value), 1 byte, 256.
const SIZES = [50, 300, 255, 1, 256, 10];
function fakePackets() {
  return SIZES.map((n, k) => Uint8Array.from({ length: n }, (_, i) => (i * 7 + k * 31) & 0xff));
}

/** Split a file into its raw pages, reading each header by hand. */
function parsePages(file) {
  const pages = [];
  let at = 0;
  while (at < file.length) {
    const view = new DataView(file.buffer, file.byteOffset + at);
    assert.equal(String.fromCharCode(...file.subarray(at, at + 4)), 'OggS');
    const nsegs = file[at + 26];
    let body = 0;
    for (let i = 0; i < nsegs; i++) body += file[at + 27 + i];
    const len = 27 + nsegs + body;
    pages.push({
      headerType: file[at + 5],
      granule: Number(view.getBigInt64(6, true)),
      serial: view.getUint32(14, true),
      seq: view.getUint32(18, true),
      bytes: file.subarray(at, at + len),
    });
    at += len;
  }
  return pages;
}

test('muxer_file_round_trips_through_the_demuxer', () => {
  const head = opusHead();
  const tags = opusTags();
  const packets = fakePackets();
  const file = new OggOpusMuxer(head, tags).file(packets);

  const out = new OggDemuxer().feed(file);
  assert.equal(out.length, 2 + packets.length);
  assert.deepEqual(out[0].data, head);
  assert.equal(out[0].bos, true);
  assert.deepEqual(out[1].data, tags);
  assert.equal(out[1].bos, false);
  for (let i = 0; i < packets.length; i++) {
    assert.deepEqual(out[2 + i].data, packets[i], `packet ${i}`);
    assert.equal(out[2 + i].bos, false);
    assert.equal(out[2 + i].granule, PRE_SKIP + 5760);
  }
});

test('muxer_pages_have_valid_crcs_and_sequence', () => {
  const file = new OggOpusMuxer(opusHead(), opusTags()).file(fakePackets());
  const pages = parsePages(file);
  assert.equal(pages.length, 3);
  assert.deepEqual(pages.map((p) => p.seq), [0, 1, 2]);
  assert.deepEqual(pages.map((p) => p.granule), [0, 0, PRE_SKIP + 5760]);
  assert.equal(pages[0].headerType, 0x02, 'first page is bos only');
  assert.equal(pages[1].headerType, 0x00, 'tags page has no flags');
  assert.equal(pages[2].headerType, 0x04, 'last page is eos only');
  assert.equal(new Set(pages.map((p) => p.serial)).size, 1, 'one logical stream');
  // The demuxer drops any page whose CRC is wrong, so each page fed on its
  // own must still yield its packets.
  const counts = pages.map((p) => new OggDemuxer().feed(p.bytes).length);
  assert.deepEqual(counts, [1, 1, SIZES.length]);
});

test('muxer_lacing_ends_a_255_byte_packet_with_a_zero', () => {
  const file = new OggOpusMuxer(opusHead(), opusTags()).file([new Uint8Array(255)]);
  const audio = parsePages(file)[2].bytes;
  assert.equal(audio[26], 2);
  assert.deepEqual([...audio.subarray(27, 29)], [255, 0]);
});

test('muxer_spreads_more_than_255_lacing_values_over_several_pages', () => {
  // A burst: 70 packets of 1000 bytes need 4 lacing values each (280 in
  // all), more than the 255 one page can hold.
  const packets = Array.from({ length: 70 }, (_, k) => new Uint8Array(1000).fill(k));
  const file = new OggOpusMuxer(opusHead(), opusTags()).file(packets);
  const pages = parsePages(file);
  assert.ok(pages.length > 3);
  assert.deepEqual(pages.map((p) => p.seq), pages.map((_, i) => i));
  const last = pages[pages.length - 1];
  assert.equal(last.headerType, 0x04);
  assert.equal(last.granule, PRE_SKIP + 960 * 70);
  for (const p of pages.slice(2, -1)) assert.equal(p.headerType, 0x00);
  const out = new OggDemuxer().feed(file).slice(2);
  assert.equal(out.length, 70);
  out.forEach((p, k) => assert.deepEqual(p.data, packets[k]));
});

test('isOggOpusDecodable_is_a_function', () => {
  assert.equal(typeof isOggOpusDecodable, 'function');
  assert.equal(PREROLL_PACKETS, 16);
});
