from concurrent.futures import Future, ThreadPoolExecutor
from concurrent.futures.process import BrokenProcessPool

import threading
import zlib

import numpy as np
import pytest
from fastapi.testclient import TestClient

from tagger import app as app_module, moods, rhythm, tune
from tagger.app import create_app
from tagger.embed import EmbeddingCache
from tagger.reference import ReferenceSet, reference_stats

TOKEN = "secret"
HEADERS = {"X-LuteDrop-Token": TOKEN}
DIM = 8
FEATS = {k: 0.0 for k in rhythm.FEATURES}


class FakeEmbedder:
    name = "fake@test"

    def load(self):
        pass

    def embed_text(self, texts):
        # Points along the direction of the "hi" tracks.
        v = np.linspace(1, 2, DIM)
        return np.tile(v / np.linalg.norm(v), (len(texts), 1))

    def embed_windows(self, wavs):
        out = np.zeros((len(wavs), DIM), dtype=np.float32)
        for i, w in enumerate(wavs):
            m = float(np.mean(w))
            out[i] = np.linspace(m, m * 2, DIM)
        return out


def synthetic(level: float, seed: int) -> np.ndarray:
    rng = np.random.default_rng(seed)
    return (level + rng.normal(0, 0.05, size=(2, 100))).astype(np.float32)


@pytest.fixture
def calls():
    return []


@pytest.fixture
def client(tmp_path, monkeypatch, calls):
    def fake_load(path: str) -> np.ndarray:
        calls.append(path)
        if path.startswith("bad"):
            raise OSError("cannot decode")
        kind, idx = path.split("-")
        level = 1.0 if kind == "hi" else -1.0
        return synthetic(level, int(idx))

    monkeypatch.setattr(app_module, "decode_track", lambda p: (fake_load(p), FEATS))
    app = create_app(
        FakeEmbedder(), EmbeddingCache(tmp_path), TOKEN, executor_factory=lambda: ThreadPoolExecutor(2)
    )
    return TestClient(app)


def embed(client, items):
    return client.post(
        "/embed",
        json={"items": [{"hash": h, "path": p} for h, p in items]},
        headers=HEADERS,
    )


def test_rejects_missing_or_wrong_token(client):
    assert client.get("/health").status_code == 401
    assert client.get("/health", headers={"X-LuteDrop-Token": "nope"}).status_code == 401
    assert client.get("/health", headers=HEADERS).status_code == 200


def test_empty_server_token_rejects_empty_header(tmp_path):
    app = create_app(FakeEmbedder(), EmbeddingCache(tmp_path), "")
    c = TestClient(app)
    assert c.get("/health", headers={"X-LuteDrop-Token": ""}).status_code == 401
    assert c.get("/health").status_code == 401


def test_health_reports_model_and_cache_count(client):
    embed(client, [("h1", "hi-1")])
    body = client.get("/health", headers=HEADERS).json()
    assert body == {"model": "fake@test", "cache_count": 1, "ready": True, "error": None, "reference": None}


class SlowEmbedder(FakeEmbedder):
    """Stays loading until `release` is set."""

    def __init__(self):
        self.release = threading.Event()

    def load(self):
        self.release.wait(5)


def test_embed_waits_for_model_then_answers_503(tmp_path, monkeypatch):
    monkeypatch.setattr(app_module, "LOAD_WAIT_SECONDS", 0.05)
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    embedder = SlowEmbedder()
    c = TestClient(
        create_app(embedder, EmbeddingCache(tmp_path), TOKEN, executor_factory=lambda: ThreadPoolExecutor(1))
    )
    assert c.get("/health", headers=HEADERS).json()["ready"] is False
    r = embed(c, [("h1", "hi-1")])
    assert r.status_code == 503
    assert "loading" in r.json()["detail"]

    embedder.release.set()
    monkeypatch.setattr(app_module, "LOAD_WAIT_SECONDS", 5)
    assert embed(c, [("h1", "hi-1")]).json()["results"] == [{"hash": "h1", "ok": True}]
    assert c.get("/health", headers=HEADERS).json()["ready"] is True


class BrokenEmbedder(FakeEmbedder):
    def load(self):
        raise RuntimeError("no weights")


def test_failed_model_load_is_reported(tmp_path):
    c = TestClient(create_app(BrokenEmbedder(), EmbeddingCache(tmp_path), TOKEN))
    r = embed(c, [("h1", "hi-1")])
    assert r.status_code == 503
    assert "no weights" in r.json()["detail"]
    body = c.get("/health", headers=HEADERS).json()
    assert body["ready"] is False
    assert body["error"] == "no weights"


def test_embed_caches_and_skips_cached(client, calls):
    r1 = embed(client, [("h1", "hi-1")])
    assert r1.json()["results"] == [{"hash": "h1", "ok": True}]
    assert calls == ["hi-1"]
    r2 = embed(client, [("h1", "hi-1")])
    assert r2.json()["results"] == [{"hash": "h1", "ok": True}]
    assert calls == ["hi-1"]


def test_embed_reports_unreadable_path(client):
    r = embed(client, [("a", "hi-1"), ("b", "bad-1"), ("c", "lo-2")])
    results = {x["hash"]: x for x in r.json()["results"]}
    assert results["a"]["ok"] is True
    assert results["c"]["ok"] is True
    assert results["b"]["ok"] is False
    assert isinstance(results["b"]["error"], str) and results["b"]["error"]


