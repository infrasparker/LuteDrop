//! Pure planning for auto-tagging: which entries train the analyzer, and which
//! role and tags its scores earn. Nothing here touches the database or the
//! network.

use crate::models::{AutoTag, EntryTagScores, MoodId, Role, SceneConfig, TagScore};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Name the gate's scores and quality carry in saved scores and reports.
pub const GATE_NAME: &str = "combat";

/// What the planner needs to know about one library entry.
#[derive(Debug, Clone)]
pub struct EntryFacts {
    pub id: String,
    /// SHA-256 of the file; `None` until the entry has been hashed.
    pub hash: Option<String>,
    /// Tags the user voted yes on (the analyzer's tags are not included).
    pub user_tags: Vec<String>,
    /// Tags the user voted no on.
    pub rejected: Vec<String>,
    pub is_music: bool,
    /// The user's role (`role_user`); `None` when the user has not set one,
    /// whatever the analyzer says.
    pub user_role: Option<Role>,
}

/// Hashes of the entries that are examples for one tag (or for the gate).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Examples {
    pub positives: Vec<String>,
    pub negatives: Vec<String>,
}

/// Body of the analyzer's `POST /score`. The gate is the Combat vs
/// Non-combat classifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreRequest {
    pub gate: Examples,
    pub tags: HashMap<String, Examples>,
    pub targets: Vec<String>,
}

/// How well a classifier did on its own training examples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quality {
    /// `None` when the examples were too few to measure.
    pub auc: Option<f64>,
    pub pos: u32,
    pub neg: u32,
}

/// One classifier's result: the gate's, or one vocabulary tag's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreSet {
    pub quality: Quality,
    pub trusted: bool,
    /// Score an entry needs to be tagged; `None` if no cutoff was good enough.
    pub threshold: Option<f64>,
    /// For the gate: the value `1 - score` needs to reach for Non-combat.
    /// `None` if no cutoff was good enough, or if the analyzer sent none
    /// (vocabulary tags may leave it out).
    #[serde(default)]
    pub neg_threshold: Option<f64>,
    /// Score per entry hash. Only entries with a cached embedding appear.
    pub scores: HashMap<String, f64>,
}

/// Response body of the analyzer's `POST /score`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreResponse {
    pub model: String,
    pub gate: ScoreSet,
    pub tags: HashMap<String, ScoreSet>,
    /// Raw mood scores per entry hash. Empty when the analyzer sent none
    /// (its mood model is not loaded, or it predates mood). An entry without
    /// the `{moods, energy}` shape (an analyzer that predates the mood
    /// profile) is left out, as if the analyzer sent no mood for it.
    #[serde(default, deserialize_with = "complete_moods")]
    pub mood: HashMap<String, MoodRaw>,
}

/// The analyzer's raw mood scores for one entry: each mood's absolute prompt
/// similarity keyed by mood id, and Energy (energetic minus calm prompt
/// similarity). Any other field the analyzer sends (`pools`, or the scales
/// `bright`, `grand`, `sad`, `scary` and `light`, from older analyzers) is
/// ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoodRaw {
    pub moods: HashMap<String, f64>,
    pub energy: f64,
}

/// Reads the `mood` map, keeping only the entries in the current shape.
fn complete_moods<'de, D: Deserializer<'de>>(d: D) -> Result<HashMap<String, MoodRaw>, D::Error> {
    let raw: HashMap<String, serde_json::Value> = HashMap::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|(hash, value)| Some((hash, serde_json::from_value(value).ok()?)))
        .collect())
}

/// A calibration needs raw mood scores from at least this many music tracks.
pub const CALIBRATION_MIN_TRACKS: usize = 50;

/// The slope of the logistic that turns a z-score into 0..1:
/// `score = 1 / (1 + exp(-MOOD_SLOPE * z))`.
pub const MOOD_SLOPE: f64 = 1.5;

/// Mean and standard deviation of one mood's raw score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MoodStat {
    pub mean: f64,
    pub sd: f64,
}

/// The frozen mood calibration: per mood id (Energy included), the mean and
/// population standard deviation of its raw score across the library's music
/// when it was taken. Taken once, from the first `/score` response that
/// covers at least `CALIBRATION_MIN_TRACKS` music tracks, and never
/// recomputed automatically, so values do not drift as the library grows.
/// Stored as JSON `{ id: {mean, sd}, ... }` in the `mood_calibration`
/// setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MoodCalibration(pub BTreeMap<String, MoodStat>);

