from concurrent.futures import Future, ThreadPoolExecutor
from concurrent.futures.process import BrokenProcessPool

import numpy as np
import pytest
from fastapi.testclient import TestClient

from tagger import app as app_module
from tagger.app import create_app
from tagger.embed import EmbeddingCache

TOKEN = "secret"
HEADERS = {"X-LuteDrop-Token": TOKEN}
DIM = 8


class FakeEmbedder:
    name = "fake@test"

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

    monkeypatch.setattr(app_module, "load_windows", fake_load)
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
    assert body == {"model": "fake@test", "cache_count": 1}


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
    monkeypatch.setattr(app_module, "load_windows", lambda p: synthetic(1.0, 1))
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
