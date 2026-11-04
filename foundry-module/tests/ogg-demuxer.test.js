import { test } from 'node:test';
import assert from 'node:assert/strict';
import { OggDemuxer } from '../lutedrop-stream/scripts/player.js';
import { buildPage, concat } from './helpers/ogg.js';

function bytes(n, fill) {
  return new Uint8Array(n).fill(fill);
}

test('yields_whole_packets_from_one_page', () => {
  const a = bytes(10, 1);
  const b = bytes(300, 2);
  const page = buildPage({ seq: 2, granule: 960, segments: [a, b] });
  const out = new OggDemuxer().feed(page);
  assert.equal(out.length, 2);
  assert.deepEqual(out[0].data, a);
  assert.deepEqual(out[1].data, b);
  assert.equal(out[0].granule, 960);
  assert.equal(out[1].granule, 960);
});

test('splits_a_page_header_across_feeds', () => {
  const a = bytes(10, 1);
  const b = bytes(300, 2);
  const page = buildPage({ seq: 2, granule: 960, segments: [a, b] });
  const d = new OggDemuxer();
  const out = [
    ...d.feed(page.subarray(0, 1)),
    ...d.feed(page.subarray(1, 13)),
    ...d.feed(page.subarray(13, 20)),
    ...d.feed(page.subarray(20)),
  ];
  assert.equal(out.length, 2);
  assert.deepEqual(out[0].data, a);
  assert.deepEqual(out[1].data, b);
});

test('joins_a_packet_spanning_two_pages', () => {
  const whole = Uint8Array.from({ length: 600 }, (_, i) => i % 251);
  // Page 1 carries 255+255 and leaves the packet open; page 2 carries the last 90.
  const p1 = buildPage({
    seq: 2,
    granule: -1,
    segments: [whole.subarray(0, 510)],
    leaveLastOpen: true,
  });
  const p2 = buildPage({
    seq: 3,
    granule: 1920,
    continued: true,
    segments: [whole.subarray(510)],
  });
  const out = new OggDemuxer().feed(concat(p1, p2));
  assert.equal(out.length, 1);
  assert.deepEqual(out[0].data, whole);
  assert.equal(out[0].granule, 1920);
});

test('marks_header_packets_with_bos', () => {
  const head = new TextEncoder().encode('OpusHead\x01\x02');
  const p1 = buildPage({ seq: 0, granule: 0, bos: true, segments: [head] });
  const p2 = buildPage({ seq: 2, granule: 960, segments: [bytes(20, 7)] });
  const out = new OggDemuxer().feed(concat(p1, p2));
  assert.equal(out.length, 2);
  assert.equal(out[0].bos, true);
  assert.equal(out[1].bos, false);
});

test('skips_garbage_before_capture_pattern', () => {
  const garbage = Uint8Array.from({ length: 37 }, (_, i) => ((i * 73 + 11) % 200) + 1);
  const a = bytes(40, 9);
  const page = buildPage({ seq: 2, granule: 960, segments: [a] });
  const out = new OggDemuxer().feed(concat(garbage, page));
  assert.equal(out.length, 1);
  assert.deepEqual(out[0].data, a);
});

test('rejects_bad_crc_and_resyncs', () => {
  const bad = buildPage({ seq: 2, granule: 960, segments: [bytes(50, 3)] });
  bad[bad.length - 5] ^= 0xff; // flip one body byte
  const goodData = bytes(30, 4);
  const good = buildPage({ seq: 3, granule: 1920, segments: [goodData] });
  const out = new OggDemuxer().feed(concat(bad, good));
  assert.equal(out.length, 1);
  assert.deepEqual(out[0].data, goodData);
  assert.equal(out[0].granule, 1920);
});

test('reset_drops_partial_state', () => {
  const half = buildPage({ seq: 2, granule: 960, segments: [bytes(100, 5)] });
  const fullData = bytes(25, 6);
  const full = buildPage({ seq: 3, granule: 1920, segments: [fullData] });
  const d = new OggDemuxer();
  assert.equal(d.feed(half.subarray(0, half.length / 2)).length, 0);
  d.reset();
  const out = d.feed(full);
  assert.equal(out.length, 1);
  assert.deepEqual(out[0].data, fullData);
});