def test_embed_over_32_items_rejected(client):
    items = [(f"h{i}", "hi-1") for i in range(33)]
    assert embed(client, items).status_code == 422


def seed(client, n=6):
    items = [(f"hi{i}", f"hi-{i}") for i in range(n)] + [
        (f"lo{i}", f"lo-{i}") for i in range(n)
    ]
    embed(client, items)
    return [h for h, _ in items]


def test_score_shape(client):
    hashes = seed(client)
    pos = [h for h in hashes if h.startswith("hi")]
    neg = [h for h in hashes if h.startswith("lo")]
    body = {
        "scene": "encounter",
        "gate": {"positives": pos, "negatives": neg},
        "tags": {"brass": {"positives": pos, "negatives": neg}},
        "targets": hashes,
    }
    r = client.post("/score", json=body, headers=HEADERS)
    assert r.status_code == 200
    data = r.json()
    assert data["model"] == "fake@test"
    assert set(data["gate"]["scores"]) == set(hashes)
    assert data["gate"]["quality"]["pos"] == 6
    brass = data["tags"]["brass"]
    assert brass["trusted"] is True
    assert brass["threshold"] is not None
    assert brass["quality"] == {"auc": brass["quality"]["auc"], "pos": 6, "neg": 6}
    assert set(brass["scores"]) == set(hashes)
    assert all(brass["scores"][h] > 0.5 for h in pos)
    assert all(brass["scores"][h] < 0.5 for h in neg)


def test_score_ignores_unknown_hashes(client):
    hashes = seed(client)
    pos = [h for h in hashes if h.startswith("hi")] + ["ghost"]
    neg = [h for h in hashes if h.startswith("lo")]
    body = {
        "scene": "encounter",
        "gate": {"positives": pos, "negatives": neg},
        "tags": {},
        "targets": hashes + ["ghost2"],
    }
    r = client.post("/score", json=body, headers=HEADERS)
    assert r.status_code == 200
    data = r.json()
    assert data["gate"]["quality"]["pos"] == 6
    assert "ghost2" not in data["gate"]["scores"]
    assert set(data["gate"]["scores"]) == set(hashes)


class BrokenPool:
    def __init__(self, fail_on):
        self.fail_on = fail_on

    def submit(self, fn, *args):
        if self.fail_on == "submit":
            raise BrokenProcessPool("worker died")
        f = Future()
        f.set_exception(BrokenProcessPool("worker died"))
        return f

    def shutdown(self, **kwargs):
        pass


@pytest.mark.parametrize("fail_on", ["submit", "result"])
def test_embed_recovers_from_broken_pool(tmp_path, monkeypatch, fail_on):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    pools = iter([BrokenPool(fail_on)])
    app = create_app(
        FakeEmbedder(),
        EmbeddingCache(tmp_path),
        TOKEN,
        executor_factory=lambda: next(pools, None) or ThreadPoolExecutor(2),
    )
    c = TestClient(app)
    first = embed(c, [("h1", "hi-1")]).json()["results"][0]
    assert first["ok"] is False and first["error"]
    second = embed(c, [("h1", "hi-1")]).json()["results"][0]
    assert second == {"hash": "h1", "ok": True}


def test_score_with_no_cached_targets(client):
    hashes = seed(client)
    pos = [h for h in hashes if h.startswith("hi")]
    neg = [h for h in hashes if h.startswith("lo")]
    for targets in ([], ["ghost"]):
        body = {
            "gate": {"positives": pos, "negatives": neg},
            "tags": {"brass": {"positives": pos, "negatives": neg}},
            "targets": targets,
        }
        r = client.post("/score", json=body, headers=HEADERS)
        assert r.status_code == 200
        data = r.json()
        assert data["gate"]["scores"] == {}
        assert data["tags"]["brass"]["scores"] == {}
        assert data["gate"]["quality"]["pos"] == 6
        assert data["tags"]["brass"]["trusted"] is True


def test_score_gate_has_neg_threshold(client):
    hashes = seed(client)
    pos = [h for h in hashes if h.startswith("hi")]
    neg = [h for h in hashes if h.startswith("lo")]
    body = {"gate": {"positives": pos, "negatives": neg}, "tags": {}, "targets": hashes}
    data = client.post("/score", json=body, headers=HEADERS).json()
    assert data["gate"]["neg_threshold"] is not None
    assert 0.0 < data["gate"]["neg_threshold"] <= 1.0


def test_score_untrusted_gate_has_null_neg_threshold(client):
    hashes = seed(client)
    body = {
        "gate": {"positives": hashes[:1], "negatives": hashes[-1:]},
        "tags": {},
        "targets": hashes,
    }
    assert client.post("/score", json=body, headers=HEADERS).json()["gate"]["neg_threshold"] is None


def test_similar_ranks_by_centroid_and_excludes_query(client):
    hashes = seed(client)
    his = [h for h in hashes if h.startswith("hi")]
    body = {"query": his[:2], "candidates": hashes, "limit": 50}
    r = client.post("/similar", json=body, headers=HEADERS)
    assert r.status_code == 200
    results = r.json()["results"]
    got = [x["hash"] for x in results]
    assert not set(got) & set(his[:2])
    assert set(got[: len(his) - 2]) == set(his[2:])
    scores = [x["score"] for x in results]
    assert scores == sorted(scores, reverse=True)


