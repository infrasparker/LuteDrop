// DecodeAudioDataPlayer's give-up rule, run against fakes: a fake fetch() that
// serves a tiny Ogg Opus stream, a fake AudioContext, a fake
// OfflineAudioContext whose decodeAudioData we control, and node:test's mock
// timers so the reconnect delays (1 s, 2 s, ...) pass instantly.
import { test, mock, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { setImmediate as nextTurn } from 'node:timers/promises';
import {
  DecodeAudioDataPlayer,
  DECODE_GIVE_UP_AFTER,
  DECODE_TIMEOUT_MS,
} from '../lutedrop-stream/scripts/player.js';
import { buildPage } from './helpers/ogg.js';

const GIVE_UP_LINE =
  'lutedrop-stream | decodeAudioData player giving up: 3 decodes failed in a row and none ever succeeded';

function ascii(text, extra = 0) {
  const out = new Uint8Array(text.length + extra);
  for (let i = 0; i < text.length; i++) out[i] = text.charCodeAt(i);
  return out;
}

// OpusHead: version 1, 2 channels, pre-skip 0, 48 kHz.
const OPUS_HEAD = ascii('OpusHead', 11);
OPUS_HEAD[8] = 1;
OPUS_HEAD[9] = 2;
new DataView(OPUS_HEAD.buffer).setUint32(12, 48000, true);
const OPUS_TAGS = ascii('OpusTags', 8);
const SILENT_PACKET = new Uint8Array([0xfc, 0xff, 0xfe]);

const HEADER_PAGES = [
  buildPage({ seq: 0, granule: 0, bos: true, segments: [OPUS_HEAD] }),
  buildPage({ seq: 1, granule: 0, segments: [OPUS_TAGS] }),
];
let audioSeq = 2;
function audioPage() {
  const seq = audioSeq++;
  return buildPage({ seq, granule: seq * 960, segments: [SILENT_PACKET] });
}

// ---- fakes ----

let decodeImpl; // what OfflineAudioContext.decodeAudioData does in this test
let fetchCount;
let liveStream; // controller of the newest stream response, for pump()
let warnings;
let infos;

function fakeAudioContext() {
  const node = () => ({ connect() {}, disconnect() {} });
  return {
    currentTime: 0,
    destination: node(),
    resume: async () => {},
    createGain: () => ({ ...node(), gain: { value: 0, setTargetAtTime() {} } }),
    createBuffer: () => ({ copyToChannel() {} }),
    createBufferSource: () => ({ ...node(), addEventListener() {}, start() {}, stop() {} }),
  };
}

function fakeDecoded() {
  return { length: 960, numberOfChannels: 2, sampleRate: 48000, getChannelData: () => new Float32Array(960) };
}

beforeEach(() => {
  fetchCount = 0;
  liveStream = null;
  warnings = [];
  infos = [];
  mock.method(console, 'warn', (...args) => warnings.push(args.join(' ')));
  mock.method(console, 'info', (...args) => infos.push(args.join(' ')));
  mock.timers.enable({ apis: ['setTimeout'] });
  globalThis.OfflineAudioContext = class {
    decodeAudioData(buffer) {
      return decodeImpl(buffer);
    }
  };
  globalThis.fetch = async (_url, { signal } = {}) => {
    fetchCount++;
    let controller;
    const body = new ReadableStream({
      start(c) {
        controller = c;
        for (const page of [...HEADER_PAGES, audioPage()]) c.enqueue(page);
      },
    });
    signal?.addEventListener('abort', () => {
      try {
        controller.error(new DOMException('aborted', 'AbortError'));
      } catch {
        // already closed
      }
    });
    liveStream = controller;
    return new Response(body, { status: 200 });
  };
});

afterEach(() => {
  mock.timers.reset();
  mock.restoreAll();
  delete globalThis.OfflineAudioContext;
  delete globalThis.fetch;
});

/** Let pending promise callbacks run (setImmediate is not mocked). */
async function flush() {
  for (let i = 0; i < 30; i++) await nextTurn();
}

/**
 * Move the fake clock on by `ms`, in 500 ms steps. Each step also sends one
 * more audio page on the live connection, as a real stream would, so the
 * player's 5 s "no data" timer never fires.
 */
async function advance(ms) {
  await flush();
  for (let t = 0; t < ms; t += 500) {
    mock.timers.tick(500);
    try {
      liveStream?.enqueue(audioPage());
    } catch {
      // that connection was aborted
    }
    await flush();
  }
}

function startPlayer() {
  let gaveUp = 0;
  const player = new DecodeAudioDataPlayer({ audioContext: fakeAudioContext(), onGiveUp: () => gaveUp++ });
  player.setUrl('http://example.test/stream');
  player.start();
  return { player, gaveUps: () => gaveUp };
}

// ---- tests ----

test('exports_the_decode_constants', () => {
  assert.equal(DECODE_GIVE_UP_AFTER, 3);
  assert.equal(DECODE_TIMEOUT_MS, 5000);
});

test('gives_up_after_three_failed_decodes_when_none_ever_succeeded', async () => {
  decodeImpl = () => Promise.reject(new Error('EncodingError'));
  const { player, gaveUps } = startPlayer();
  await advance(20000);
  assert.equal(gaveUps(), 1);
  assert.equal(fetchCount, 3, 'one connection per failed decode, none after giving up');
  assert.ok(warnings.includes(GIVE_UP_LINE), `warnings were: ${warnings.join(' / ')}`);
  player.setUrl('');
});

test('never_gives_up_once_a_decode_has_succeeded', async () => {
  let calls = 0;
  decodeImpl = () => (calls++ === 0 ? Promise.resolve(fakeDecoded()) : Promise.reject(new Error('EncodingError')));
  const { player, gaveUps } = startPlayer();
  await advance(60000);
  assert.equal(gaveUps(), 0);
  assert.ok(fetchCount > DECODE_GIVE_UP_AFTER, `only ${fetchCount} connections`);
  player.setUrl('');
});

test('a_decode_that_never_finishes_times_out_and_counts_as_a_failure', async () => {
  decodeImpl = () => new Promise(() => {});
  const { player, gaveUps } = startPlayer();
  await advance(40000);
  assert.equal(gaveUps(), 1);
  assert.ok(warnings.includes(GIVE_UP_LINE));
  assert.ok(
    infos.some((line) => line.includes('decodeAudioData failed') && line.includes('timed out')),
    `infos were: ${infos.join(' / ')}`,
  );
  player.setUrl('');
});
