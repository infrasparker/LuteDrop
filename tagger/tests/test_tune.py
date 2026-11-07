import itertools
import math

import numpy as np
import pytest

from tagger.tune import FlagTarget, Mapping, bars, beam_search, broken, combo_raw, loss, split_units


def test_split_units_dedupes_and_strips():
    assert split_units(["Sad, grief ,sad", "", "a sad farewell"]) == ["sad", "grief", "a sad farewell"]


def test_split_units_drops_overlong_items():
    assert split_units(["ok, " + "x" * 301]) == ["ok"]


def test_bars_match_app_reference_mapping():
    m = Mapping(midpoint_z=0.674, slope=math.log(3) / (1.2816 - 0.674), source="reference")
    ref = np.array([0.0, 1.0])            # mean 0.5, sd 0.5
    b = bars(np.array([0.5 + 0.5 * 0.674, 0.5 + 0.5 * 1.2816]), ref, m)
    assert b[0] == pytest.approx(0.5) and b[1] == pytest.approx(0.75)


def test_bars_flat_reference_gives_half():
    m = Mapping(midpoint_z=0.0, slope=2.0, source="library")
    flat = np.array([0.3, 0.3, 0.3])
    assert bars(flat, flat, m) == pytest.approx([0.5, 0.5, 0.5])


def test_loss_band_floor_ceiling_and_aim_margin():
    f_floor = FlagTarget(0, 0.75, None, 1.0, True, "high")   # at least Understandable
    f_in = FlagTarget(1, 0.5, 0.75, 1.0, True, "mid")        # Stretch
    assert loss(np.array([0.95, 0.6]), [f_floor, f_in]) == 0.0   # above a floor is fine
    assert loss(np.array([0.70, 0.6]), [f_floor, f_in]) == pytest.approx(0.75 + 0.03 - 0.70)
    assert loss(np.array([0.95, 0.80]), [f_floor, f_in]) == pytest.approx(0.80 - (0.75 - 0.03))


def test_stale_flags_count_half():
    fresh = FlagTarget(0, 0.75, None, 1.0, True, "high")
    stale = FlagTarget(0, 0.75, None, 0.5, False, "high")
    assert loss(np.array([0.65]), [stale]) == pytest.approx(loss(np.array([0.65]), [fresh]) / 2)


def test_order_penalty_high_below_low():
    hi = FlagTarget(0, 0.75, None, 1.0, True, "high")
    lo = FlagTarget(1, None, 0.5, 1.0, True, "low")
    assert loss(np.array([0.30, 0.40]), [hi, lo]) > loss(np.array([0.30, 0.20]), [hi, lo])


def test_broken_lists_flag_positions_outside_band():
    flags = [
        FlagTarget(0, 0.75, None, 1.0, True, "high"),
        FlagTarget(1, None, 0.5, 1.0, True, "low"),
        FlagTarget(2, 0.5, 0.75, 1.0, True, "mid"),
    ]
    assert broken(np.array([0.9, 0.6, 0.6]), flags) == [1]
    assert broken(np.array([0.7, 0.4, 0.8]), flags) == [0, 2]


def test_combo_raw_is_row_mean():
    U = np.array([[1.0, 3.0], [3.0, 5.0], [9.0, 9.0]])
    assert combo_raw(U, [0, 1]) == pytest.approx([2.0, 4.0])


# --- beam search -------------------------------------------------------------
# 5 units x 6 songs. Songs 0-1 must read high, songs 4-5 low. Unit 1 lifts only
# song 0 and unit 3 only song 1, so each alone fails one "high" flag; unit 2
# lifts the low songs (and would break the low flags); unit 0 is noise and unit 4 lifts song 4 alone.
UNITS = ["noise a", "lifts song 0", "lifts low songs", "lifts song 1", "noise b"]
U_PLANTED = np.array([
    [0.0, 0.0, 0.0, 0.0, 0.0, 0.5],
    [1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 0.0, 1.0, 1.0],
    [0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, 0.0, 0.0, 0.0, 3.0, 0.0],
])
M_LIB = Mapping(midpoint_z=0.0, slope=2.0, source="library")
FLAGS = [
    FlagTarget(0, 0.75, None, 1.0, True, "high"),
    FlagTarget(1, 0.75, None, 1.0, True, "high"),
    FlagTarget(4, None, 0.5, 1.0, True, "low"),
    FlagTarget(5, None, 0.5, 1.0, True, "low"),
]


def test_planted_combination_is_the_only_zero_loss_subset():
    # Guards the premise of the search tests below, using the real loss/bars.
    zero = []
    for n in range(1, 6):
        for rows in itertools.combinations(range(5), n):
            raw = combo_raw(U_PLANTED, list(rows))
            if loss(bars(raw, raw, M_LIB), FLAGS) == pytest.approx(0.0):
                zero.append(rows)
    assert zero == [(1, 3)]


def test_beam_search_finds_planted_combination():
    best = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB)
    assert best[0].lines == (UNITS[1], UNITS[3])
    assert best[0].loss == pytest.approx(0.0)
    assert [c.loss for c in best] == sorted(c.loss for c in best)
    assert len(best) <= 3


def test_beam_search_respects_max_lines():
    results = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, max_lines=2)
    assert results and all(len(c.lines) <= 2 for c in results)
    single = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, max_lines=1)
    assert all(len(c.lines) == 1 for c in single)
    assert single[0].loss > 0.0          # one line cannot satisfy both high songs


def test_beam_search_reference_source_uses_reference_stats():
    # Same library matrix, but bars are read against a reference set whose cosines
    # are all identical per unit except a spread on unit 1/3, shifting the mapping.
    U_ref = np.array([[0.0, 1.0], [0.0, 1.0], [0.0, 1.0], [0.0, 1.0], [0.0, 1.0]])
    m_ref = Mapping(midpoint_z=0.0, slope=2.0, source="reference")
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    res = beam_search(U_PLANTED, U_ref, UNITS, flags, m_ref, max_lines=1)
    # ref mean 0.5 / sd 0.5: unit 1 raw 1.0 on song 0 -> z 1 -> bar logistic(2)=.88; others 0 -> z -1
    assert res[0].lines == (UNITS[1],)
    assert res[0].loss == pytest.approx(0.0)
