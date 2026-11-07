import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  LiveClock,
  TARGET_LEAD_S,
  LEAD_STEP_S,
  MAX_LEAD_S,
  LEAD_DECAY_AFTER_S,
  MAX_RATE_DEV,
  RATE_GAIN_PER_S,
  ERROR_SMOOTHING_S,
  SKIP_SLACK_S,
  SKIP_RESUME_SLACK_S,
} from '../lutedrop-stream/scripts/player.js';

// Floating-point sums like 0.2 + 0.02 are not exact, so compare with a tolerance.
function near(actual, expected, tol = 1e-9) {
  assert.ok(Math.abs(actual - expected) < tol, `expected ${expected}, got ${actual}`);
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
  assert.equal(LEAD_DECAY_AFTER_S, 60);
  near(MAX_RATE_DEV, 0.003);
  near(RATE_GAIN_PER_S, 0.1);
  near(ERROR_SMOOTHING_S, 1);
  near(SKIP_SLACK_S, 1.5);
  near(SKIP_RESUME_SLACK_S, 0.05);
});

test('first_chunk_starts_target_lead_ahead', () => {
  const { clock } = makeClock();
  const p = clock.place(0.02);
  near(p.at, 0.2);
  assert.equal(p.rate, 1);
  assert.equal(p.restart, true);
  assert.equal(p.skip, false);
  near(clock.lead, 0.2);
});

test('steady_state_appends_back_to_back', () => {
  const { clock } = makeClock();
  clock.place(0.02);
  const p = clock.place(0.02);
  near(p.at, 0.22);
  assert.equal(p.rate, 1); // exactly on the cushion: normal speed
  assert.equal(p.restart, false);
  assert.equal(p.skip, false);
});

test('underrun_restarts_and_grows_lead', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  state.t = 5;
  const p = clock.place(0.02);
  near(p.at, 5.3);
  assert.equal(p.restart, true);
  assert.equal(p.rate, 1);
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
  assert.equal(clock.pending, 0);
  assert.equal(clock.place(0.02).restart, true);
});

// ---- speed control ----

test('too_much_queued_plays_faster_and_too_little_slower', () => {
  // Too much: half a second extra queued.
  const fast = makeClock();
  fast.clock.place(0.02);
  fast.clock.place(0.5); // queued up to 0.72
  fast.state.t = 0.3; // pending 0.42, i.e. 0.22 over the cushion
  const p = fast.clock.place(0.02);
  assert.ok(p.rate > 1, `rate ${p.rate}`);
  assert.ok(p.rate <= 1 + MAX_RATE_DEV + 1e-12);

  // Too little: time runs ahead of the queue, but not into an underrun.
  const slow = makeClock();
  slow.clock.place(0.02); // queued up to 0.22
  slow.state.t = 0.15; // pending 0.07, i.e. 0.13 under the cushion
  const q = slow.clock.place(0.02);
  assert.ok(q.rate < 1, `rate ${q.rate}`);
  assert.ok(q.rate >= 1 - MAX_RATE_DEV - 1e-12);
});

test('rate_is_clamped', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  clock.place(1.2); // queued up to 1.42: far beyond the clamp, under the skip line
  state.t = 1; // pending 0.42; a second between readings, so the smoothing takes most of it
  const p = clock.place(0.02);
  near(p.rate, 1 + MAX_RATE_DEV);
});

test('placement_stays_gapless_with_rates_other_than_one', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  clock.place(0.5); // lots queued, so the following chunks play faster
  let prev = null;
  let sawFast = false;
  for (let i = 1; i <= 200; i++) {
    state.t = i * 0.02;
    const p = clock.place(0.02);
    assert.equal(p.skip, false);
    assert.equal(p.restart, false);
    if (prev) near(p.at, prev.at + 0.02 / prev.rate, 1e-12); // next start = at + d / r
    if (p.rate !== 1) sawFast = true;
    prev = p;
  }
  assert.ok(sawFast, 'the rate should have left 1');
  // And slower than 1: let the queue run low.
  const s = makeClock();
  s.clock.place(0.02);
  s.state.t = 0.18; // 0.04 pending, well under the cushion
  const a = s.clock.place(0.02);
  assert.ok(a.rate < 1);
  const b = s.clock.place(0.02);
  near(b.at, a.at + 0.02 / a.rate, 1e-12);
});