/// The raw scores of one entry, in `MoodId::ALL` order, or `None` if any mood
/// is missing (an analyzer with another vocabulary).
fn mood_raws(mood: &MoodRaw) -> Option<[f64; MoodId::COUNT]> {
    let mut out = [0.0; MoodId::COUNT];
    for (slot, id) in out.iter_mut().zip(MoodId::ALL) {
        *slot = match id {
            MoodId::Energy => mood.energy,
            _ => *mood.moods.get(id.as_db_str())?,
        };
    }
    Some(out)
}

/// Computes a mood calibration from one `/score` response's raw mood scores
/// (population standard deviation). `None` when fewer than
/// `CALIBRATION_MIN_TRACKS` entries carry every score, or when an id has no
/// spread (its z-scores would be meaningless). The response only holds music
/// targets, so every row is a music track.
pub fn calibrate(mood: &HashMap<String, MoodRaw>) -> Option<(MoodCalibration, usize)> {
    let rows: Vec<[f64; MoodId::COUNT]> = mood.values().filter_map(mood_raws).collect();
    if rows.len() < CALIBRATION_MIN_TRACKS {
        return None;
    }
    let n = rows.len() as f64;
    let mut stats = BTreeMap::new();
    for (i, id) in MoodId::ALL.iter().enumerate() {
        let mean = rows.iter().map(|r| r[i]).sum::<f64>() / n;
        let sd = (rows.iter().map(|r| (r[i] - mean).powi(2)).sum::<f64>() / n).sqrt();
        if !sd.is_finite() || sd <= 1e-12 {
            return None;
        }
        stats.insert(id.as_db_str().to_string(), MoodStat { mean, sd });
    }
    Some((MoodCalibration(stats), rows.len()))
}

/// Turns one raw score into 0..1: `z = (raw - mean) / sd`, then
/// `1 / (1 + exp(-MOOD_SLOPE * z))`, clamped to [0, 1].
pub fn mood_score(raw: f64, stat: MoodStat) -> f64 {
    let z = (raw - stat.mean) / stat.sd;
    (1.0 / (1.0 + (-MOOD_SLOPE * z).exp())).clamp(0.0, 1.0)
}

/// The analyzer's 0..1 value for every mood id of one entry, in
/// `MoodId::ALL` order. `None` when the entry lacks a raw score or the
/// calibration lacks an id.
///
/// Rule: moods never depend on Role. Only the entry's raw scores and the
/// frozen calibration go in; a combat track can be Peaceful.
///
/// Learned mood heads are out of scope for this release: the user's levels
/// are recorded in `library_entry_moods.user_level` for a later release to
/// regress heads on, and the values here are zero-shot only.
pub fn auto_moods(mood: &MoodRaw, calibration: &MoodCalibration) -> Option<[(MoodId, f64); MoodId::COUNT]> {
    let raws = mood_raws(mood)?;
    let mut out = [(MoodId::Peaceful, 0.0); MoodId::COUNT];
    for ((slot, id), raw) in out.iter_mut().zip(MoodId::ALL).zip(raws) {
        let stat = *calibration.0.get(id.as_db_str())?;
        *slot = (id, mood_score(raw, stat));
    }
    Some(out)
}

/// The analyzer layer one scored entry should carry after a run. It is
/// decided from the scores alone, whatever the user's layer holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// The auto role and the Combat probability it is stored with. `None`
    /// clears the auto role.
    pub role: Option<(Role, f64)>,
    /// The analyzer's value for every mood id, when there is a mood
    /// calibration and the analyzer sent the entry's raw mood scores.
    pub moods: Option<[(MoodId, f64); MoodId::COUNT]>,
    /// The tags the analyzer gives the entry; its other auto scores go.
    pub tags: Vec<AutoTag>,
}

/// Tags are stored trimmed and lowercase, so compare them that way.
fn norm(tag: &str) -> String {
    tag.trim().to_lowercase()
}

/// The vocabulary, normalized, without duplicates or blanks, in the order it
/// was written.
fn vocabulary(scene: &SceneConfig) -> Vec<String> {
    let mut seen = HashSet::new();
    scene
        .vocabulary
        .iter()
        .map(|t| norm(t))
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .collect()
}

fn has_tag(tags: &[String], wanted: &str) -> bool {
    tags.iter().any(|t| norm(t) == wanted)
}

/// Turns a local library path into the path the analyzer host sees.
///
/// The prefix match ignores ASCII case and treats `\` and `/` alike. Returns
/// `None` when `local_prefix` is empty, the path is not inside it, nothing
/// follows the prefix, or the rest of the path has a `..` segment.
pub fn map_path(local: &str, local_prefix: &str, remote_prefix: &str) -> Option<String> {
    let local = local.replace('\\', "/");
    let prefix = local_prefix.replace('\\', "/");
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() || local.len() < prefix.len() || !local.is_char_boundary(prefix.len()) {
        return None;
    }
    let (head, rest) = local.split_at(prefix.len());
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    // "Z:/media/audio2" must not match the prefix "Z:/media/audio".
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() || rest.split('/').any(|part| part == "..") {
        return None;
    }
    let remote = remote_prefix.trim_end_matches('/');
    Some(format!("{remote}/{rest}"))
}