def test_similar_limit(client):
    hashes = seed(client)
    body = {"query": ["hi0"], "candidates": hashes, "limit": 3}
    assert len(client.post("/similar", json=body, headers=HEADERS).json()["results"]) == 3


def test_similar_limit_is_capped(client):
    hashes = seed(client)
    body = {"query": ["hi0"], "candidates": hashes, "limit": 100000}
    assert client.post("/similar", json=body, headers=HEADERS).status_code == 200


def test_similar_empty_inputs(client):
    hashes = seed(client)
    for body in (
        {"query": [], "candidates": hashes},
        {"query": ["hi0"], "candidates": []},
        {"query": ["ghost"], "candidates": hashes},
        {"query": ["hi0"], "candidates": ["ghost"]},
    ):
        r = client.post("/similar", json=body, headers=HEADERS)
        assert r.status_code == 200
        assert r.json() == {"results": []}


def test_search_uses_text_embedding(client):
    hashes = seed(client)
    body = {"text": "bright and heroic", "candidates": hashes, "limit": 6}
    r = client.post("/search", json=body, headers=HEADERS)
    assert r.status_code == 200
    got = [x["hash"] for x in r.json()["results"]]
    assert len(got) == 6
    assert all(h.startswith("hi") for h in got)


def test_search_empty_text_or_candidates(client):
    hashes = seed(client)
    for body in ({"text": "  ", "candidates": hashes}, {"text": "x", "candidates": []}):
        r = client.post("/search", json=body, headers=HEADERS)
        assert r.json() == {"results": []}


def test_map_shape_range_and_cache(client, monkeypatch):
    hashes = seed(client)
    n_calls = []
    real = app_module.scry.layout_2d
    monkeypatch.setattr(
        app_module.scry, "layout_2d", lambda v: (n_calls.append(1), real(v))[1]
    )
    first = client.post("/map", json={"hashes": hashes}, headers=HEADERS).json()["points"]
    second = client.post(
        "/map", json={"hashes": list(reversed(hashes))}, headers=HEADERS
    ).json()["points"]
    assert len(n_calls) == 1
    assert {p["hash"] for p in first} == set(hashes)
    assert all(0.0 <= p["x"] <= 1.0 and 0.0 <= p["y"] <= 1.0 for p in first)
    by = lambda pts: {p["hash"]: (p["x"], p["y"]) for p in pts}
    assert by(first) == by(second)


def test_map_tiny_inputs(client):
    hashes = seed(client, n=3)
    for n in (0, 1, 2, 4):
        r = client.post("/map", json={"hashes": hashes[:n]}, headers=HEADERS)
        assert r.status_code == 200
        pts = r.json()["points"]
        assert len(pts) == n
    one = client.post("/map", json={"hashes": hashes[:1]}, headers=HEADERS).json()["points"]
    assert (one[0]["x"], one[0]["y"]) == (0.5, 0.5)


def test_endpoints_drop_uncached_hashes(client):
    hashes = seed(client)
    r = client.post("/map", json={"hashes": hashes + ["ghost"]}, headers=HEADERS)
    assert {p["hash"] for p in r.json()["points"]} == set(hashes)
    r = client.post(
        "/similar", json={"query": ["hi0"], "candidates": hashes + ["ghost"]}, headers=HEADERS
    )
    assert "ghost" not in {x["hash"] for x in r.json()["results"]}
    r = client.post("/search", json={"text": "x", "candidates": hashes + ["ghost"]}, headers=HEADERS)
    assert "ghost" not in {x["hash"] for x in r.json()["results"]}


@pytest.mark.parametrize(
    "path,body",
    [
        ("/similar", {"query": [], "candidates": []}),
        ("/search", {"text": "x", "candidates": []}),
        ("/map", {"hashes": []}),
    ],
)
def test_scry_endpoints_require_token(client, path, body):
    assert client.post(path, json=body).status_code == 401
    assert client.post(path, json=body, headers={"X-LuteDrop-Token": "nope"}).status_code == 401


def _rhythm_client(tmp_path, monkeypatch, calls):
    def fake_decode(path):
        calls.append(path)
        return synthetic(1.0, 1), {k: 0.5 for k in rhythm.FEATURES}

    monkeypatch.setattr(app_module, "decode_track", fake_decode)
    cache = EmbeddingCache(tmp_path)
    app = create_app(FakeEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2))
    return TestClient(app), cache


def test_embed_backfills_rhythm_for_cached_embedding(tmp_path, monkeypatch):
    calls = []
    c, cache = _rhythm_client(tmp_path, monkeypatch, calls)
    cache.put("h1", np.ones((2, DIM), dtype=np.float32))
    monkeypatch.setattr(
        FakeEmbedder, "embed_windows", lambda self, w: (_ for _ in ()).throw(AssertionError("embedder called"))
    )
    assert not cache.has_rhythm("h1")
    r = embed(c, [("h1", "hi-1")])
    assert r.json()["results"] == [{"hash": "h1", "ok": True}]
    assert calls == ["hi-1"]
    assert cache.has_rhythm("h1")
    assert cache.get_rhythm("h1") == {k: 0.5 for k in rhythm.FEATURES}
    assert np.array_equal(cache.get_windows("h1"), np.ones((2, DIM), dtype=np.float32))


