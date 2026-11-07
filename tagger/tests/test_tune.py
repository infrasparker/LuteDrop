import itertools
import math

import numpy as np
import pytest

from tagger.tune import (
    Combo, FlagTarget, Mapping, bars, beam_search, blend_units, broken, build_report, combo_raw, leave_one_out,
    loss, name_suggestions, spearman, split_units, stability, try_split,
)
from tagger import tune


def test_split_units_dedupes_and_strips():
    assert split_units(["Sad, grief ,sad", "", "a sad farewell"]) == ["sad", "grief", "a sad farewell"]


def test_split_units_drops_overlong_items():
    assert split_units(["ok, " + "x" * 301]) == ["ok"]


def test_excluded_matches_whole_words_ignoring_case():
    assert tune.excluded("Ambient drones", ["ambient"])
    assert tune.excluded("a dream, sad", ["DREAM"])
    assert not tune.excluded("ambiental", ["ambient"])
    assert not tune.excluded("dreamy music", ["dream"])
    assert tune.excluded("a haunted house", ["haunted house"])
    assert not tune.excluded("anything", [])
    assert not tune.excluded("anything", ["", "  "])


def test_unit_pool_lists_every_source_that_contributed_a_unit():
    pool = tune.UnitPool()
    pool.add_parts("word bank", ["sad, calm", "ambient"])
    pool.add_parts("current lines", ["Sad, grief", "a sad farewell"])
    pool.add_whole("current lines", ["Sad, grief", "a sad farewell"])
    pool.add_parts("Mysterious words", ["ambient, eerie"])
    # Phrases only add what is not there yet: an existing text is not credited to them.
    pool.add_whole("phrase list", ["A sad farewell", "a lament"], only_new=True, lower=True)
    assert pool.sources == {
        "sad": ["word bank", "current lines"], "calm": ["word bank"], "ambient": ["word bank", "Mysterious words"],
        "grief": ["current lines"], "a sad farewell": ["current lines"], "Sad, grief": ["current lines"],
        "eerie": ["Mysterious words"], "a lament": ["phrase list"],
    }
    assert pool.units == list(pool.sources)
    pool.drop(lambda u: "sad" in u)
    assert pool.units == ["calm", "ambient", "grief", "Sad, grief", "eerie", "a lament"]


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


def test_focused_flag_counts_three_times():
    plain = FlagTarget(0, 0.75, None, 1.0, True, "high")
    focused = FlagTarget(0, 0.75, None, 1.0, True, "high", focus=True)
    assert not plain.focus
    assert loss(np.array([0.65]), [focused]) == pytest.approx(3 * loss(np.array([0.65]), [plain]))


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
    assert best[0].loss == pytest.approx(2 * tune.LINE_COST)   # flags all met
    assert [c.loss for c in best] == sorted(c.loss for c in best)
    assert len(best) <= 3


def test_beam_search_respects_max_lines():
    results = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, max_lines=2)
    assert results and all(len(c.lines) <= 2 for c in results)
    single = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, max_lines=1)
    assert all(len(c.lines) == 1 for c in single)
    assert single[0].loss > tune.LINE_COST          # one line cannot satisfy both high songs


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
    assert by_library[0].lines == ("weak",) and by_library[0].loss == pytest.approx(tune.LINE_COST)
    m_ref = Mapping(midpoint_z=0.0, slope=2.0, source="reference")
    by_reference = beam_search(U, U_ref, units, flags, m_ref, max_lines=1)
    assert by_reference[0].lines == ("strong",)
    assert by_reference[0].loss == pytest.approx(tune.LINE_COST)
    assert by_reference[1].lines == ("weak",) and by_reference[1].loss > 0.2


def test_beam_search_explores_the_seed_plus_or_minus_one_line():
    # The seed is the planted pair plus a noise line. With a pool of one unit and a
    # beam of one, plain growth cannot reach the pair; dropping one line of the seed can.
    plain = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, top_units=1, beam=1)
    assert plain[0].loss > 2 * tune.LINE_COST
    seeded = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, top_units=1, beam=1, seeds=[(0, 1, 3)])
    assert seeded[0].lines == (UNITS[1], UNITS[3]) and seeded[0].loss == pytest.approx(2 * tune.LINE_COST)


def test_beam_search_favours_the_focused_flag():
    # One line can lift song 0 (unit 1) or song 1 (unit 3), not both. Unfocused it is a
    # tie, settled by row order; focusing song 1's flag makes unit 3 the winner.
    plain = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, max_lines=1)
    assert plain[0].lines == (UNITS[1],)
    focused = [FLAGS[0], FlagTarget(1, 0.75, None, 1.0, True, "high", focus=True), *FLAGS[2:]]
    got = beam_search(U_PLANTED, None, UNITS, focused, M_LIB, max_lines=1)
    assert got[0].lines == (UNITS[3],)


