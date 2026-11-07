import itertools
import math

import numpy as np
import pytest

from tagger.tune import (
    Combo, FlagTarget, Mapping, bars, beam_search, broken, build_report, combo_raw, leave_one_out, loss,
    name_suggestions, spearman, split_units, try_split,
)


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
    # Two units with the same library column, so library stats cannot tell them
    # apart (a tie, resolved by row order: "weak" wins). Against the reference set,
    # "weak" has no spread at all (bars read 0.5) while "strong" sits well above its
    # reference mean, so only reference stats make "strong" the winner.
    units = ["weak", "strong"]
    U = np.array([[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]])
    U_ref = np.array([[0.5, 0.5], [0.0, 0.5]])
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    by_library = beam_search(U, None, units, flags, M_LIB, max_lines=1)
    assert by_library[0].lines == ("weak",) and by_library[0].loss == pytest.approx(0.0)
    m_ref = Mapping(midpoint_z=0.0, slope=2.0, source="reference")
    by_reference = beam_search(U, U_ref, units, flags, m_ref, max_lines=1)
    assert by_reference[0].lines == ("strong",)
    assert by_reference[0].loss == pytest.approx(0.0)
    assert by_reference[1].lines == ("weak",) and by_reference[1].loss > 0.2


# --- report pieces -----------------------------------------------------------


def test_spearman_identical_and_reversed():
    a = np.arange(10.0)
    assert spearman(a, a) == pytest.approx(1.0)
    assert spearman(a, -a) == pytest.approx(-1.0)
    assert spearman(a, np.ones(10)) == 0.0


def test_name_suggestions_ranks_the_matching_word_first():
    pattern = np.array([0.0, 1.0, 2.0, 3.0, 4.0, 5.0])
    U = np.vstack([pattern[::-1], pattern * 2 + 1, np.array([3.0, 0.0, 5.0, 1.0, 4.0, 2.0]), pattern])
    units = ["happy", "sad", "odd", "not a candidate"]
    got = name_suggestions(U, units, ["happy", "sad", "odd", "missing"], pattern, k=2)
    assert [w for w, _ in got] == ["sad", "odd"]
    assert got[0][1] == pytest.approx(1.0)


def test_leave_one_out_counts_flags_the_rest_still_explain():
    # Songs 0-2 read high together (unit "a" lifts all three), so leaving one out
    # still leaves a search that lifts it; song 3 is lifted by unit "c" alone and
    # nothing else hints at it, so leaving it out loses it.
    units = ["a", "b", "c"]
    U = np.array([
        [1, 1, 1, 0, 0, 0, 0, 0],
        [0, 0, 0, 0, 1, 1, 0, 0],
        [0, 0, 0, 1, 0, 0, 0, 0],
    ], dtype=float)
    flags = [FlagTarget(i, 0.75, None, 1.0, True, "high") for i in range(4)]
    flags.append(FlagTarget(0, 0.75, None, 0.5, False, "high"))   # stale: not tested
    winner = Combo(("a", "c"), 0.0)
    held, total = leave_one_out(U, None, units, flags, M_LIB, winner)
    assert (held, total) == (3, 4)


def _split_world():
    # 20 songs. Unit "a" lifts songs 0-2 and pushes 3-5 down; "b" does the reverse, so
    # together they cancel. Songs 6-7 sit low under both. Songs 0-2 and 3-5 also
    # point different ways in embedding space.
    units = ["a", "b", "noise 1", "noise 2"]
    U = np.zeros((4, 20))
    U[0, :3], U[0, 3:6] = 1.0, -1.0
    U[1, :3], U[1, 3:6] = -1.0, 1.0
    U[:2, 6:8] = -0.5
    U[2] = np.sin(np.arange(20) * 1.3) * 0.2
    U[3] = np.cos(np.arange(20) * 2.1) * 0.2
    X = np.random.default_rng(0).normal(size=(20, 6)) * 0.1
    X[:3, 0] += 1.0
    X[3:6, 1] += 1.0
    X /= np.linalg.norm(X, axis=1, keepdims=True)
    return units, U, X


def test_split_offered_when_two_groups_conflict():
    units, U, X = _split_world()
    flags = [FlagTarget(i, 0.75, None, 1.0, True, "high") for i in range(6)]
    flags += [FlagTarget(i, None, 0.5, 1.0, True, "low") for i in (6, 7)]
    best = beam_search(U, None, units, flags, M_LIB)[0]
    raw = combo_raw(U, [units.index(u) for u in best.lines])
    assert broken(bars(raw, raw, M_LIB), flags)          # no single combination fits all six
    split = try_split(U, None, units, flags, M_LIB, X, best)
    assert split is not None and split["rho"] < 0.9
    assert {tuple(split["a_flags"]), tuple(split["b_flags"])} == {(0, 1, 2), (3, 4, 5)}
    assert {split["a"].lines[0], split["b"].lines[0]} == {"a", "b"}


def test_split_not_offered_when_one_mood_satisfies_all():
    best = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB)[0]
    assert best.loss == pytest.approx(0.0)
    X = np.random.default_rng(1).normal(size=(6, 4))
    assert try_split(U_PLANTED, None, UNITS, FLAGS, M_LIB, X, best) is None


def test_build_report_describes_a_change():
    hashes = [f"h{i}" for i in range(20)]
    before = np.zeros(20)
    before[0] = 1.0
    after = np.zeros(20)
    after[[0, 1]] = 1.0
    after[5] = 0.5
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high"), FlagTarget(1, 0.75, None, 0.5, False, "high"),
             FlagTarget(2, 0.75, None, 1.0, True, "high")]
    r = build_report(
        lines=["x"], after=(after, None), before=(before, None), flags=flags,
        flag_hashes=["h0", "h1", "h2"], hashes=hashes, m=M_LIB, other_raw={"grim": after},
        loo=(1, 2), names=[("sad", 0.5)],
    )
    assert r["lines"] == ["x"] and r["loo"] == {"held": 1, "total": 2}
    assert [f["holds"] for f in r["flags"]] == [True, True, False]
    assert r["unsatisfied"] == ["h2"]
    assert len(r["top"]) == 15 and r["top"][0]["bar"] >= r["top"][-1]["bar"]
    movers = r["risers"] + r["fallers"]
    assert [x["hash"] for x in r["risers"]] == ["h5"]            # the one unflagged song that rose
    assert len(r["fallers"]) == 10 and all(x["after"] < x["before"] for x in r["fallers"])
    assert not {"h0", "h1", "h2"} & {x["hash"] for x in movers}   # flagged songs excluded
    assert r["fires_before"] < r["fires_after"]
    assert r["overlap"][0]["mood_id"] == "grim" and r["overlap"][0]["after"] == pytest.approx(1.0)
    assert r["names"] == [{"word": "sad", "rho": 0.5}]
