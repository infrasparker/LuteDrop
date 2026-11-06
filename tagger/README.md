# LuteDrop tagger

An analyzer service that embeds audio with MuQ-MuLan and scores tracks against
tag examples sent by LuteDrop. It keeps no tag state: every `/score` request
carries the current positive and negative examples, and the only thing stored
is the per-track embedding cache.

## Deploy

Needs Docker with the NVIDIA container toolkit.

```
cp .env.example .env     # set LUTEDROP_TAGGER_TOKEN
docker compose up -d --build --remove-orphans
curl -H "X-LuteDrop-Token: <token>" http://localhost:8765/health
```

The container is named `lutedrop-analyzer`. The service starts loading the model as soon as it starts. On first start the
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
| `GET /health` | none | `{model, cache_count, ready, error, reference}`; `error` is null unless the model failed to load; `reference` is `{set, tracks}` for the loaded reference set, or null |
| `POST /embed` | `{items: [{hash, path}]}`, at most 32 items | `{results: [{hash, ok, error?}]}`; cached hashes return immediately; 503 while the model is loading or if it failed to load |
| `POST /score` | `{gate: {positives, negatives}, tags: {name: {positives, negatives}}, pace?: {positives, negatives}, targets, mood_prompts?: {id: [prompt]}, energy_prompts?: {positive: [prompt], negative: [prompt]}}`; an optional `scene` field is accepted and ignored | `{model, gate, tags, pace?, mood, mood_reference?}`, each head with `quality {auc, pos, neg}`, `trusted`, `threshold` and `scores` keyed by hash; `pace` is present only when requested; `mood` maps each target hash to `{moods: {id: raw}, energy: raw}` |
| `POST /preview` | `{prompts: [prompt], targets: [hash]}`, at least one prompt | `{hash: raw}`: each cached target's mean cosine with the prompts, uncalibrated, as a mood is scored in `/score`; for previewing a prompt set before saving it; 503 while the model is loading |
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
are raw, not calibrated: `moods` has one absolute score per mood id, the mean
cosine of the track's mean embedding with that mood's prompts. `energy` is
two-sided: the mean cosine with the energetic prompts minus the mean cosine
with the calm ones. The app calibrates them into 0..1 against a frozen
calibration.

The app sends its own mood vocabulary: `mood_prompts` (one list of prompts per
id; ids are lowercase letters, each list has at least one prompt) and
`energy_prompts` (both sides, at least one prompt each). Either one, when
present, replaces the matching defaults in `tagger/moods.py` (`peaceful`,
`sorrowful`, `wondrous`, `joyful`, `quirky`, `mysterious`, `grim`, `heroic`,
plus Energy); a bad vocabulary is a 422. `moods` in the response has exactly
the ids that were used. Prompt embeddings are cached per distinct text, so an
edited mood embeds only its new prompts. While the model
is still loading or has failed, `mood` is omitted from the response; `/score`
never waits for it, and `gate`, `tags` and `pace` are unaffected.

`mood_reference`, present whenever `mood` is non-empty and a reference set is
loaded, is `{set, tracks, stats: {id: {mean, sd}, energy: {mean, sd}}}`: the
mean and population standard deviation of each raw mood score (and Energy)
over the reference tracks, for the same prompts as `mood`. The app uses it to
read a score relative to typical soundtrack music.

## Reference set

`tagger/reference/reference.npz` (with `manifest.json` beside it) holds the
embeddings of a fixed set of soundtrack tracks, `incompetech-300-v1`. It ships
in the image and is loaded at start; if the file is missing, or was built with a
different model than the analyzer's, it is ignored and `mood_reference` is
omitted. Only embeddings are stored, never audio.

Tracks by Kevin MacLeod (incompetech.com), licensed under Creative Commons:
By Attribution 4.0 (https://creativecommons.org/licenses/by/4.0/).

To rebuild it (needs the model installed; checks each file against the
manifest's sha256):

```
python scripts/build_reference.py --manifest tagger/reference/manifest.json \n    --audio-dir <dir> --out tagger/reference/reference.npz [--download] [--cache-dir <dir>]
```

Hashes with no cached embedding are ignored as examples and receive no score
as targets.

## Development

```
python -m venv .venv
.venv/Scripts/pip install -r requirements-dev.txt   # Scripts -> bin on Linux/macOS
python -m pytest -q
```

Tests use a fake embedder, so torch and muq are not needed.
