import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  ReconnectPolicy,
  BACKOFF_START_MS,
  BACKOFF_MAX_MS,
  STABLE_AFTER_MS,
  GIVE_UP_AFTER,
  STALL_TIMEOUT_MS,
} from '../lutedrop-stream/scripts/player.js';

test('exports_the_policy_constants', () => {
  assert.equal(BACKOFF_START_MS, 1000);
  assert.equal(BACKOFF_MAX_MS, 15000);
  assert.equal(STABLE_AFTER_MS, 10000);
  assert.equal(GIVE_UP_AFTER, 3);
  assert.equal(STALL_TIMEOUT_MS, 5000);
});

test('backoff_doubles_to_cap', () => {
  const p = new ReconnectPolicy();
  const delays = [1, 2, 3, 4, 5].map(() => p.nextDelayMs());
  assert.deepEqual(delays, [1000, 2000, 4000, 8000, 15000]);
});

test('stable_resets_backoff', () => {
  const p = new ReconnectPolicy();
  p.nextDelayMs();
  p.nextDelayMs();
  p.markStable();
  assert.equal(p.nextDelayMs(), 1000);
});

test('gives_up_after_three_failed_connects_with_no_bytes', () => {
  const p = new ReconnectPolicy();
  assert.equal(p.markFailure(), false);
  assert.equal(p.markFailure(), false);
  assert.equal(p.markFailure(), true);
});

test('never_gives_up_once_bytes_arrived', () => {
  const p = new ReconnectPolicy();
  p.markBytesReceived();
  for (let i = 0; i < 10; i++) assert.equal(p.markFailure(), false);
});

test('reset_clears_failures', () => {
  const p = new ReconnectPolicy();
  p.markFailure();
  p.markFailure();
  p.reset();
  assert.equal(p.markFailure(), false);
  assert.equal(p.markFailure(), false);
});
