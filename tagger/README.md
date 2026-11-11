# LuteDrop tagger

An analyzer service that embeds audio with MuQ-MuLan and scores tracks against
the mood prompts sent by LuteDrop. It keeps no mood state: every `/score` request
carries the current prompts, and the only thing stored is the per-track
embedding cache.

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

The tuner keeps the existing stability objective: flag fit, unflagged library
movement, changes in mood prevalence, and a small cost per line. Candidate
pruning reserves at most one third of its slots for individual flag specialists,
and beam frontiers preserve a few distinct patterns of flag failures. This lets
complementary descriptions reach combination search without changing the objective.
Comma blends include bounded specialist pairs and are embedded as actual text.
When the leading proposals all miss the same fresh flags, one alternative may
hold a strict superset of those flags; the loss winner stays first, and every
offered proposal must still beat Current. Arrange remains limited to the supplied
words; Suggest can explore the built-in vocabulary. No language model is used.

Every request needs the `X-LuteDrop-Token` header. A server with an empty
token rejects all requests.

| Endpoint | Request | Response |
|---|---|---|
| `GET /health` | none | `{model, cache_count, ready, error, reference}`; `error` is null unless the model failed to load; `reference` is `{set, tracks}` for the loaded reference set, or null |
| `POST /embed` | `{items: [{hash, path}]}`, at most 32 items | `{results: [{hash, ok, error?}]}`; cached hashes return immediately; 503 while the model is loading or if it failed to load |
| `POST /score` | `{targets, mood_prompts?: {id: [prompt]}, energy_prompts?: {positive: [prompt], negative: [prompt]}}`; fields older apps still send (`scene`, `gate`, `tags`, `pace`) are accepted and ignored | `{model, mood, mood_reference?}`; `mood` maps each target hash to `{moods: {id: raw}, energy: raw}` |
| `POST /preview` | `{prompts: [prompt], targets: [hash]}`, at least one prompt | `{hash: raw}`: each cached target's mean cosine with the prompts, uncalibrated, as a mood is scored in `/score`; for previewing a prompt set before saving it; 503 while the model is loading |
| `POST /similar` | `{query: [hash], candidates: [hash], limit}` | `{results: [{hash, score}]}`; cosine similarity to the normalized centroid of the query vectors, query hashes excluded, best first |
| `POST /search` | `{text, candidates: [hash], limit}` | `{results: [{hash, score}]}`; cosine similarity of the text embedding with each candidate's mean audio embedding; 503 while the model is loading |
| `POST /map` | `{hashes: [hash]}` | `{points: [{hash, x, y}]}`; 2D layout normalized to [0, 1] (t-SNE with cosine metric, PCA below 5 items), cached in memory by the sorted hash set |

`limit` defaults to 50 and is capped at 500. `mood` scores
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
never waits for it.

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
python scripts/build_reference.py --manifest tagger/reference/manifest.json \
    --audio-dir <dir> --out tagger/reference/reference.npz [--download] [--cache-dir <dir>]
