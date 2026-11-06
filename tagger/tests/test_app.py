from concurrent.futures import Future, ThreadPoolExecutor
from concurrent.futures.process import BrokenProcessPool

import threading

import numpy as np
import pytest
from fastapi.testclient import TestClient

from tagger import app as app_module, rhythm
from tagger.app import create_app
from tagger.embed import EmbeddingCache

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
    assert body == {"model": "fake@test", "cache_count": 1, "ready": True, "error": None}


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
    real = app_module.vibe.layout_2d
    monkeypatch.setattr(
        app_module.vibe, "layout_2d", lambda v: (n_calls.append(1), real(v))[1]
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
def test_vibe_endpoints_require_token(client, path, body):
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
    texts = [t for group in moods.POOLS.values() for t in group]
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
    # One embed_text per pool, computed once.
    assert MoodEmbedder.calls == len(moods.POOLS)
    assert set(data["mood"]) == {"a", "b"}

    mean = cache.get("a")

    def pole(texts):
        return float(np.mean([mean @ dirs[t] for t in texts]))

    # Only pools: the mood scales are gone.
    assert set(data["mood"]["a"]) == {"pools"}
    # Pools are absolute similarities (mean cosine with the pool's prompts).
    pools = data["mood"]["a"]["pools"]
    assert set(pools) == {"peaceful", "lighthearted", "somber", "awe", "mysterious", "epic"}
    for pool_id, prompts in moods.POOLS.items():
        assert pools[pool_id] == pytest.approx(pole(prompts))
    assert any(v != 0 for v in pools.values())


def test_score_mood_recovers_after_partial_embed_failure(tmp_path, monkeypatch):
    class FlakyEmbedder(FakeEmbedder):
        calls = 0

        def embed_text(self, texts):
            FlakyEmbedder.calls += 1
            if FlakyEmbedder.calls == 3:  # fail part-way through the pools
                raise RuntimeError("transient")
            return super().embed_text(texts)

    monkeypatch.setattr(app_module, "decode_track", lambda p: (synthetic(1.0, 1), FEATS))
    cache = EmbeddingCache(tmp_path)
    c = TestClient(create_app(FlakyEmbedder(), cache, TOKEN, executor_factory=lambda: ThreadPoolExecutor(2)))
    embed(c, [("a", "x-1")])
    body = {"gate": {}, "targets": ["a"]}
    first = c.post("/score", json=body, headers=HEADERS)
    assert first.status_code == 200 and "mood" not in first.json()
    second = c.post("/score", json=body, headers=HEADERS)
    assert second.status_code == 200
    assert set(second.json()["mood"]["a"]) == {"pools"}
    assert len(second.json()["mood"]["a"]["pools"]) == 6


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