// ---- cushion decay ----

// Steady playback: time moves on by exactly the length of each 20 ms chunk, so
// the amount of audio queued ahead of "now" stays (nearly) the same. Runs until
// state.t reaches `until` (seconds) and returns every placement with its time
// `t`. Time is computed from a counter, not summed, so float error cannot pile up.
function steady(state, clock, until) {
  const start = state.t;
  const results = [];
  for (let i = 1; start + i * 0.02 <= until + 1e-9; i++) {
    state.t = start + i * 0.02;
    results.push({ t: state.t, ...clock.place(0.02) });
  }
  return results;
}

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

test('decay_drains_smoothly_without_any_jump', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  state.t = 5;
  clock.place(0.02); // underrun: cushion 0.3
  const results = steady(state, clock, 200); // decays to 0.2 at 65 s
  near(clock.lead, 0.2);
  assert.ok(results.every((p) => !p.skip && !p.restart), 'no skip, no underrun');
  for (let k = 1; k < results.length; k++) {
    near(results[k].at, results[k - 1].at + 0.02 / results[k - 1].rate, 1e-9);
  }
  // By 200 s the extra 0.1 s has drained away.
  assert.ok(Math.abs(clock.pending - clock.lead) < 0.03, `pending ${clock.pending}`);
});

// ---- big bursts ----

test('a_huge_backlog_skips_incoming_chunks_then_resumes_gaplessly', () => {
  const { state, clock } = makeClock();
  clock.place(0.02);
  // A burst: 3 s of chunks all at once.
  const burst = [];
  for (let i = 0; i < 150; i++) burst.push(clock.place(0.02));
  const firstSkip = burst.findIndex((p) => p.skip);
  assert.ok(firstSkip > 0, 'a skip should have been requested');
  // Everything placed before the skip stays exactly where it was put: nothing
  // already scheduled moves or stops.
  const kept = burst.slice(0, firstSkip);
  const lastKept = kept.at(-1);
  const queuedUntil = lastKept.at + 0.02 / lastKept.rate;
  assert.ok(queuedUntil - state.t > clock.lead + SKIP_SLACK_S);
  // The rest of the burst is dropped, with no position.
  for (const p of burst.slice(firstSkip)) {
    assert.equal(p.skip, true);
    assert.equal(p.at, null);
  }
  near(clock.lead, 0.2); // a skip never touches the cushion
  // Live chunks keep arriving (one per 20 ms) and are dropped until the queue
  // is back near the cushion; then they continue right where the queue ends.
  let resumed = null;
  for (let i = 1; i <= 200 && !resumed; i++) {
    state.t = i * 0.02;
    const p = clock.place(0.02);
    if (!p.skip) resumed = p;
  }
  assert.ok(resumed, 'playback should have resumed');
  assert.equal(resumed.restart, false);
  near(resumed.at, queuedUntil, 1e-9); // gapless: no silence, no overlap
  assert.ok(resumed.at - state.t <= clock.lead + SKIP_RESUME_SLACK_S + 1e-9);
});

test('a_moderate_backlog_does_not_skip', () => {
  const { clock } = makeClock();
  clock.place(0.02);
  const p = clock.place(1.2); // 1.2 s over the cushion: under SKIP_SLACK_S
  assert.equal(p.skip, false);
  assert.equal(clock.place(0.02).skip, false);
});