/// Builds the `/score` request from the user's layer only.
///
/// Only music entries with a hash take part. The gate's positives are entries
/// the user set to Combat and its negatives those set to Non-combat. For each
/// vocabulary tag, positives are entries the user voted yes on; negatives are
/// entries the user voted no on, plus entries with a user role but no yes
/// vote for the tag. The analyzer's layer never appears in `EntryFacts`, so
/// it cannot train on its own guesses. Targets are every music entry whose
/// hash is in `embedded`.
pub fn build_score_request(
    entries: &[EntryFacts],
    scene: &SceneConfig,
    embedded: &HashSet<String>,
) -> ScoreRequest {
    let hashed: Vec<(&EntryFacts, &String)> = entries
        .iter()
        .filter(|e| e.is_music)
        .filter_map(|e| Some((e, e.hash.as_ref()?)))
        .collect();

    let mut gate = Examples::default();
    for (entry, hash) in &hashed {
        match entry.user_role {
            Some(Role::Combat) => gate.positives.push((*hash).clone()),
            Some(Role::NonCombat) => gate.negatives.push((*hash).clone()),
            None => {}
        }
    }

    let mut tags = HashMap::new();
    for vocab_tag in vocabulary(scene) {
        let mut examples = Examples::default();
        for (entry, hash) in &hashed {
            if has_tag(&entry.user_tags, &vocab_tag) {
                examples.positives.push((*hash).clone());
            } else if entry.user_role.is_some() || has_tag(&entry.rejected, &vocab_tag) {
                // A user role means the user has looked at the entry, so a
                // missing tag counts as "no".
                examples.negatives.push((*hash).clone());
            }
        }
        tags.insert(vocab_tag, examples);
    }

    let targets = hashed
        .iter()
        .filter(|(_, hash)| embedded.contains(*hash))
        .map(|(_, hash)| (*hash).clone())
        .collect();

    ScoreRequest { gate, tags, targets }
}

/// The role a Combat probability `p` earns from the gate: Combat when the
/// gate is trusted and `p` reaches its threshold, otherwise Non-combat when
/// the gate is trusted and `1 - p` reaches the Non-combat threshold,
/// otherwise none.
fn role_for(gate: &ScoreSet, p: f64) -> Option<(Role, f64)> {
    if !gate.trusted {
        return None;
    }
    if gate.threshold.is_some_and(|t| p >= t) {
        Some((Role::Combat, p))
    } else if gate.neg_threshold.is_some_and(|t| 1.0 - p >= t) {
        Some((Role::NonCombat, p))
    } else {
        None
    }
}

/// Decides the analyzer layer (role, moods and tags) of each scored
/// entry.
///
/// Only entries the analyzer scored (music with a hash found in the gate's
/// scores) appear in the result, so callers can tell them apart from entries
/// that were not scored. The decision depends on the scores alone: entries
/// the user has set get an analyzer layer too, shown beside the user's. A tag
/// is earned when its classifier is trusted and the score reaches its
/// threshold. Moods are scored only with a `calibration` (see
/// `auto_moods`).
pub fn decide(
    entries: &[EntryFacts],
    scene: &SceneConfig,
    resp: &ScoreResponse,
    calibration: Option<&MoodCalibration>,
) -> HashMap<String, Decision> {
    let vocabulary = vocabulary(scene);

    let mut decisions = HashMap::new();
    for entry in entries.iter().filter(|e| e.is_music) {
        let Some(hash) = &entry.hash else { continue };
        let Some(&gate_score) = resp.gate.scores.get(hash) else {
            continue;
        };

        let role = role_for(&resp.gate, gate_score);

        let mut tags = Vec::new();
        for vocab_tag in &vocabulary {
            let Some(set) = resp.tags.get(vocab_tag) else {
                continue;
            };
            let Some(&score) = set.scores.get(hash) else {
                continue;
            };
            if set.trusted && set.threshold.is_some_and(|t| score >= t) {
                tags.push(AutoTag {
                    tag: vocab_tag.clone(),
                    score,
                });
            }
        }

        decisions.insert(
            entry.id.clone(),
            Decision {
                role,
                moods: calibration.and_then(|cal| auto_moods(resp.mood.get(hash)?, cal)),
                tags,
            },
        );
    }
    decisions
}