def test_embed_stores_rhythm_for_new_files(tmp_path, monkeypatch):
    c, cache = _rhythm_client(tmp_path, monkeypatch, [])
    assert embed(c, [("h2", "hi-1")]).json()["results"] == [{"hash": "h2", "ok": True}]
    assert cache.has("h2") and cache.has_rhythm("h2")


def test_embed_skips_rhythm_when_features_missing(tmp_path, monkeypatch):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), None))
    cache = EmbeddingCache(tmp_path)
    app = create_app(FakeEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2))
    c = TestClient(app)
    assert embed(c, [("h3", "hi-1")]).json()["results"] == [{"hash": "h3", "ok": True}]
    assert cache.has("h3") and not cache.has_rhythm("h3")


def _rhythm_scored(client, cache, n=6):
    """Seed embeddings plus rhythm vectors where positives have high perc_ratio."""
    hashes = seed(client, n)
    rng = np.random.default_rng(3)
    for h in hashes:
        feats = {k: float(rng.normal(0, 1)) for k in rhythm.FEATURES}
        feats["perc_ratio"] = (5.0 if h.startswith("hi") else -5.0) + float(rng.normal(0, 0.3))
        cache.put_rhythm(h, feats)
    return hashes


@pytest.fixture
def scored(tmp_path, monkeypatch):
    def fake_decode(path):
        kind, idx = path.split("-")
        return synthetic(1.0 if kind == "hi" else -1.0, int(idx)), FEATS

    monkeypatch.setattr(app_module, "decode_track", fake_decode)
    cache = EmbeddingCache(tmp_path)
    app = create_app(FakeEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2))
    return TestClient(app), cache


def test_score_returns_pace_head_on_rhythm(scored):
    client, cache = scored
    hashes = _rhythm_scored(client, cache)
    pos = [h for h in hashes if h.startswith("hi")]
    neg = [h for h in hashes if h.startswith("lo")]
    cache._rhythm_path("lo0").unlink()  # one target without rhythm
    body = {
        "gate": {"positives": pos, "negatives": neg},
        "pace": {"positives": pos, "negatives": neg[1:]},
        "targets": hashes,
    }
    data = client.post("/score", json=body, headers=HEADERS).json()
    pace = data["pace"]
    assert set(pace) == {"quality", "trusted", "threshold", "neg_threshold", "scores"}
    assert pace["trusted"] is True
    assert set(pace["scores"]) == set(hashes) - {"lo0"}
    assert all(pace["scores"][h] > 0.5 for h in pos)
    assert all(pace["scores"][h] < 0.5 for h in neg[1:])


def test_score_ignores_cached_rhythm_with_other_keys(scored):
    client, cache = scored
    hashes = _rhythm_scored(client, cache)
    pos = [h for h in hashes if h.startswith("hi")]
    neg = [h for h in hashes if h.startswith("lo")]
    cache.put_rhythm("lo0", {"other_feature": 1.0})  # written with a different feature set
    body = {
        "gate": {"positives": pos, "negatives": neg},
        "pace": {"positives": pos, "negatives": neg[1:]},
        "targets": hashes,
    }
    r = client.post("/score", json=body, headers=HEADERS)
    assert r.status_code == 200
    assert set(r.json()["pace"]["scores"]) == set(hashes) - {"lo0"}


def test_embed_warns_when_rhythm_features_missing(tmp_path, monkeypatch, caplog):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), None))
    cache = EmbeddingCache(tmp_path)
    app = create_app(FakeEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2))
    with caplog.at_level("WARNING"):
        embed(TestClient(app), [("h9", "hi-1")])
    assert any("h9" in m for m in caplog.messages)


def test_score_without_pace_has_no_pace_key(client):
    hashes = seed(client)
    pos = [h for h in hashes if h.startswith("hi")]
    neg = [h for h in hashes if h.startswith("lo")]
    body = {"gate": {"positives": pos, "negatives": neg}, "targets": hashes}
    assert "pace" not in client.post("/score", json=body, headers=HEADERS).json()


