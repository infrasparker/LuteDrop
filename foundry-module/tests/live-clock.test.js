import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  LiveClock,
  TARGET_LEAD_S,
  LEAD_STEP_S,
  MAX_LEAD_S,
  CATCHUP_SLACK_S,
  LEAD_DECAY_AFTER_S,
  DRIFT_SLACK_S,
  DRIFT_AFTER_S,
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
  near(clock.lead, 0.2); // a catch-up moves the queue, never the cushion
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

// ---- cushion decay and sustained drift ----

// Steady playback: time moves on by exactly the length of each 20 ms chunk, so
// the amount of audio queued ahead of "now" stays the same. Runs until state.t
// reaches `until` (seconds) and returns every placement with its time `t`.
// Time is computed from a counter, not summed, so float error cannot pile up.
function steady(state, clock, until) {
  const start = state.t;
  const results = [];
  for (let i = 1; start + i * 0.02 <= until + 1e-9; i++) {
    state.t = start + i * 0.02;
    results.push({ t: state.t, ...clock.place(0.02) });
  }
  return results;
}

test('exports_the_decay_and_drift_constants', () => {
  assert.equal(LEAD_DECAY_AFTER_S, 60);
  near(DRIFT_SLACK_S, 0.1);
  assert.equal(DRIFT_AFTER_S, 5);
});

test('lead_decays_after_a_quiet_minute', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  state.t = 5;
  clock.place(0.02); // underrun: 0.3
  state.t = 10;
  clock.place(0.02); // underrun: 0.4
  near(clock.lead, 0.4);
  steady(state, clock, 69.9);
  near(clock.lead, 0.4); // not yet a full quiet minute
  steady(state, clock, 70.1);
  near(clock.lead, 0.3);
  steady(state, clock, 129.9);
  near(clock.lead, 0.3);
  steady(state, clock, 130.1);
  near(clock.lead, 0.2);
  steady(state, clock, 400);
  near(clock.lead, 0.2); // never below the target
});

test('underrun_resets_the_decay_timer', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  state.t = 5;
  clock.place(0.02); // underrun at 5 s: 0.3
  steady(state, clock, 50);
  state.t = 55;
  clock.place(0.02); // underrun at 55 s: 0.4, and the quiet minute starts over
  near(clock.lead, 0.4);
  // Counting from 5 s it would have decayed at 65 s; counting from 55 s, not before 115 s.
  steady(state, clock, 114.9);
  near(clock.lead, 0.4);
  steady(state, clock, 115.1);
  near(clock.lead, 0.3);
});

test('sustained_small_drift_triggers_catch_up', () => {
  const { state, clock } = makeClock();
  clock.place(0.02); // queued up to 0.22
  clock.place(0.15); // queued up to 0.37: from now on 0.35 s is pending at each placement
  // 0.35 is above lead + DRIFT_SLACK_S (0.3) but under the immediate
  // catch-up threshold (lead + CATCHUP_SLACK_S = 0.6).
  const results = steady(state, clock, 8);
  const jumps = results.filter((p) => p.catchUp);
  assert.equal(jumps.length, 1, 'exactly one catch-up');
  const jump = jumps[0];
  // Drift is first seen at t = 0.02, so the jump comes DRIFT_AFTER_S later.
  assert.ok(jump.t >= 5.02 - 1e-9 && jump.t < 5.05, `jumped at ${jump.t}`);
  near(jump.at, jump.t + 0.2); // back to now + lead
  assert.equal(jump.restart, false);
  near(clock.lead, 0.2);
  // After the jump: plain back-to-back appends.
  const i = results.indexOf(jump);
  for (let k = i + 1; k < results.length; k++) {
    near(results[k].at, results[k - 1].at + 0.02);
    assert.equal(results[k].restart, false);
  }
});

test('brief_drift_does_not_trigger', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  clock.place(0.15); // 0.35 s pending at each placement, as above
  let caughtUp = steady(state, clock, 2).some((p) => p.catchUp);
  // Then the network slows a little: 40 ms pass per 20 ms chunk until the
  // queue is back near the 0.2 s cushion.
  for (let i = 1; i <= 8; i++) {
    state.t = 2 + i * 0.04;
    caughtUp ||= clock.place(0.02).catchUp;
  }
  caughtUp ||= steady(state, clock, 20).some((p) => p.catchUp);
  assert.equal(caughtUp, false);
  near(clock.lead, 0.2);
});