```

Hashes with no cached embedding receive no score as targets.

## Saved mood profiles

Every mood uses the same description-and-examples workflow. Version 2 profiles
combine editable signed text descriptions with 19 fixed publisher-trained audio
signals. A line is embedded as a complete phrase (commas remain in it); a leading
`- ` makes it negative evidence. `/tune` with `mode: "learn"` and `extra_words`
previews an edited description, using `current_lines` when none are supplied.
It accepts zero examples: that produces a description-only profile with zero
audio weights. Sparse feedback is strongly regularized; nested leave-one-song-out
validation is reported only with at least five examples. These diagnostics are
not a claim that a personal mood is objectively correct.

Audio extraction still returns the original normalized 512-dimensional MuQ-MuLan
search vectors. In the same forward pass, layers 3, 6, 9 and 12 provide four
1024-dimensional vectors, pooled over frames and sampled windows. These are
cached separately under a pinned model revision. Nineteen small classifiers,
trained only on the 300 Incompetech recordings' published feel labels, convert
those hidden vectors into audio signals. The user's examples do not retrain
MuQ-MuLan or those classifiers; they fit a small combination of the audio signals
and reference-standardized text similarities.

For features `x` and saved coefficients `w`, the score is
`0.5 + atan(bias + x @ w) / pi`. This smooth, monotonic mapping has no flat clipping
plateau. Scores express fit to a personal concept, not probabilities or library
percentiles. The fixed reference statistics and coefficients make scores
independent of which other tracks are imported or scored in a batch.

Version 5 adds a fixed semantic-reference safeguard to directional fitting.
The 300 aligned Incompetech recordings supply 19 audio features and each
current text axis, standardized with the same frozen reference statistics.
`semantic-anchors.npz` is checked against the evidence ID and the exact
`reference.npz` SHA-256; missing or mismatched anchors refuse a new fit rather
than silently reverting to unconstrained binary learning.

The objective sums confidence/4 times feedback loss (it no longer averages
away the amount of evidence), plus .0001 times squared parameter changes,
plus a penalty for changing predictions on the fixed reference:
`anchor_strength * mean((reference_features @ (weights - word_prior)) ** 2)`.
Anchor strength is .3 without both positive and negative membership examples,
and .03 with both. Numeric overrides and historical relative limits keep their
own semantics; they are not counted as opposing membership examples. This
policy is shared by every mood, deterministic, and does not choose a noisy
hyperparameter winner. Held-out feedback error is still measured when there
are enough examples. No score histogram, percentile quota, or imported-library
statistics enter the safeguard. The reference penalizes unsupported shifts;
it does not label unjudged songs as negative or rewrite personal flags.
Saved inference and explicit Preview/Apply remain stable across imports.

Version 4 treats feedback as membership direction and conviction, not point
ratings. Definitely and No have confidence 4; Understandable and Stretch have
confidence 1. Positive judgments minimize -log(score), negative judgments
minimize -log(1-score). Losses are weighted by confidence/4 and averaged over
examples, plus the semantic prior penalty. No exact .9/.65/.35/.1 targets or
zero-loss membership intervals are used. Scores keep the same arctangent
mapping, so values above .9 stay ordered and agreeing more is always rewarded.
These are model match scores, not calibrated probabilities of a listener's opinion.

Higher/Lower compare with the original score anchor using a shifted latent
score. Explicit numeric overrides remain numeric; historical exceptional
ceilings/floors (e.g. Understandable or less) retain their original limit.
Optional [stronger, weaker] comparisons add .02 times mean softplus of the
negative latent-score difference, asking for order without a hard gap.
Nested song exclusion selects regularization using confidence-weighted log
loss, excluding all comparisons touching the held recording. Candidates whose
paired validation differences are within one estimated standard error favor
weaker shrinkage, avoiding strong pull toward wording for tiny validation wins.
Scoring details report held-out disagreement separately from training loss.

Frozen v1/v2/v3 profiles remain readable. Version 3 used point ratings; these
must not be interpreted as numerically equivalent to version 4 scores. The
one-time directional migration replaces all named mood profiles using their
unchanged descriptions and feedback, preserves flags and manual overrides,
and backs up the database before updating automatic scores. Future updates
remain explicit Preview/Apply; importing recordings never silently refits.
Positive and negative description weights retain their requested signs; audio
weights can be positive or negative for any mood. Regularization is selected by
exclusion when enough examples exist; otherwise a conservative fixed value is
used. Row focus and playback do not affect this fit.

The desktop stores an applied profile with the vocabulary and sends it in the
`/score` `profiles` map. Final scores arrive in each result's `learned` map and
bypass word-score recalibration. Creating a preview never activates it. Imports,
playback and feedback never silently refit a saved profile. Apply is explicit;
Undo restores the previous profile. Judgments keep their meaning and full
weight after a version change. Higher/Lower feedback retains its numeric
anchor and is not silently moved or converted into a different band.

Version 1 Horror profiles remain readable against the new evidence bundle for
exact comparison and rollback; they keep their original clipped mapping. The
upgrade script prepares and verifies version 2 profiles for every mood, rejects
changed source data, and uses SQLite's backup API before applying them. Manual
levels, flags, clips, scenes and other library metadata are preserved.

## Development

### Evidence-controlled learning

Since version 7, negative description axes use `max(0,z)` before
their negative weight. Absence of a negative quality contributes zero, rather
than a bonus for the mood. Positive descriptions retain their centered axes.
Saved versions 1–6 retain their exact earlier inference; the change does not
rescore or refit accepted profiles automatically.

All moods use the same policy. With no examples, signed descriptions alone
define scores. Otherwise flags first produce local corrections in the four
normalized 1024-dimensional MuQ audio layers. Influence decays with cosine
distance; the default kernel width is .05 and its regularization is .001.
Portable float32 exemplars and coefficients are saved with the profile, so
deleting an example or clearing its embedding cache cannot change the scorer.
The same fixed 300-song reference penalizes unsupported local drift (.03 with
both membership directions, .3 otherwise); it supplies no invented labels.

At least five judgments, including positive and negative membership examples,
are required to consider wider kernels (.1, .3) or reference-anchored global
fits (ridge .1, .01, .0001). Deterministic audio-grouped five-fold validation
compares candidates. A paired one-standard-error rule prefers narrower reach;
broader fits that cannot improve on the word baseline are rejected. An outer
set of excluded folds evaluates the complete selection procedure. Identical
audio stays in the same fold, and comparisons touching excluded examples are
removed. Small or one-sided sets retain local corrections. No unflagged song
is treated as a negative example and no library prevalence target is imposed.

The saved profile records Words only, Similar sounds, or Broad pattern.
Preview, wording proposals, Apply and subsequent scoring use the same scorer.
Versions 1–5 remain readable. New flags affect scores only after Preview/Apply;
imports never refit. `scripts/upgrade-directional-moods.py` now prepares version
7 for all moods and verifies preview/inference parity before a backed-up,
source-checked application. Words, flags and manual levels remain unchanged.

### Stable feedback updates (version 8)

When the wording is unchanged, Preview retains the accepted model's bias,
audio weights, word calibration, and original local correction. It fits a
separate residual with the existing narrow .05 audio kernel, .001 RKHS
regularization, and .03 fixed-reference drift penalty. It never reruns model
selection. A new flag therefore cannot switch the whole mood from a broad
model back to word-based local scoring. Neighboring audio can still move;
the residual is evidence of similarity, not an understanding of emotion.

The portable `adjustment` is replaced against the same frozen foundation on
each update, rather than added to the previous adjustment. Identical feedback
is idempotent, removal of feedback removes its residual influence, and new
unjudged imports cannot affect the fit. The foundation itself can still encode
earlier feedback. `wordVersion` preserves its exact original negative-axis
operator, including version 6. Old saved profiles retain their exact scores.

Changing wording, or explicitly choosing **Wording → Rebuild from wording**,
can relearn the whole mood through the validated selection policy above.
Both paths remain proposals until Apply. A local update reports no held-out
validation loss: its accepted foundation may already contain those examples.

```
python -m venv .venv
.venv/Scripts/pip install -r requirements-dev.txt   # Scripts -> bin on Linux/macOS
python -m pytest -q
```

Tests use a fake embedder, so torch and muq are not needed.

### Signed wording proposals

The desktop sends `scoring_version: 8` for wording search. Recombine and Suggest
then evaluate signed descriptions with the same combined scorer as Preview and
Apply. Recombination can split or merge phrases, remove terms and change their
polarity; Suggest also considers the word bank with either sign. A bounded
screening pass is followed by full fitting and exclusion diagnostics for the
finalists. These are proposals, not automatic edits or promises of improvement.
Previously proposed descriptions are excluded from repeat suggestions. Older
API callers retain the legacy word-only search unless they opt into version 2.
