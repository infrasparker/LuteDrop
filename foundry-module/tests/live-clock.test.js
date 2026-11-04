import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  LiveClock,
  TARGET_LEAD_S,
  LEAD_STEP_S,
  MAX_LEAD_S,
  CATCHUP_SLACK_S,
} from '../lutedrop-stream/scripts/player.js';

// Floating-point sums like 0.2 + 0.02 are not exact, so compare with a tolerance.
function near(actual, expected) {
  assert.ok(Math.abs(actual - expected) < 1e-9, `expected ${expected}, got ${actual}`);
}

// A fake clock: tests move time by assigning state.t.
function makeClock() {
  const state = { t: 0 };
  const clock = new LiveClock(() => state.t);
  return { state, clock };
}

test('exports_the_policy_constants', () => {
  near(TARGET_LEAD_S, 0.2);
  near(LEAD_STEP_S, 0.1);
  near(MAX_LEAD_S, 0.6);
  near(CATCHUP_SLACK_S, 0.4);
});

test('first_chunk_starts_target_lead_ahead', () => {
  const { clock } = makeClock();
  const p = clock.place(0.02);
  near(p.at, 0.2);
  assert.equal(p.restart, true);
  assert.equal(p.catchUp, false);
  near(clock.lead, 0.2);
});

test('steady_state_appends_back_to_back', () => {
  const { clock } = makeClock();
  clock.place(0.02);
  const p = clock.place(0.02);
  near(p.at, 0.22);
  assert.equal(p.restart, false);
  assert.equal(p.catchUp, false);
});

test('underrun_restarts_and_grows_lead', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  state.t = 5;
  const p = clock.place(0.02);
  near(p.at, 5.3);
  assert.equal(p.restart, true);
  near(clock.lead, 0.3);
});

test('lead_caps_at_max', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  for (let i = 1; i <= 6; i++) {
    state.t = i * 10;
    clock.place(0.02);
  }
  near(clock.lead, 0.6);
});

test('catch_up_when_far_ahead', () => {
  const { clock } = makeClock();
  clock.place(0.02);
  let jump = null;
  for (let i = 0; i < 40 && !jump; i++) {
    const p = clock.place(0.02);
    if (p.catchUp) jump = p;
  }
  assert.ok(jump, 'a catch-up should have been requested');
  near(jump.at, 0.2);
  assert.equal(jump.restart, false);
  const next = clock.place(0.02);
  near(next.at, 0.22);
  assert.equal(next.catchUp, false);
  assert.equal(next.restart, false);
});

test('reset_restores_target_lead', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  state.t = 5;
  clock.place(0.02);
  state.t = 10;
  clock.place(0.02);
  assert.ok(clock.lead > 0.2);
  clock.reset();
  near(clock.lead, 0.2);
  assert.equal(clock.place(0.02).restart, true);
});