// ---- whole-stream simulations ----
//
// These model the real thing: the sender emits one 40 ms Ogg page (two 20 ms
// Opus packets) per 40 ms of ITS clock, the network delivers each page after a
// fixed delay (plus whatever stalls a test adds), and the player places both
// chunks of a page when it arrives, by the LISTENER's clock. `ppm` is how much
// faster the sender's clock runs than the listener's (negative: slower), so a
// page that leaves at sender time s arrives at listener time s / (1 + ppm/1e6).
//
// It also replays what Web Audio would do with the placements: a chunk of
// 20 ms at rate r covers [at, at + 0.02 / r). Silence is any moment between
// the first start and the end of the run that no chunk covers.

const PAGE_S = 0.04;
const CHUNK_S = 0.02;

/**
 * @param {object} o
 * @param {number} o.ppm  sender faster (+) or slower (-) than the listener
 * @param {number} o.minutes  simulated length
 * @param {(sendT: number) => number} [o.delay]  network delay for a page sent at sendT
 */
function simulate({ ppm, minutes, delay = () => 0.005 }) {
  const { state, clock } = makeClock();
  const stats = {
    placements: 0,
    underruns: 0, // restarts after the first chunk
    skipEpisodes: 0,
    skipped: 0, // chunks dropped
    gapS: 0, // total silence after the first chunk started
    gaps: [], // [startT, lengthS] of each silence
    overlapS: 0, // audio scheduled on top of other audio (must stay 0)
    maxRateDev: 0,
    pendingMin: Infinity, // pending - lead, after the first 30 s of each settled stretch
    pendingMax: -Infinity,
    pendingLog: [], // [t, pending - lead] once per second, for inspection
    finalPendingErr: 0, // pending - lead at the last page
  };
  const pages = Math.round((minutes * 60) / PAGE_S);
  // Arrival time of every page; arrivals never overtake (TCP keeps order).
  let lastArrival = 0;
  let end = null; // where the scheduled audio currently ends
  let wasSkipping = false;
  let lastLogged = -1;
  let settleUntil = 30; // ignore the band check while the loop settles after a start/underrun
  for (let k = 0; k < pages; k++) {
    const sendT = (k + 1) * PAGE_S; // the page is complete at the end of its 40 ms
    const arrival = Math.max(lastArrival, (sendT + delay(sendT)) / (1 + ppm / 1e6));
    lastArrival = arrival;
    state.t = arrival;
    for (let c = 0; c < PAGE_S / CHUNK_S; c++) {
      const p = clock.place(CHUNK_S);
      stats.placements++;
      if (p.skip) {
        stats.skipped++;
        if (!wasSkipping) stats.skipEpisodes++;
        wasSkipping = true;
        continue;
      }
      wasSkipping = false;
      if (p.restart && stats.placements > 1) {
        stats.underruns++;
        settleUntil = state.t + 30;
      }
      if (end !== null) {
        if (p.at > end + 1e-12) {
          stats.gapS += p.at - end;
          stats.gaps.push([end, p.at - end]);
        }
        if (p.at < end - 1e-12) stats.overlapS += end - p.at;
      }
      end = p.at + CHUNK_S / p.rate;
      stats.maxRateDev = Math.max(stats.maxRateDev, Math.abs(p.rate - 1));
      const err = end - state.t - clock.lead - CHUNK_S / p.rate; // pending before this chunk
      if (c === 0 && state.t >= settleUntil) {
        stats.pendingMin = Math.min(stats.pendingMin, err);
        stats.pendingMax = Math.max(stats.pendingMax, err);
      }
      if (c === 0) stats.finalPendingErr = err;
      if (Math.floor(state.t) !== lastLogged) {
        lastLogged = Math.floor(state.t);
        stats.pendingLog.push([lastLogged, err]);
      }
    }
  }
  stats.finalLead = clock.lead;
  return stats;
}