def test_line_cost_makes_the_shorter_combination_win_a_tie():
    # "sad again" reads exactly like "sad", so adding it changes no bar: the pair ties
    # the single line on the flags, and the line cost settles it for the single line.
    units = ["sad", "sad again", "noise"]
    U = np.array([[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 0.3, 0.1, 0.2]])
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    got = beam_search(U, None, units, flags, M_LIB, keep=5)
    assert got[0].lines in (("sad",), ("sad again",))
    assert got[0].loss == pytest.approx(tune.LINE_COST)
    pairs = [c for c in got if len(c.lines) == 2]
    assert all(c.loss >= 2 * tune.LINE_COST - 1e-12 for c in pairs)


def test_words_of_drops_small_words_and_punctuation():
    assert tune.words_of("Haunted house, a strange discovery") == {"haunted", "house", "strange", "discovery"}


def test_search_never_pairs_a_line_with_one_holding_the_same_words():
    units = ["haunted house", "horror, haunted house", "haunted house, horror", "horror", "a strange discovery",
             "haunted house, a strange discovery"]
    rng = np.random.default_rng(4)
    U = rng.normal(size=(len(units), 12))
    flags = [FlagTarget(i, 0.75, None, 1.0, True, "high") for i in range(4)]
    got = beam_search(U, None, units, flags, M_LIB, keep=50)
    assert got
    for c in got:
        sets = [tune.words_of(line) for line in c.lines]
        for i, a in enumerate(sets):
            for b in sets[i + 1:]:
                assert not (a <= b or b <= a), c.lines


def test_best_rows_keeps_the_best_units_alone():
    # Unit 1 lifts song 0 and unit 3 song 1; the noise units satisfy neither high flag.
    assert sorted(tune.best_rows(U_PLANTED, None, [0, 1, 3, 4], FLAGS, M_LIB, 2)) == [1, 3]


def test_beam_search_skips_seeds_longer_than_max_lines():
    got = beam_search(U_PLANTED, None, UNITS, FLAGS, M_LIB, max_lines=2, seeds=[(0, 1, 2, 3)])
    assert all(len(c.lines) <= 2 for c in got)


def test_blend_units_dedupe_words_and_drop_collapsed_blends():
    units = ["sad", "sad, deep", "memory, sad, lonely", "deep"]
    U = np.array([[1.0, 0.0, 0.0], [0.9, 0.0, 0.0], [0.8, 0.0, 0.0], [0.7, 0.0, 0.0]])
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    out = blend_units(U, None, units, flags, M_LIB, top=4)
    for text in out:
        parts = [p.strip() for p in text.split(",")]
        assert len(parts) == len(set(parts)) >= 2, text
    assert not set(out) & set(units)               # "sad" + "sad, deep" collapses to "sad, deep"
    assert len(out) == len(set(out))
    assert "sad, memory, lonely" in out            # parts kept in order, the repeat dropped
    assert "sad, deep, memory, lonely" in out


# --- stability: unflagged songs ----------------------------------------------


def test_stability_measures_unflagged_moves_and_fires():
    before = np.array([0.9, 0.2, 0.2, 0.6, 0.4])
    after = np.array([0.1, 0.4, 0.2, 0.6, 0.6])        # song 0 is flagged: its move is free
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    mean_move = (0.2 + 0.0 + 0.0 + 0.2) / 4
    fires = abs(2 - 2) / 10                                # 0.9 and 0.6 fire before; 0.6 and 0.6 after
    assert stability(after, before, flags) == pytest.approx(
        tune.STABILITY_WEIGHT * mean_move + tune.FIRES_WEIGHT * fires)
    assert stability(before, before, flags) == 0.0
    lifted = np.array([0.9, 0.7, 0.7, 0.6, 0.6])       # three more songs fire
    assert stability(lifted, before, flags) == pytest.approx(
        tune.STABILITY_WEIGHT * (0.5 + 0.5 + 0 + 0.2) / 4 + tune.FIRES_WEIGHT * 3 / 10)


def _stability_world():
    # 40 songs. Song 0 is flagged "at least Understandable". "current" leaves it at the
    # midpoint; "fixes" lifts it and nudges the songs a little; "leaks" lifts it the same
    # and also lifts songs 1-6 (unflagged) high; "gentle" lifts it and moves nothing else.
    rng = np.random.default_rng(3)
    base = rng.normal(size=40) * 0.3
    base[0] = -0.2
    current = base.copy()
    fixes = base + rng.normal(size=40) * 0.05
    fixes[0] = 2.0
    leaks = base.copy()
    leaks[0] = 2.0
    leaks[1:7] = 1.5
    gentle = base.copy()
    gentle[0] = 2.0
    return base, np.vstack([current, fixes, leaks, gentle])


