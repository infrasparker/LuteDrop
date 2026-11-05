# LuteDrop tagger

An analyzer service that embeds audio with MuQ-MuLan and scores tracks against
tag examples sent by LuteDrop. It keeps no tag state: every `/score` request
carries the current positive and negative examples, and the only thing stored
is the per-track embedding cache.

## Deploy

Needs Docker with the NVIDIA container toolkit.

```
cp .env.example .env     # set LUTEDROP_TAGGER_TOKEN
docker compose up -d --build
curl -H "X-LuteDrop-Token: <token>" http://localhost:8765/health
```

The service starts loading the model as soon as it starts. On first start the
weights download into `./hf`, which can take several minutes; `/health`
reports `ready: false` until loading finishes, and `/embed` answers 503 in the
meantime. If loading fails, `/health` carries the reason in `error`. Embeddings
are cached as
`<content hash>.npy` files in `./cache`. The audio library is mounted read-only
at `/media/audio`; `path` values in `/embed` are paths inside the container.

## API

Every request needs the `X-LuteDrop-Token` header. A server with an empty
token rejects all requests.

| Endpoint | Request | Response |
|---|---|---|
| `GET /health` | none | `{model, cache_count, ready, error}`; `error` is null unless the model failed to load |
| `POST /embed` | `{items: [{hash, path}]}`, at most 32 items | `{results: [{hash, ok, error?}]}`; cached hashes return immediately; 503 while the model is loading or if it failed to load |
| `POST /score` | `{scene, gate: {positives, negatives}, tags: {name: {positives, negatives}}, targets}` | `{model, gate, tags}`, each head with `quality {auc, pos, neg}`, `trusted`, `threshold` and `scores` keyed by hash |

Hashes with no cached embedding are ignored as examples and receive no score
as targets.

## Development

```
python -m venv .venv
.venv/Scripts/pip install -r requirements-dev.txt   # Scripts -> bin on Linux/macOS
python -m pytest -q
```

Tests use a fake embedder, so torch and muq are not needed.
