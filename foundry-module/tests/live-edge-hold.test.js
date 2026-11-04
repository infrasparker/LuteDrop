import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  LiveEdgeHold,
  HOLD_TARGET_S,
  HOLD_MAX_GAP_S,
  HOLD_CATCHUP_RATE,
  HOLD_POLL_MS,
  HOLD_STALL_S,
} from '../lutedrop-stream/scripts/player.js';

// Each poll describes what the <audio> element shows: where it is playing
// (currentTime), how far it has downloaded (bufferedEnd), and the wall clock
// (now). All in seconds. Polls are 0.25 s apart, like the real interval.

test('exports_the_hold_constants', () => {
  assert.equal(HOLD_TARGET_S, 0.5);
  assert.equal(HOLD_MAX_GAP_S, 1.0);
  assert.equal(HOLD_CATCHUP_RATE, 1.15);
  assert.equal(HOLD_POLL_MS, 250);
  assert.equal(HOLD_STALL_S, 3);
});

test('seeks_when_gap_exceeds_max', () => {
  const hold = new LiveEdgeHold();
  assert.deepEqual(hold.poll({ currentTime: 10, bufferedEnd: 12.5, now: 100 }), { action: 'seek', to: 12.0 });
});

test('falls_back_to_rate_when_seek_did_not_take', () => {
  const hold = new LiveEdgeHold();
  hold.poll({ currentTime: 10, bufferedEnd: 12.5, now: 100 });
  // The seek was ignored: time moved on normally and the gap is unchanged.
  assert.deepEqual(hold.poll({ currentTime: 10.25, bufferedEnd: 12.75, now: 100.25 }), {
    action: 'rate',
    rate: 1.15,
  });
});

test('keeps_rate_until_caught_up_then_normal', () => {
  const hold = new LiveEdgeHold();
  hold.poll({ currentTime: 10, bufferedEnd: 12.5, now: 100 }); // seek
  hold.poll({ currentTime: 10.25, bufferedEnd: 12.75, now: 100.25 }); // rate
  assert.deepEqual(hold.poll({ currentTime: 11, bufferedEnd: 11.8, now: 101 }), { action: 'rate', rate: 1.15 });
  assert.deepEqual(hold.poll({ currentTime: 11.4, bufferedEnd: 11.8, now: 101.25 }), { action: 'normal' });
  assert.deepEqual(hold.poll({ currentTime: 11.65, bufferedEnd: 12.05, now: 101.5 }), { action: 'none' });
});

test('does_nothing_within_target', () => {
  const hold = new LiveEdgeHold();
  assert.deepEqual(hold.poll({ currentTime: 10, bufferedEnd: 10.4, now: 100 }), { action: 'none' });
  assert.deepEqual(hold.poll({ currentTime: 10.25, bufferedEnd: 10.65, now: 100.25 }), { action: 'none' });
});

test('reconnects_when_time_stalls', () => {
  const hold = new LiveEdgeHold();
  // currentTime stuck at 10 while the wall clock moves on.
  assert.deepEqual(hold.poll({ currentTime: 10, bufferedEnd: 10.4, now: 100 }), { action: 'none' });
  assert.deepEqual(hold.poll({ currentTime: 10, bufferedEnd: 10.4, now: 101 }), { action: 'none' });
  assert.deepEqual(hold.poll({ currentTime: 10, bufferedEnd: 10.4, now: 102 }), { action: 'none' });
  assert.deepEqual(hold.poll({ currentTime: 10, bufferedEnd: 10.4, now: 103 }), { action: 'reconnect' });
});

test('reset_clears_state', () => {
  const hold = new LiveEdgeHold();
  hold.poll({ currentTime: 10, bufferedEnd: 12.5, now: 100 }); // seek
  hold.reset();
  // A new connection: a big gap is answered with a seek again, not with rate.
  assert.deepEqual(hold.poll({ currentTime: 0, bufferedEnd: 3, now: 100.25 }), { action: 'seek', to: 2.5 });
});