def _bars_of(U, row):
    return bars(U[row], U[row], M_LIB)


def test_a_fix_for_a_broken_flag_beats_the_baseline_despite_small_moves():
    _, U = _stability_world()
    units = ["current", "fixes"]
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    before = _bars_of(U, 0)
    assert broken(before, flags) == [0]
    got = beam_search(U[:2], None, units, flags, M_LIB, max_lines=1, before=before)
    assert got[0].lines == ("fixes",)
    assert stability(_bars_of(U, 1), before, flags) > 0.0
    assert got[0].loss < loss(before, flags)


def test_less_reshaping_wins_a_tie_on_the_flags():
    _, U = _stability_world()
    units = ["current", "leaks", "gentle"]
    U3 = U[[0, 2, 3]]
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    before = _bars_of(U3, 0)
    # Both satisfy the flag; on the flags alone it is a tie, settled by row order.
    assert loss(_bars_of(U3, 1), flags) == loss(_bars_of(U3, 2), flags) == 0.0
    plain = beam_search(U3, None, units, flags, M_LIB, max_lines=1)
    assert plain[0].lines == ("leaks",)
    steady = beam_search(U3, None, units, flags, M_LIB, max_lines=1, before=before)
    assert steady[0].lines == ("gentle",)


def test_a_leak_into_unflagged_songs_loses_to_a_near_miss():
    # The leak satisfies the flag fully; the steady line leaves it a hair short (0.02 under
    # its aim). Lifting six unflagged songs of 40 into the fire zone costs more than that.
    _, U = _stability_world()
    U2 = U[[0, 2, 3]].copy()
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    before = _bars_of(U2, 0)
    # Tune "gentle" so song 0 lands 0.02 short of the aim (0.78).
    lo, hi = -1.0, 5.0
    for _ in range(60):
        mid = (lo + hi) / 2
        U2[2, 0] = mid
        lo, hi = (mid, hi) if _bars_of(U2, 2)[0] < 0.76 else (lo, mid)
    assert loss(_bars_of(U2, 2), flags) == pytest.approx(0.02, abs=1e-3)
    assert loss(_bars_of(U2, 1), flags) == 0.0
    assert stability(_bars_of(U2, 1), before, flags) > 0.02 + stability(_bars_of(U2, 2), before, flags)
    got = beam_search(U2, None, ["current", "leaks", "near miss"], flags, M_LIB, max_lines=1, before=before)
    assert got[0].lines == ("near miss",)


def test_blend_groups_name_their_members():
    units = ["sad", "deep", "lonely"]
    U = np.array([[1.0, 0.0, 0.0], [0.9, 0.0, 0.0], [0.8, 0.0, 0.0]])
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    groups = tune.blend_groups(U, None, units, flags, M_LIB, top=3)
    assert ("sad, deep", (0, 1)) in groups
    assert ("sad, deep, lonely", (0, 1, 2)) in groups
    assert [t for t, _ in groups] == blend_units(U, None, units, flags, M_LIB, top=3)


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