def test_score_returns_raw_mood_for_targets(tmp_path, monkeypatch):
    from tagger import moods

    dirs = {}
    texts = [t for group in moods.MOODS.values() for t in group] + moods.ENERGY_POS + moods.ENERGY_NEG
    assert len(set(texts)) == len(texts)
    for i, text in enumerate(texts):
        v = np.zeros(DIM)
        v[i % DIM] = 1.0
        dirs[text] = v

    class MoodEmbedder(FakeEmbedder):
        calls = 0

        def embed_text(self, texts):
            MoodEmbedder.calls += 1
            return np.vstack([dirs[t] for t in texts])

    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    cache = EmbeddingCache(tmp_path)
    app = create_app(MoodEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2))
    c = TestClient(app)
    embed(c, [("a", "x-1"), ("b", "x-2")])
    body = {"gate": {}, "targets": ["a", "b", "ghost"]}
    data = c.post("/score", json=body, headers=HEADERS).json()
    c.post("/score", json=body, headers=HEADERS)
    # Every prompt text embedded in one call, then reused.
    assert MoodEmbedder.calls == 1
    assert set(data["mood"]) == {"a", "b"}

    mean = cache.get("a")

    def pole(texts):
        return float(np.mean([mean @ dirs[t] for t in texts]))

    # The default moods plus energy; the pools and the scales are gone.
    assert set(data["mood"]["a"]) == {"moods", "energy"}
    # Moods are absolute similarities (mean cosine with the mood's prompts).
    scores = data["mood"]["a"]["moods"]
    assert list(scores) == list(moods.MOODS)
    for mood_id, prompts in moods.MOODS.items():
        assert scores[mood_id] == pytest.approx(pole(prompts))
    assert any(v != 0 for v in scores.values())
    # Energy is two-sided: positive minus negative.
    energy = data["mood"]["a"]["energy"]
    assert energy == pytest.approx(pole(moods.ENERGY_POS) - pole(moods.ENERGY_NEG))
    assert energy != 0


def test_score_mood_recovers_after_partial_embed_failure(tmp_path, monkeypatch):
    class FlakyEmbedder(FakeEmbedder):
        calls = 0

        def embed_text(self, texts):
            FlakyEmbedder.calls += 1
            if FlakyEmbedder.calls == 1:  # the first prompt batch fails
                raise RuntimeError("transient")
            return super().embed_text(texts)

    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    cache = EmbeddingCache(tmp_path)
    c = TestClient(create_app(FlakyEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2)))
    embed(c, [("a", "x-1")])
    from tagger import moods

    body = {"gate": {}, "targets": ["a"]}
    first = c.post("/score", json=body, headers=HEADERS)
    assert first.status_code == 200 and "mood" not in first.json()
    second = c.post("/score", json=body, headers=HEADERS)
    assert second.status_code == 200
    assert set(second.json()["mood"]["a"]) == {"moods", "energy"}
    assert len(second.json()["mood"]["a"]["moods"]) == len(moods.MOODS)


def _text_embedder():
    """An embedder that gives each new prompt text its own axis and counts
    the texts it was asked to embed."""

    class TextEmbedder(FakeEmbedder):
        def __init__(self):
            self.axes: dict[str, int] = {}
            self.embedded: list[str] = []

        def embed_text(self, texts):
            self.embedded.extend(texts)
            rows = []
            for t in texts:
                v = np.zeros(DIM)
                v[self.axes.setdefault(t, len(self.axes) % DIM)] = 1.0
                rows.append(v)
            return np.vstack(rows)

    return TextEmbedder()


def test_score_uses_mood_prompts_from_the_request(tmp_path, monkeypatch):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    cache = EmbeddingCache(tmp_path)
    embedder = _text_embedder()
    c = TestClient(create_app(embedder, cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2)))
    embed(c, [("a", "x-1")])
    vocab = {
        "gate": {},
        "targets": ["a"],
        "mood_prompts": {"spooky": ["ghost music", "  haunted halls  "], "calm": ["still water"]},
        "energy_prompts": {"positive": ["fast"], "negative": ["slow", "still water"]},
    }
    data = c.post("/score", json=vocab, headers=HEADERS).json()
    # Only the request's moods come back, in its order; prompts are trimmed.
    assert list(data["mood"]["a"]["moods"]) == ["spooky", "calm"]
    assert sorted(embedder.embedded) == sorted(["ghost music", "haunted halls", "still water", "fast", "slow"])

    mean = cache.get("a")

    def pole(texts):
        return float(np.mean([mean @ np.eye(DIM)[embedder.axes[t] % DIM] for t in texts]))

    assert data["mood"]["a"]["moods"]["spooky"] == pytest.approx(pole(["ghost music", "haunted halls"]))
    assert data["mood"]["a"]["energy"] == pytest.approx(pole(["fast"]) - pole(["slow", "still water"]))

    # Changing one mood's prompts embeds only the new text.
    embedder.embedded.clear()
    vocab["mood_prompts"]["calm"] = ["still water", "a quiet lake"]
    c.post("/score", json=vocab, headers=HEADERS)
    assert embedder.embedded == ["a quiet lake"]

    # Leaving the vocabulary out falls back to the defaults in moods.py.
    from tagger import moods

    data = c.post("/score", json={"gate": {}, "targets": ["a"]}, headers=HEADERS).json()
    assert list(data["mood"]["a"]["moods"]) == list(moods.MOODS)


def test_prompt_cache_overflow_keeps_the_texts_in_use(tmp_path, monkeypatch):
    monkeypatch.setattr(app_module, "MAX_CACHED_PROMPTS", 5)
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    embedder = _text_embedder()
    c = TestClient(create_app(embedder, EmbeddingCache(tmp_path), TOKEN, executor_factory=lambda: ThreadPoolExecutor(2)))
    embed(c, [("a", "x-1")])
    vocab = {
        "gate": {},
        "targets": ["a"],
        "mood_prompts": {"calm": ["still water"]},
        "energy_prompts": {"positive": ["fast"], "negative": ["slow"]},
    }
    assert c.post("/score", json=vocab, headers=HEADERS).status_code == 200
    # Three cached texts plus three new ones overflow the cache: it starts
    # over and still has every text this request uses.
    vocab["mood_prompts"]["calm"] = ["still water", "a quiet lake", "dusk", "fog"]
    embedder.embedded.clear()
    res = c.post("/score", json=vocab, headers=HEADERS)
    assert res.status_code == 200
    assert res.json()["mood"]["a"]["moods"]["calm"] is not None
    assert sorted(embedder.embedded) == sorted(["still water", "a quiet lake", "dusk", "fog", "fast", "slow"])


