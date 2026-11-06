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
| `POST /score` | `{gate: {positives, negatives}, tags: {name: {positives, negatives}}, pace?: {positives, negatives}, targets}`; an optional `scene` field is accepted and ignored | `{model, gate, tags, pace?, mood}`, each head with `quality {auc, pos, neg}`, `trusted`, `threshold` and `scores` keyed by hash; `pace` is present only when requested; `mood` maps each target hash to `{pools: {id: raw}}` |
| `POST /similar` | `{query: [hash], candidates: [hash], limit}` | `{results: [{hash, score}]}`; cosine similarity to the normalized centroid of the query vectors, query hashes excluded, best first |
| `POST /search` | `{text, candidates: [hash], limit}` | `{results: [{hash, score}]}`; cosine similarity of the text embedding with each candidate's mean audio embedding; 503 while the model is loading |
| `POST /map` | `{hashes: [hash]}` | `{points: [{hash, x, y}]}`; 2D layout normalized to [0, 1] (t-SNE with cosine metric, PCA below 5 items), cached in memory by the sorted hash set |

`limit` defaults to 50 and is capped at 500. The `/score` gate also returns
`neg_threshold`, the leave-one-out precision threshold applied to `1 - score`
with the classes swapped. It is null when no cutoff reaches the target
precision, including when a class has fewer than two examples, so consumers
check `trusted` before using it.

`pace` is a head trained on the cached rhythm features (standardized, in the
fixed `rhythm.FEATURES` order) instead of embeddings. Its examples and targets
need rhythm features, so hashes without them are ignored and receive no pace
score. `pace` is deprecated: apps after v0.12 no longer send it, and the head
and rhythm features can be removed once older apps are retired. `mood` scores
are raw, not z-scored: `pools` has one absolute score per mood pool
(`peaceful`, `lighthearted`, `somber`, `awe`, `mysterious`, `epic`): the mean
cosine of the track's mean embedding with that pool's prompts. The app z-scores them against a frozen calibration and
suggests the best pool. The prompt embeddings are
computed once, the first time `/score` finds the model loaded. While the model
is still loading or has failed, `mood` is omitted from the response; `/score`
never waits for it, and `gate`, `tags` and `pace` are unaffected.

Hashes with no cached embedding are ignored as examples and receive no score
as targets.

## Development

```
python -m venv .venv
.venv/Scripts/pip install -r requirements-dev.txt   # Scripts -> bin on Linux/macOS
python -m pytest -q
```

Tests use a fake embedder, so torch and muq are not needed.