def test_leave_one_out_reuses_searches_of_the_same_size(monkeypatch):
    units = ["a", "b", "c"]
    U = np.array([[1, 1, 1, 0], [0, 0, 0, 1], [0, 1, 0, 1]], dtype=float)
    flags = [FlagTarget(i, 0.75, None, 1.0, True, "high") for i in range(3)]
    calls = []
    real = tune.beam_search
    monkeypatch.setattr(tune, "beam_search", lambda *a, **k: calls.append(1) or real(*a, **k))
    memo: dict = {}
    first = leave_one_out(U, None, units, flags, M_LIB, Combo(("a", "b"), 0.0), memo=memo)
    assert len(calls) == 3
    second = leave_one_out(U, None, units, flags, M_LIB, Combo(("a", "c"), 0.0), memo=memo)
    assert len(calls) == 3 and second == first      # same size: no new searches
    leave_one_out(U, None, units, flags, M_LIB, Combo(("a",), 0.0), memo=memo)
    assert len(calls) == 6                          # another size searches again


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
    assert best.loss == pytest.approx(tune.LINE_COST * len(best.lines))
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
             FlagTarget(2, 0.75, None, 1.0, True, "high", focus=True)]
    r = build_report(
        lines=["x"], after=(after, None), before=(before, None), flags=flags,
        flag_hashes=["h0", "h1", "h2"], hashes=hashes, m=M_LIB, other_raw={"grim": after},
        loo=(1, 2), names=[("sad", 0.5)], sources={"x": ["phrase list"]},
    )
    assert r["lines"] == ["x"] and r["loo"] == {"held": 1, "total": 2}
    assert r["line_sources"] == [["phrase list"]]
    assert [f["holds"] for f in r["flags"]] == [True, True, False]
    assert [f["focus"] for f in r["flags"]] == [False, False, True]
    assert r["unsatisfied"] == ["h2"]
    assert len(r["top"]) == 15 and r["top"][0]["bar"] >= r["top"][-1]["bar"]
    movers = r["risers"] + r["fallers"]
    assert [x["hash"] for x in r["risers"]] == ["h5"]            # the one unflagged song that rose
    assert len(r["fallers"]) == 10 and all(x["after"] < x["before"] for x in r["fallers"])
    assert not {"h0", "h1", "h2"} & {x["hash"] for x in movers}   # flagged songs excluded
    assert r["fires_before"] < r["fires_after"]
    assert r["overlap"][0]["mood_id"] == "grim" and r["overlap"][0]["after"] == pytest.approx(1.0)
    assert r["names"] == [{"word": "sad", "rho": 0.5}]
    b_after, b_before = bars(after, after, M_LIB), bars(before, before, M_LIB)
    assert r["stability"] == pytest.approx(stability(b_after, b_before, flags))
    assert r["loss"] == pytest.approx(loss(b_after, flags) + r["stability"] + tune.LINE_COST * 1)


def test_blends_leave_out_a_part_whose_words_are_already_there():
    units = ["horror, haunted house, terror", "a haunted house", "dark"]
    U = np.array([[1.0, 0.0, 0.0], [0.9, 0.0, 0.0], [0.8, 0.0, 0.0]])
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    out = blend_units(U, None, units, flags, M_LIB, top=3)
    # "a haunted house" adds no word to the first unit, so that pair is the unit itself (dropped).
    assert out == ["horror, haunted house, terror, dark", "a haunted house, dark"]


def _leaky_split_world():
    # _split_world plus "a leaks", listed first: it lifts songs 0-2 like "a" and also
    # lifts every unflagged song (8-19). On the flags alone it ties "a" and wins by row order.
    units, U, X = _split_world()
    leak = U[0].copy()
    leak[:3] = 2.0
    leak[8:20] = 1.2
    flags = [FlagTarget(i, 0.75, None, 1.0, True, "high") for i in range(6)]
    flags += [FlagTarget(i, None, 0.5, 1.0, True, "low") for i in (6, 7)]
    return ["a leaks", *units], np.vstack([leak, U]), X, flags


def test_split_halves_pay_for_reshaping_unflagged_songs():
    units, U, X, flags = _leaky_split_world()
    before = _bars_of(U, units.index("noise 1"))         # the current lines
    best = beam_search(U, None, units, flags, M_LIB, before=before)[0]
    plain = try_split(U, None, units, flags, M_LIB, X, best)
    assert plain is not None and "a leaks" in plain["a"].lines + plain["b"].lines
    steady = try_split(U, None, units, flags, M_LIB, X, best, before=before)
    assert steady is not None
    halves = steady["a"].lines + steady["b"].lines
    assert "a leaks" not in halves and "a" in halves and "b" in halves


def test_blend_slots_go_to_units_that_do_not_leak():
    # 30 songs; song 0 is flagged high. "leaky" (listed first, so it wins ties) lifts song 0
    # and ten unflagged songs; "clean one" and "clean two" lift song 0 alone. On the flags
    # alone all three tie and "leaky" takes a blend slot; with stability it does not.
    base = np.random.default_rng(7).normal(size=30) * 0.2
    base[0] = 0.0
    leaky, clean1, clean2 = base.copy(), base.copy(), base.copy()
    leaky[0] = clean1[0] = clean2[0] = 3.0
    leaky[1:11] = 1.5
    units = ["leaky", "clean one", "clean two", "current"]
    U = np.vstack([leaky, clean1, clean2, base])
    flags = [FlagTarget(0, 0.75, None, 1.0, True, "high")]
    before = _bars_of(U, 3)
    plain = [t for t, _ in tune.blend_groups(U, None, units, flags, M_LIB, top=2)]
    assert any("leaky" in t for t in plain), plain
    steady = [t for t, _ in tune.blend_groups(U, None, units, flags, M_LIB, top=2, before=before)]
    assert steady and not any("leaky" in t for t in steady), steady