def test_preview_ranks_targets_by_raw_prompt_similarity(tmp_path, monkeypatch):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    cache = EmbeddingCache(tmp_path)
    embedder = _text_embedder()
    c = TestClient(create_app(embedder, cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2)))
    embed(c, [("a", "x-1"), ("b", "x-2")])
    body = {"prompts": ["ghost music", " dark "], "targets": ["a", "b", "ghost"]}
    data = c.post("/preview", json=body, headers=HEADERS).json()
    # Cached targets only, keyed by hash, raw mean cosine.
    assert set(data) == {"a", "b"}
    for h in ("a", "b"):
        mean = cache.get(h)
        expected = np.mean([mean @ np.eye(DIM)[embedder.axes[t] % DIM] for t in ["ghost music", "dark"]])
        assert data[h] == pytest.approx(expected)
    # The prompt embeddings are shared with /score's cache.
    embedder.embedded.clear()
    c.post("/preview", json=body, headers=HEADERS)
    assert embedder.embedded == []
    # No cached target: nothing to rank. No prompts: refused.
    assert c.post("/preview", json={"prompts": ["x"], "targets": ["ghost"]}, headers=HEADERS).json() == {}
    assert c.post("/preview", json={"prompts": [], "targets": ["a"]}, headers=HEADERS).status_code == 422
    assert c.post("/preview", json=body).status_code == 401


@pytest.mark.parametrize(
    "extra",
    [
        {"mood_prompts": {"calm": []}},
        {"mood_prompts": {"calm": ["   "]}},
        {"mood_prompts": {"Calm": ["still"]}},
        {"mood_prompts": {"energy+": ["still"]}},
        {"energy_prompts": {"positive": ["fast"], "negative": []}},
        {"energy_prompts": {"positive": ["fast"]}},
    ],
)
def test_score_rejects_a_bad_vocabulary(client, extra):
    r = client.post("/score", json={"gate": {}, "targets": [], **extra}, headers=HEADERS)
    assert r.status_code == 422


def test_score_without_loaded_model_omits_mood(tmp_path, monkeypatch):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    cache = EmbeddingCache(tmp_path)
    cache.put("a", np.ones((2, DIM), dtype=np.float32))
    embedder = SlowEmbedder()
    c = TestClient(create_app(embedder, cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(1)))
    body = {"gate": {"positives": ["a"], "negatives": []}, "targets": ["a"]}
    r = c.post("/score", json=body, headers=HEADERS)
    assert r.status_code == 200
    assert "gate" in r.json() and "mood" not in r.json()
    embedder.release.set()


def test_score_with_failed_model_omits_mood(tmp_path):
    cache = EmbeddingCache(tmp_path)
    cache.put("a", np.ones((2, DIM), dtype=np.float32))
    c = TestClient(create_app(BrokenEmbedder(), cache, TOKEN))
    c.get("/health", headers=HEADERS)
    import time

    for _ in range(100):
        if c.get("/health", headers=HEADERS).json()["error"]:
            break
        time.sleep(0.02)
    r = c.post("/score", json={"gate": {}, "targets": ["a"]}, headers=HEADERS)
    assert r.status_code == 200 and "mood" not in r.json()


REF_VECTORS = np.random.default_rng(3).normal(size=(6, DIM))
REF_VECTORS /= np.linalg.norm(REF_VECTORS, axis=1, keepdims=True)


@pytest.fixture
def ref_client(tmp_path, monkeypatch):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    reference = ReferenceSet("test-set", "fake@test", REF_VECTORS)
    app = create_app(
        FakeEmbedder(),
        EmbeddingCache(tmp_path),
        TOKEN,
        executor_factory=lambda: ThreadPoolExecutor(2),
        reference=reference,
    )
    return TestClient(app)


class TextVectorEmbedder(FakeEmbedder):
    """A different unit vector for every prompt text."""

    @staticmethod
    def vec(text):
        v = np.random.default_rng(zlib.crc32(text.encode())).normal(size=DIM)
        return v / np.linalg.norm(v)

    def embed_text(self, texts):
        return np.vstack([self.vec(t) for t in texts])


def _score_reference(tmp_path, monkeypatch, extra):
    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    reference = ReferenceSet("test-set", "fake@test", REF_VECTORS)
    app = create_app(
        TextVectorEmbedder(),
        EmbeddingCache(tmp_path),
        TOKEN,
        executor_factory=lambda: ThreadPoolExecutor(2),
        reference=reference,
    )
    c = TestClient(app)
    embed(c, [("h1", "hi-1")])
    body = c.post(
        "/score", json={"gate": {"positives": [], "negatives": []}, "targets": ["h1"], **extra}, headers=HEADERS
    ).json()
    return reference, body["mood_reference"]