// Print the numbers when asked: LIVECLOCK_SIM=1 node --test ...
function report(name, s) {
  if (!process.env.LIVECLOCK_SIM) return;
  console.log(
    `${name}: underruns=${s.underruns} skips=${s.skipEpisodes} (${s.skipped} chunks) ` +
      `gap=${(s.gapS * 1000).toFixed(1)}ms maxRateDev=${(s.maxRateDev * 100).toFixed(4)}% ` +
      `pending-lead=[${(s.pendingMin * 1000).toFixed(1)}, ${(s.pendingMax * 1000).toFixed(1)}] ms ` +
      `finalLead=${s.finalLead.toFixed(2)}`,
  );
}

for (const ppm of [200, 500, 1000, -200, -500, -1000]) {
  test(`clock_drift_${ppm}ppm_for_30_minutes_never_gaps_or_skips`, () => {
    const s = simulate({ ppm, minutes: 30 });
    report(`drift ${ppm} ppm`, s);
    assert.equal(s.underruns, 0, 'underruns');
    assert.equal(s.skipEpisodes, 0, 'skips');
    assert.equal(s.gapS, 0, 'silence');
    assert.equal(s.overlapS, 0, 'overlap');
    assert.ok(s.maxRateDev <= MAX_RATE_DEV + 1e-12, `rate deviation ${s.maxRateDev}`);
    // The queue holds within 30 ms of the cushion the whole time.
    assert.ok(s.pendingMin > -0.03 && s.pendingMax < 0.03, `pending-lead in [${s.pendingMin}, ${s.pendingMax}]`);
    near(s.finalLead, TARGET_LEAD_S);
  });
}

test('one_network_hiccup_gives_exactly_one_gap_and_nothing_after', () => {
  // At 60 s the network holds everything for 0.35 s, then delivers it at once.
  const hiccup = (sendT) => (sendT >= 60 && sendT < 60.35 ? 60.35 - sendT + 0.005 : 0.005);
  for (const ppm of [0, 1000, -1000]) {
    const s = simulate({ ppm, minutes: 10, delay: hiccup });
    report(`hiccup ${ppm} ppm`, s);
    assert.equal(s.underruns, 1, 'exactly one underrun');
    assert.equal(s.gaps.length, 1, 'exactly one silence');
    assert.ok(s.gaps[0][0] > 60 && s.gaps[0][0] < 61, `gap at ${s.gaps[0][0]}`);
    assert.ok(s.gapS < 0.5, `gap ${s.gapS} s`);
    assert.equal(s.skipEpisodes, 0, 'no skip');
    assert.equal(s.overlapS, 0, 'overlap');
    assert.ok(s.maxRateDev <= MAX_RATE_DEV + 1e-12);
    // The cushion grew to 0.3 s and has since decayed back, and the extra
    // queue from the delivered backlog has drained.
    near(s.finalLead, TARGET_LEAD_S);
    assert.ok(Math.abs(s.finalPendingErr) < 0.03, `final pending-lead ${s.finalPendingErr}`);
  }
});

test('a_3s_stall_delivered_as_a_burst_skips_once_without_cutting_audio', () => {
  // At 60 s the network holds everything for 3 s, then delivers it at once.
  const stall = (sendT) => (sendT >= 60 && sendT < 63 ? 63 - sendT + 0.005 : 0.005);
  const s = simulate({ ppm: 500, minutes: 10, delay: stall });
  report('3 s stall', s);
  assert.equal(s.underruns, 1, 'the stall itself runs the queue dry once');
  assert.equal(s.skipEpisodes, 1, 'exactly one skip');
  assert.ok(s.skipped > 0);
  // Skipping never stops or overlaps audio: the only silence is the stall's.
  assert.equal(s.gaps.length, 1, 'one silence: the stall');
  assert.equal(s.overlapS, 0, 'overlap');
  near(s.finalLead, TARGET_LEAD_S);
  assert.ok(Math.abs(s.finalPendingErr) < 0.03, `final pending-lead ${s.finalPendingErr}`);
});
