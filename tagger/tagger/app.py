import hmac
import os
import threading
from collections.abc import Callable
from concurrent.futures import Executor, ProcessPoolExecutor
from concurrent.futures.process import BrokenProcessPool
from pathlib import Path

import numpy as np
from fastapi import Depends, FastAPI, Header, HTTPException
from pydantic import BaseModel, Field

from . import heads
from .embed import Embedder, EmbeddingCache, MuqEmbedder, load_windows

MAX_EMBED_ITEMS = 32


class EmbedItem(BaseModel):
    hash: str
    path: str


class EmbedRequest(BaseModel):
    items: list[EmbedItem] = Field(max_length=MAX_EMBED_ITEMS)


class Examples(BaseModel):
    positives: list[str] = []
    negatives: list[str] = []


class ScoreRequest(BaseModel):
    scene: str = ""
    gate: Examples
    tags: dict[str, Examples] = {}
    targets: list[str] = []


def _decode(path: str) -> np.ndarray:
    # Module-level so a process pool can pickle it by reference.
    return load_windows(path)


def _failure(hash: str, e: Exception) -> dict:
    return {"hash": hash, "ok": False, "error": str(e) or type(e).__name__}


def create_app(
    embedder: Embedder,
    cache: EmbeddingCache,
    token: str,
    executor_factory: Callable[[], Executor] | None = None,
) -> FastAPI:
    make_pool = executor_factory or (lambda: ProcessPoolExecutor(os.cpu_count()))
    pool_lock = threading.Lock()
    current = [make_pool()]

    def submit(path: str):
        with pool_lock:
            pool = current[0]
            return pool, pool.submit(_decode, path)

    def replace_pool(broken: Executor) -> None:
        """Swap in a fresh pool once a worker has died; a no-op if another
        request already did."""
        with pool_lock:
            if current[0] is broken:
                broken.shutdown(wait=False, cancel_futures=True)
                current[0] = make_pool()

    def require_token(x_lutedrop_token: str = Header(default="")) -> None:
        ok = hmac.compare_digest(x_lutedrop_token.encode(), token.encode())
        if not token or not ok:
            raise HTTPException(status_code=401, detail="unauthorized")

    app = FastAPI(dependencies=[Depends(require_token)])

    @app.get("/health")
    def health():
        return {"model": embedder.name, "cache_count": cache.count()}

    @app.post("/embed")
    def embed(req: EmbedRequest):
        results: dict[str, dict] = {}
        pending = {}
        for item in req.items:
            if cache.has(item.hash):
                results[item.hash] = {"hash": item.hash, "ok": True}
            elif item.hash not in pending:
                try:
                    pending[item.hash] = submit(item.path)
                except BrokenProcessPool as e:
                    replace_pool(current[0])
                    results[item.hash] = _failure(item.hash, e)
        for hash, (pool, future) in pending.items():
            try:
                windows = future.result()
                cache.put(hash, embedder.embed_windows(windows))
                results[hash] = {"hash": hash, "ok": True}
            except BrokenProcessPool as e:
                replace_pool(pool)
                results[hash] = _failure(hash, e)
            except Exception as e:
                results[hash] = _failure(hash, e)
        return {"results": [results[i.hash] for i in req.items]}

    def vectors(hashes: list[str]) -> tuple[list[str], np.ndarray]:
        """Mean embeddings for the hashes that are cached, in input order."""
        found, rows = [], []
        for h in dict.fromkeys(hashes):
            v = cache.get(h)
            if v is not None:
                found.append(h)
                rows.append(v)
        return found, (np.vstack(rows) if rows else np.empty((0, 0)))

    def score_head(examples: Examples, target_hashes, X_targets):
        _, X_pos = vectors(examples.positives)
        _, X_neg = vectors(examples.negatives)
        # fit_and_score cannot predict on an empty array; score one example
        # row instead and drop the result when there are no targets.
        probe = X_targets if len(target_hashes) else (X_pos[:1] if len(X_pos) else X_neg[:1])
        r = heads.fit_and_score(X_pos, X_neg, probe)
        return {
            "quality": {"auc": r.auc, "pos": r.pos, "neg": r.neg},
            "trusted": r.trusted,
            "threshold": r.threshold,
            "scores": {h: float(s) for h, s in zip(target_hashes, r.scores)}
            if len(target_hashes)
            else {},
        }

    @app.post("/score")
    def score(req: ScoreRequest):
        target_hashes, X_targets = vectors(req.targets)
        return {
            "model": embedder.name,
            "gate": score_head(req.gate, target_hashes, X_targets),
            "tags": {
                tag: score_head(ex, target_hashes, X_targets)
                for tag, ex in req.tags.items()
            },
        }

    return app


def factory() -> FastAPI:
    return create_app(
        MuqEmbedder(),
        EmbeddingCache(Path(os.environ.get("TAGGER_CACHE_DIR", "/cache"))),
        os.environ.get("LUTEDROP_TAGGER_TOKEN", ""),
    )


def main() -> None:
    import uvicorn

    uvicorn.run(factory(), host="0.0.0.0", port=8765)


if __name__ == "__main__":
    main()