def _expected(reference, mood_groups, pos, neg):
    emb = TextVectorEmbedder
    prompts = {k: np.vstack([emb.vec(t) for t in v]) for k, v in mood_groups.items()}
    prompts["energy+"] = np.vstack([emb.vec(t) for t in pos])
    prompts["energy-"] = np.vstack([emb.vec(t) for t in neg])
    return reference_stats(reference, prompts, list(mood_groups))


def _assert_stats(got, expected):
    assert got["set"] == expected["set"] == "test-set" and got["tracks"] == 6
    assert set(got["stats"]) == set(expected["stats"])
    for k, v in expected["stats"].items():
        assert got["stats"][k]["mean"] == pytest.approx(v["mean"])
        assert got["stats"][k]["sd"] == pytest.approx(v["sd"])
        assert v["sd"] > 0


def test_score_returns_mood_reference_for_the_prompts_sent(tmp_path, monkeypatch):
    groups = {"grim": ["a", "b"], "joyful": ["c"]}
    pos, neg = ["d", "f"], ["e"]
    reference, got = _score_reference(
        tmp_path,
        monkeypatch,
        {"mood_prompts": groups, "energy_prompts": {"positive": pos, "negative": neg}},
    )
    assert set(got["stats"]) == {"grim", "joyful", "energy"}
    _assert_stats(got, _expected(reference, groups, pos, neg))
    # Swapped energy sides would give a different mean.
    swapped = _expected(reference, groups, neg, pos)["stats"]["energy"]["mean"]
    assert got["stats"]["energy"]["mean"] != pytest.approx(swapped)


def test_score_reference_uses_default_prompts_when_none_sent(tmp_path, monkeypatch):
    reference, got = _score_reference(tmp_path, monkeypatch, {})
    assert set(got["stats"]) == set(moods.MOODS) | {"energy"}
    _assert_stats(got, _expected(reference, moods.MOODS, moods.ENERGY_POS, moods.ENERGY_NEG))


def test_score_without_reference_has_no_mood_reference(client):
    embed(client, [("h1", "hi-1")])
    body = client.post(
        "/score",
        json={"gate": {"positives": [], "negatives": []}, "targets": ["h1"]},
        headers=HEADERS,
    ).json()
    assert "mood" in body
    assert "mood_reference" not in body


def test_health_reports_reference(ref_client, client):
    got = ref_client.get("/health", headers=HEADERS).json()
    assert got["reference"] == {"set": "test-set", "tracks": 6}
    assert client.get("/health", headers=HEADERS).json()["reference"] is None


# --- /tune ---------------------------------------------------------------------

REPORT_KEYS = {
    "lines", "loss", "flags", "top", "top_before", "risers", "fallers", "fires_before",
    "fires_after", "overlap", "loo", "names", "unsatisfied",
}


class TuneEmbedder(FakeEmbedder):
    """Prompt texts mentioning "alpha" point along axis 0 and "beta" along axis 1;
    every other text gets its own fixed random direction."""

    @staticmethod
    def vec(text):
        v = np.random.default_rng(zlib.crc32(text.encode())).normal(size=DIM) * 0.3
        if "alpha" in text:
            v[0] += 3.0
        if "beta" in text:
            v[1] += 3.0
        return v / np.linalg.norm(v)

    def embed_text(self, texts):
        return np.vstack([self.vec(t) for t in texts])


def _tune_library(cache):
    # s0..s2 lean towards axis 0 (alpha songs), s3..s11 towards axis 1 (beta songs); a
    # minority group, so its z-scored bars can clear the 0.75 floor.
    rng = np.random.default_rng(5)
    for i in range(12):
        v = rng.normal(size=DIM) * 0.3
        v[0 if i < 3 else 1] += 2.0
        cache.put(f"s{i}", v[None, :].astype(np.float32))


def _tune_body(**over):
    body = {
        "mood_id": "sorrowful", "name": "Sad", "current_lines": ["beta"],
        "flags": [
            {"hash": "s0", "min": 0.75, "max": None, "weight": 1.0, "fresh": True, "side": "high"},
            {"hash": "s1", "min": 0.75, "max": None, "weight": 1.0, "fresh": True, "side": "high"},
            {"hash": "s6", "min": None, "max": 0.5, "weight": 1.0, "fresh": True, "side": "low"},
            {"hash": "s7", "min": None, "max": 0.5, "weight": 0.5, "fresh": False, "side": "low"},
        ],
        "targets": [f"s{i}" for i in range(12)],
        "bank": ["alpha", "beta", "gamma, delta"],
        "other_moods": {"grim": ["alpha beta"]},
        "calibration": {"source": "library", "midpoint_z": 0.5, "slope": 1.5},
    }
    body.update(over)
    return body


@pytest.fixture
def client_tune(tmp_path):
    cache = EmbeddingCache(tmp_path)
    _tune_library(cache)
    app = create_app(TuneEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2))
    return TestClient(app)