/// The gate score and every tag score the analyzer produced for one entry,
/// for showing in the UI. The gate's `tag` is `"combat"`. `applied` is left
/// empty; it depends on the entry's current role and tags and is filled in
/// when the scores are loaded.
///
/// Returns `None` if the analyzer did not score this hash.
pub fn entry_scores(hash: &str, resp: &ScoreResponse) -> Option<EntryTagScores> {
    let to_score = |tag: &str, set: &ScoreSet| {
        set.scores.get(hash).map(|&score| TagScore {
            tag: tag.to_string(),
            score,
            threshold: set.threshold,
            trusted: set.trusted,
            applied: None,
        })
    };

    let gate = to_score(GATE_NAME, &resp.gate)?;
    let mut tags: Vec<TagScore> = resp
        .tags
        .iter()
        .filter_map(|(name, set)| to_score(name, set))
        .collect();
    tags.sort_by(|a, b| a.tag.cmp(&b.tag));
    Some(EntryTagScores { gate, tags })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Role;

    fn scene() -> SceneConfig {
        SceneConfig {
            vocabulary: vec!["brass".into(), "dark".into()],
        }
    }

    fn entry(id: &str, user_tags: &[&str]) -> EntryFacts {
        EntryFacts {
            id: id.into(),
            hash: Some(format!("h-{id}")),
            user_tags: user_tags.iter().map(|t| t.to_string()).collect(),
            rejected: vec![],
            is_music: true,
            user_role: None,
        }
    }

    /// The entry with the user's role.
    fn with_role(mut e: EntryFacts, role: Role) -> EntryFacts {
        e.user_role = Some(role);
        e
    }

    fn set(trusted: bool, threshold: Option<f64>, scores: &[(&str, f64)]) -> ScoreSet {
        ScoreSet {
            quality: Quality {
                auc: Some(0.9),
                pos: 10,
                neg: 10,
            },
            trusted,
            threshold,
            neg_threshold: None,
            scores: scores.iter().map(|(h, s)| (h.to_string(), *s)).collect(),
        }
    }

    /// A gate set with both thresholds.
    fn gate(
        trusted: bool,
        threshold: Option<f64>,
        neg_threshold: Option<f64>,
        scores: &[(&str, f64)],
    ) -> ScoreSet {
        ScoreSet {
            neg_threshold,
            ..set(trusted, threshold, scores)
        }
    }

    fn response(gate: ScoreSet, tags: &[(&str, ScoreSet)]) -> ScoreResponse {
        ScoreResponse {
            mood: HashMap::new(),
            model: "test".into(),
            gate,
            tags: tags
                .iter()
                .map(|(n, s)| (n.to_string(), s.clone()))
                .collect(),
        }
    }

    fn tag_names(decision: &Decision) -> Vec<&str> {
        decision.tags.iter().map(|t| t.tag.as_str()).collect()
    }

    fn hashes(list: &[&str]) -> Vec<String> {
        list.iter().map(|h| h.to_string()).collect()
    }

    #[test]
    fn map_path_handles_case_and_slashes() {
        assert_eq!(
            map_path(
                "z:/Media/Audio/music/A/b.opus",
                "Z:\\media\\audio",
                "/media/audio"
            ),
            Some("/media/audio/music/A/b.opus".to_string())
        );
    }

    #[test]
    fn map_path_outside_prefix_is_none() {
        assert_eq!(
            map_path("D:/other/a.opus", "Z:/media/audio", "/media/audio"),
            None
        );
        // Same leading letters, different folder.
        assert_eq!(
            map_path("Z:/media/audio2/a.opus", "Z:/media/audio", "/media/audio"),
            None
        );
    }

    #[test]
    fn map_path_rejects_empty_remainder_and_parent_segments() {
        assert_eq!(
            map_path("Z:/media/audio", "Z:/media/audio", "/media/audio"),
            None
        );
        assert_eq!(
            map_path("Z:/media/audio/", "Z:/media/audio", "/media/audio"),
            None
        );
        assert_eq!(
            map_path(
                "Z:/media/audio/../secret.opus",
                "Z:/media/audio",
                "/media/audio"
            ),
            None
        );
    }

    #[test]
    fn map_path_empty_prefix_is_none() {
        assert_eq!(map_path("Z:/media/audio/a.opus", "", "/media/audio"), None);
    }

    #[test]
    fn decide_dedupes_vocabulary() {
        let cfg = SceneConfig {
            vocabulary: vec!["brass".into(), "Brass".into(), " ".into()],
        };
        let entries = vec![entry("a", &[])];
        let resp = response(
            gate(true, Some(0.6), None, &[("h-a", 0.9)]),
            &[("brass", set(true, Some(0.5), &[("h-a", 0.7)]))],
        );
        let out = decide(&entries, &cfg, &resp, None);
        assert_eq!(tag_names(&out["a"]), vec!["brass"]);

        let req = build_score_request(&entries, &cfg, &HashSet::new());
        assert_eq!(req.tags.keys().collect::<Vec<_>>(), vec!["brass"]);
    }

    #[test]
    fn request_excludes_auto_tags_from_training() {
        // `EntryFacts` holds user tags and user roles only: an entry whose
        // tags and role are all auto has none, so it is no example at all.
        let entries = vec![
            with_role(entry("a", &["brass"]), Role::Combat),
            entry("auto-only", &[]),
        ];
        let req = build_score_request(&entries, &scene(), &HashSet::new());
        assert_eq!(req.gate.positives, hashes(&["h-a"]));
        assert!(req.gate.negatives.is_empty());
        let brass = &req.tags["brass"];
        assert!(!brass.positives.contains(&"h-auto-only".to_string()));
        assert!(!brass.negatives.contains(&"h-auto-only".to_string()));
    }

    #[test]
    fn request_uses_user_roles_for_gate() {
        let mut not_music = with_role(entry("n", &[]), Role::Combat);
        not_music.is_music = false;
        let entries = vec![
            with_role(entry("a", &[]), Role::Combat),
            with_role(entry("b", &[]), Role::NonCombat),
            entry("unset", &[]),
            not_music,
        ];
        let req = build_score_request(&entries, &scene(), &HashSet::new());
        assert_eq!(req.gate.positives, hashes(&["h-a"]));
        assert_eq!(req.gate.negatives, hashes(&["h-b"]));
    }

    #[test]
    fn request_tag_negatives_require_user_role() {
        let mut rejected = entry("rej", &[]);
        rejected.rejected = vec!["brass".into()];
        let mut not_music = with_role(entry("n", &[]), Role::NonCombat);
        not_music.is_music = false;
        let entries = vec![
            with_role(entry("a", &["brass"]), Role::Combat),
            with_role(entry("b", &["Dark "]), Role::NonCombat),
            entry("unset", &[]),
            // A user tag is a positive with or without a role.
            entry("tagged", &["brass"]),
            rejected,
            not_music,
        ];
        let req = build_score_request(&entries, &scene(), &HashSet::new());
        assert_eq!(req.tags["brass"].positives, hashes(&["h-a", "h-tagged"]));
        assert_eq!(req.tags["brass"].negatives, hashes(&["h-b", "h-rej"]));
        assert_eq!(req.tags["dark"].positives, hashes(&["h-b"]));
        assert_eq!(req.tags["dark"].negatives, hashes(&["h-a"]));
    }

    #[test]
    fn request_targets_only_embedded_music() {
        let mut not_music = entry("n", &[]);
        not_music.is_music = false;
        let mut no_hash = entry("x", &[]);
        no_hash.hash = None;
        let entries = vec![entry("a", &[]), entry("b", &[]), not_music, no_hash];
        let embedded: HashSet<String> = ["h-a", "h-n"].iter().map(|s| s.to_string()).collect();
        let req = build_score_request(&entries, &scene(), &embedded);
        assert_eq!(req.targets, hashes(&["h-a"]));
    }

    #[test]
    fn decide_sets_combat_and_non_combat_by_thresholds() {
        let entries = vec![entry("a", &[]), entry("b", &[]), entry("c", &[])];
        let scores = [("h-a", 0.9), ("h-b", 0.2), ("h-c", 0.4)];

        let out = decide(&entries, &scene(), &response(gate(true, Some(0.6), Some(0.7), &scores), &[]), None);
        assert_eq!(out["a"].role, Some((Role::Combat, 0.9)));
        // 1 - 0.2 = 0.8 clears the Non-combat threshold; the score kept is p.
        assert_eq!(out["b"].role, Some((Role::NonCombat, 0.2)));
        // 0.4 is below 0.6, and 0.6 is below 0.7.
        assert_eq!(out["c"].role, None);

        // Without a Non-combat threshold only Combat can be set.
        let out = decide(&entries, &scene(), &response(gate(true, Some(0.6), None, &scores), &[]), None);
        assert_eq!(out["a"].role, Some((Role::Combat, 0.9)));
        assert_eq!(out["b"].role, None);

        // An untrusted gate sets no role.
        let out = decide(&entries, &scene(), &response(gate(false, Some(0.6), Some(0.7), &scores), &[]), None);
        assert!(out.values().all(|d| d.role.is_none()));
    }

    #[test]
    fn decide_gives_user_set_entries_an_auto_role_too() {
        let entries = vec![with_role(entry("a", &[]), Role::NonCombat)];
        let resp = response(gate(true, Some(0.6), Some(0.6), &[("h-a", 0.95)]), &[]);
        let out = decide(&entries, &scene(), &resp, None);
        assert_eq!(out["a"].role, Some((Role::Combat, 0.95)));
    }

    #[test]
    fn decide_tags_need_no_gate() {
        let entries = vec![entry("a", &[]), with_role(entry("b", &[]), Role::NonCombat)];
        // The gate is untrusted and scores low; tags are decided on their own.
        let resp = response(
            gate(false, None, None, &[("h-a", 0.1), ("h-b", 0.1)]),
            &[("brass", set(true, Some(0.5), &[("h-a", 0.7), ("h-b", 0.8)]))],
        );
        let out = decide(&entries, &scene(), &resp, None);
        assert_eq!(tag_names(&out["a"]), vec!["brass"]);
        assert_eq!(out["a"].tags[0].score, 0.7);
        assert_eq!(tag_names(&out["b"]), vec!["brass"]);
    }

    #[test]
    fn decide_untrusted_or_low_tag_never_applied() {
        let entries = vec![entry("a", &[])];
        let resp = response(
            gate(true, Some(0.6), None, &[("h-a", 0.9)]),
            &[
                ("brass", set(false, Some(0.5), &[("h-a", 0.99)])),
                ("dark", set(true, Some(0.5), &[("h-a", 0.4)])),
            ],
        );
        let out = decide(&entries, &scene(), &resp, None);
        assert!(out["a"].tags.is_empty());
    }

    #[test]
    fn decide_scores_tags_whatever_the_user_voted() {
        let mut rejected_tag = entry("b", &[]);
        rejected_tag.rejected = vec!["brass".into()];
        let user_has_tag = entry("c", &["Brass"]);
        let hashes = [("h-b", 0.9), ("h-c", 0.8)];
        let resp = response(
            gate(true, Some(0.6), None, &hashes),
            &[("brass", set(true, Some(0.5), &hashes))],
        );
        let out = decide(&[rejected_tag, user_has_tag], &scene(), &resp, None);
        assert_eq!(out["b"].tags, vec![AutoTag { tag: "brass".into(), score: 0.9 }]);
        assert_eq!(out["c"].tags, vec![AutoTag { tag: "brass".into(), score: 0.8 }]);
    }

    #[test]
    fn decide_omits_unscored_entries() {
        let mut no_hash = entry("n", &[]);
        no_hash.hash = None;
        let mut not_music = entry("m", &[]);
        not_music.is_music = false;
        let entries = vec![entry("scored", &[]), entry("unscored", &[]), no_hash, not_music];
        let resp = response(gate(true, Some(0.6), None, &[("h-scored", 0.9), ("h-m", 0.9)]), &[]);
        let out = decide(&entries, &scene(), &resp, None);
        assert!(out.contains_key("scored"));
        assert!(!out.contains_key("unscored"));
        assert!(!out.contains_key("n"));
        assert!(!out.contains_key("m"));
    }

    #[test]
    fn score_response_parses_analyzer_json() {
        let json = r#"{"model":"m","gate":{"quality":{"auc":0.9,"pos":12,"neg":9},
            "trusted":true,"threshold":0.61,"neg_threshold":0.7,"scores":{}},
            "tags":{"brass":{"quality":{"auc":0.88,"pos":21,"neg":15},
            "trusted":true,"threshold":0.58,"neg_threshold":null,"scores":{"h":0.83}},
            "dark":{"quality":{"auc":null,"pos":1,"neg":0},
            "trusted":false,"threshold":null,"scores":{}}}}"#;
        let resp: ScoreResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.gate.neg_threshold, Some(0.7));
        assert_eq!(resp.tags["brass"].neg_threshold, None);
        assert_eq!(resp.tags["brass"].scores["h"], 0.83);
        // An older analyzer that sends no `neg_threshold` still parses.
        assert_eq!(resp.tags["dark"].neg_threshold, None);
        assert_eq!(resp.tags["dark"].quality.auc, None);
    }

    #[test]
    fn request_sends_no_pace() {
        let req = build_score_request(&[entry("a", &[])], &scene(), &HashSet::new());
        assert!(serde_json::to_value(&req).unwrap().get("pace").is_none());
    }

    /// Raw scores in `MoodId::ALL` order (nine moods, then energy).
    fn with_moods(raws: [f64; MoodId::COUNT]) -> MoodRaw {
        MoodRaw {
            moods: MoodId::ALL[..9].iter().zip(raws).map(|(m, v)| (m.as_db_str().to_string(), v)).collect(),
            energy: raws[9],
        }
    }

    /// A calibration with the same mean and sd for every id.
    fn flat_calibration(mean: f64, sd: f64) -> MoodCalibration {
        MoodCalibration(MoodId::ALL.iter().map(|m| (m.as_db_str().to_string(), MoodStat { mean, sd })).collect())
    }

    #[test]
    fn mood_score_is_a_logistic_of_one_and_a_half_z() {
        let stat = MoodStat { mean: 0.2, sd: 0.04 };
        // z = 0, 1, -2, 2.5.
        assert_eq!(mood_score(0.2, stat), 0.5);
        assert!((mood_score(0.24, stat) - 0.817_574_476_193_643_7).abs() < 1e-12);
        assert!((mood_score(0.12, stat) - 0.047_425_873_177_566_78).abs() < 1e-12);
        assert!((mood_score(0.30, stat) - 0.977_022_630_089_974_4).abs() < 1e-12);
        // Extremes stay inside [0, 1].
        assert_eq!(mood_score(10.0, stat), 1.0);
        assert!((0.0..1e-20).contains(&mood_score(-10.0, stat)));
        assert!(mood_score(f64::MAX, stat) <= 1.0 && mood_score(f64::MIN, stat) >= 0.0);
    }

    #[test]
    fn auto_moods_scores_every_id_against_its_own_calibration() {
        let mut cal = flat_calibration(0.1, 0.05);
        // Peaceful has the highest raw score but also the highest mean.
        cal.0.insert("peaceful".into(), MoodStat { mean: 0.3, sd: 0.05 });
        // Energy is a difference, so it has its own centre and spread.
        cal.0.insert("energy".into(), MoodStat { mean: -0.02, sd: 0.01 });
        let m = with_moods([0.3, 0.15, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.0, -0.04]);
        let out = auto_moods(&m, &cal).unwrap();
        assert_eq!(out.map(|(id, _)| id), MoodId::ALL);
        assert_eq!(out[0].1, 0.5); // peaceful: z 0
        assert!((out[1].1 - 0.817_574_476_193_643_7).abs() < 1e-12); // sorrowful: z 1
        assert!((out[8].1 - 0.047_425_873_177_566_78).abs() < 1e-12); // menacing: z -2
        assert!((out[9].1 - 0.047_425_873_177_566_78).abs() < 1e-12); // energy: z -2
    }

    #[test]
    fn no_auto_moods_without_every_score() {
        let cal = flat_calibration(0.0, 1.0);
        let mut m = with_moods([0.1; MoodId::COUNT]);
        m.moods.remove("wondrous");
        assert_eq!(auto_moods(&m, &cal), None);
        // A calibration missing an id gives none either.
        let mut partial = flat_calibration(0.0, 1.0);
        partial.0.remove("energy");
        assert_eq!(auto_moods(&with_moods([0.1; MoodId::COUNT]), &partial), None);
    }

    /// `n` entries whose id `i` scores `0.1 + i * 0.01 + (k % 2) * step`.
    fn library(n: usize, step: f64) -> HashMap<String, MoodRaw> {
        (0..n)
            .map(|k| {
                let bump = if k % 2 == 0 { 0.0 } else { step };
                let raws: [f64; MoodId::COUNT] = std::array::from_fn(|i| 0.1 + i as f64 * 0.01 + bump);
                (format!("h-{k}"), with_moods(raws))
            })
            .collect()
    }

    #[test]
    fn calibrate_needs_fifty_tracks_with_every_score() {
        assert_eq!(calibrate(&library(49, 0.02)), None);
        // Entries missing a mood do not count.
        let mut short = library(49, 0.02);
        let mut partial = with_moods([0.1; MoodId::COUNT]);
        partial.moods.remove("grim");
        short.insert("h-old".into(), partial);
        assert_eq!(calibrate(&short), None);

        let (cal, tracks) = calibrate(&library(50, 0.02)).unwrap();
        assert_eq!(tracks, 50);
        assert_eq!(cal.0.len(), MoodId::COUNT);
        // Half at base, half at base + 0.02: mean base + 0.01, population sd 0.01.
        for (i, id) in MoodId::ALL.iter().enumerate() {
            let stat = cal.0[id.as_db_str()];
            assert!((stat.mean - (0.11 + i as f64 * 0.01)).abs() < 1e-12);
            assert!((stat.sd - 0.01).abs() < 1e-12);
        }
    }

    #[test]
    fn calibrate_refuses_an_id_without_spread() {
        assert_eq!(calibrate(&library(60, 0.0)), None);
    }

    #[test]
    fn calibration_round_trips_as_flat_json() {
        let (cal, _) = calibrate(&library(50, 0.02)).unwrap();
        let text = serde_json::to_string(&cal).unwrap();
        // `{ id: {mean, sd}, ... }`, nothing else.
        assert!(text.starts_with(r#"{"cheerful":{"mean":"#));
        assert!(text.contains(r#""energy":{"mean":"#));
        let back: MoodCalibration = serde_json::from_str(&text).unwrap();
        assert_eq!(back.0.keys().collect::<Vec<_>>(), cal.0.keys().collect::<Vec<_>>());
        for (id, stat) in &cal.0 {
            assert!((back.0[id].mean - stat.mean).abs() < 1e-15 && (back.0[id].sd - stat.sd).abs() < 1e-15);
        }
    }

    #[test]
    fn decide_scores_moods_only_with_a_calibration() {
        let mut resp = response(gate(true, Some(0.6), None, &[("h-a", 0.9), ("h-b", 0.9)]), &[]);
        resp.mood = [("h-a".to_string(), with_moods([0.2; MoodId::COUNT]))].into();
        let entries = [entry("a", &[]), entry("b", &[])];
        let out = decide(&entries, &scene(), &resp, None);
        assert_eq!((out["a"].moods, out["b"].moods), (None, None));

        let cal = flat_calibration(0.2, 0.01);
        let out = decide(&entries, &scene(), &resp, Some(&cal));
        assert_eq!(out["a"].moods, auto_moods(&resp.mood["h-a"], &cal));
        assert!(out["a"].moods.unwrap().iter().all(|(_, v)| *v == 0.5));
        // No mood for the entry: no moods.
        assert_eq!(out["b"].moods, None);
    }

    #[test]
    fn moods_do_not_depend_on_role() {
        // Same mood scores, opposite gate scores (Combat vs Non-combat).
        let mut resp = response(gate(true, Some(0.6), Some(0.6), &[("h-a", 0.95), ("h-b", 0.05)]), &[]);
        let m = with_moods([0.25, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2]);
        resp.mood = [("h-a".to_string(), m.clone()), ("h-b".to_string(), m)].into();
        let cal = flat_calibration(0.2, 0.01);
        let out = decide(&[entry("a", &[]), entry("b", &[])], &scene(), &resp, Some(&cal));
        assert_ne!(out["a"].role.map(|r| r.0), out["b"].role.map(|r| r.0));
        assert_eq!(out["a"].moods, out["b"].moods);
    }

    #[test]
    fn old_analyzer_response_parses_without_pace_or_mood() {
        let json = r#"{"model":"m","gate":{"quality":{"auc":0.9,"pos":12,"neg":9},
            "trusted":true,"threshold":0.61,"scores":{}},"tags":{}}"#;
        let resp: ScoreResponse = serde_json::from_str(json).unwrap();
        assert!(resp.mood.is_empty());
    }

    #[test]
    fn mood_response_drops_entries_without_the_moods_shape() {
        let json = r#"{"model":"m","gate":{"quality":{"auc":0.9,"pos":12,"neg":9},
            "trusted":true,"threshold":0.61,"scores":{}},"tags":{},
            "pace":{"quality":{"auc":null,"pos":0,"neg":0},"trusted":false,"threshold":null,"scores":{}},
            "mood":{"h-new":{"moods":{"peaceful":0.1,"sorrowful":0.2,"wondrous":0.3,"cheerful":0.4,
                                      "quirky":0.5,"mysterious":0.6,"grim":0.7,"heroic":0.8,"menacing":0.9},
                             "energy":-0.05},
                    "h-pools":{"pools":{"peaceful":0.1,"cheerful":0.2,"quirky":0.3,"somber":0.4,"awe":0.5,
                                        "mysterious":0.6,"heroic":0.7,"menacing":0.8}},
                    "h-v14":{"bright":0.1,"light":0.2,"grand":0.3,"energy":0.4,"sad":0.25,"scary":0.15},
                    "h-old":{"bright":0.1,"light":0.2}}}"#;
        let resp: ScoreResponse = serde_json::from_str(json).unwrap();
        // Entries from analyzers that predate the mood profile (pools, the
        // scales) count as no mood.
        assert_eq!(resp.mood.len(), 1);
        assert_eq!(resp.mood["h-new"], with_moods([0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, -0.05]));
    }

    #[test]
    fn entry_scores_collects_gate_and_tags() {
        let resp = response(
            gate(true, Some(0.6), Some(0.7), &[("h", 0.9)]),
            &[
                ("dark", set(false, None, &[("h", 0.1)])),
                ("brass", set(true, Some(0.5), &[("h", 0.7)])),
            ],
        );
        let scores = entry_scores("h", &resp).unwrap();
        assert_eq!(scores.gate.tag, "combat");
        assert_eq!(scores.gate.score, 0.9);
        let names: Vec<&str> = scores.tags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(names, vec!["brass", "dark"]);
        assert!(scores.tags.iter().all(|t| t.applied.is_none()));
        assert!(entry_scores("other", &resp).is_none());
    }
}