def test_tune_returns_report_shape(client_tune):
    r = client_tune.post("/tune", json=_tune_body(), headers=HEADERS)
    assert r.status_code == 200
    body = r.json()
    assert set(body) == {"baseline", "proposals", "split", "skipped"}
    assert body["skipped"] == [] and body["split"] is None
    for report in [body["baseline"], *body["proposals"]]:
        assert set(report) == REPORT_KEYS
        assert [f["hash"] for f in report["flags"]] == ["s0", "s1", "s6", "s7"]
        assert set(report["flags"][0]) == {"hash", "before", "after", "holds", "fresh"}
        assert len(report["top"]) == 12 and len(report["top_before"]) == 12   # only 12 songs
        assert [o["mood_id"] for o in report["overlap"]] == ["grim"]
        assert set(report["loo"]) == {"held", "total"}
    assert body["baseline"]["lines"] == ["beta"]
    assert 1 <= len(body["proposals"]) <= 3
    assert body["proposals"][0]["loo"]["total"] == 3          # three fresh flags


def test_tune_skips_unknown_hashes(client_tune):
    flags = _tune_body()["flags"] + [
        {"hash": "nope", "min": 0.75, "max": None, "weight": 1.0, "fresh": True, "side": "high"}
    ]
    r = client_tune.post("/tune", json=_tune_body(flags=flags), headers=HEADERS)
    assert r.status_code == 200
    assert r.json()["skipped"] == ["nope"]
    assert "nope" not in [f["hash"] for f in r.json()["baseline"]["flags"]]


def test_tune_requires_token(client_tune):
    assert client_tune.post("/tune", json=_tune_body()).status_code == 401


def test_tune_proposes_nothing_worse_than_baseline(client_tune):
    body = client_tune.post("/tune", json=_tune_body(), headers=HEADERS).json()
    assert body["proposals"]
    assert all(p["loss"] < body["baseline"]["loss"] for p in body["proposals"])
    assert [p["loss"] for p in body["proposals"]] == sorted(p["loss"] for p in body["proposals"])
    assert any("alpha" in line for line in body["proposals"][0]["lines"])


def test_tune_offers_no_proposals_when_the_current_lines_are_already_best(client_tune):
    body = client_tune.post("/tune", json=_tune_body(current_lines=["alpha"]), headers=HEADERS).json()
    assert body["baseline"]["unsatisfied"] == []
    assert body["proposals"] == []


def test_tune_reference_source_needs_a_reference_set(client_tune):
    cal = {"source": "reference", "midpoint_z": 0.5, "slope": 1.5}
    r = client_tune.post("/tune", json=_tune_body(calibration=cal), headers=HEADERS)
    assert r.status_code == 409
    assert r.json()["detail"] == "The analyzer has no reference set"


def test_tune_reads_bars_against_the_reference_set(tmp_path):
    cache = EmbeddingCache(tmp_path)
    _tune_library(cache)
    ref = np.random.default_rng(9).normal(size=(20, DIM))
    ref /= np.linalg.norm(ref, axis=1, keepdims=True)
    app = create_app(
        TuneEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2),
        reference=ReferenceSet("test-set", "fake@test", ref),
    )
    cal = {"source": "reference", "midpoint_z": 0.5, "slope": 1.5}
    ref_body = TestClient(app).post("/tune", json=_tune_body(calibration=cal), headers=HEADERS).json()
    lib_body = TestClient(app).post("/tune", json=_tune_body(), headers=HEADERS).json()
    # Same prompts, different yardstick: the baseline's bars differ.
    assert ref_body["baseline"]["top_before"] != lib_body["baseline"]["top_before"]


def test_tune_without_cached_targets_is_a_conflict(client_tune):
    r = client_tune.post("/tune", json=_tune_body(targets=["zzz"], flags=[]), headers=HEADERS)
    assert r.status_code == 409


def test_tune_while_model_fails_to_load_answers_503(tmp_path):
    class Broken(TuneEmbedder):
        def load(self):
            raise RuntimeError("no weights")

    cache = EmbeddingCache(tmp_path)
    _tune_library(cache)
    app = create_app(Broken(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2))
    r = TestClient(app).post("/tune", json=_tune_body(), headers=HEADERS)
    assert r.status_code == 503


def test_tune_reports_an_offered_split(client_tune, monkeypatch):
    # try_split's own rules are tested in test_tune.py; this checks the endpoint turns
    # an offer into two reports, each holding its high flags plus every low/mid flag.
    offer = {"a": tune.Combo(("alpha",), 0.0), "b": tune.Combo(("beta",), 0.0),
             "a_flags": [0], "b_flags": [1], "rho": 0.25}
    monkeypatch.setattr(app_module.tune, "try_split", lambda *a: offer)
    split = client_tune.post("/tune", json=_tune_body(), headers=HEADERS).json()["split"]
    assert set(split) == {"keep", "new", "keep_hashes", "new_hashes", "rho"}
    assert split["rho"] == 0.25
    # Which half keeps the name is decided by the name ranking; either way each report
    # belongs with its own group.
    by_lines = {"alpha": ["s0"], "beta": ["s1"]}
    assert split["keep_hashes"] == by_lines[split["keep"]["lines"][0]]
    assert split["new_hashes"] == by_lines[split["new"]["lines"][0]]
    assert {split["keep"]["lines"][0], split["new"]["lines"][0]} == {"alpha", "beta"}
    for half in (split["keep"], split["new"]):
        assert [f["hash"] for f in half["flags"]][1:] == ["s6", "s7"]
        assert set(half) == REPORT_KEYS
