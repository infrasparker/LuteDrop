//! Pure planning for auto-tagging: which entries train the analyzer, and which
//! role and tags its scores earn. Nothing here touches the database or the
//! network.

use crate::models::{AutoTag, EntryTagScores, Role, SceneConfig, TagScore};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Name the gate's scores and quality carry in saved scores and reports.
pub const GATE_NAME: &str = "combat";

/// What the planner needs to know about one library entry.
#[derive(Debug, Clone)]
pub struct EntryFacts {
    pub id: String,
    /// SHA-256 of the file; `None` until the entry has been hashed.
    pub hash: Option<String>,
    /// Tags the user put on the entry (auto tags are not included).
    pub user_tags: Vec<String>,
    /// Tags the user removed from the entry after the analyzer suggested them.
    pub rejected: Vec<String>,
    pub is_music: bool,
    /// The role the user set. `None` when the role is unset, auto, or cleared
    /// by the user.
    pub user_role: Option<Role>,
    /// True when the user decided the role (`role_source` is user), including
    /// a role the user cleared.
    pub role_user_set: bool,
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
}

/// What one scored entry should carry after a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// The auto role and the Combat probability it is stored with. `None`
    /// clears an auto role; the writer never changes a role the user decided.
    pub role: Option<(Role, f64)>,
    /// The auto tags the entry should have; the rest of its auto tags go.
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

/// Builds the `/score` request from the user's own roles and tags.
///
/// Only music entries with a hash take part. The gate's positives are entries
/// the user set to Combat and its negatives those set to Non-combat. For each
/// vocabulary tag, positives are entries with the user tag; negatives are
/// entries with a user role but without the tag, plus entries where the user
/// rejected the tag. Auto roles and auto tags never appear in `EntryFacts`,
/// so the analyzer cannot train on its own guesses. Targets are every music
/// entry whose hash is in `embedded`.
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

/// Decides the auto role and auto tags of each scored entry.
///
/// Only entries the analyzer scored (music with a hash found in the gate's
/// scores) appear in the result, so callers can tell them apart from entries
/// that were not scored. An entry whose role the user decided gets no role.
/// A tag is earned when its classifier is trusted, the score reaches its
/// threshold, and the user has neither put the tag on the entry nor rejected
/// it.
pub fn decide(
    entries: &[EntryFacts],
    scene: &SceneConfig,
    resp: &ScoreResponse,
) -> HashMap<String, Decision> {
    let vocabulary = vocabulary(scene);

    let mut decisions = HashMap::new();
    for entry in entries.iter().filter(|e| e.is_music) {
        let Some(hash) = &entry.hash else { continue };
        let Some(&gate_score) = resp.gate.scores.get(hash) else {
            continue;
        };

        let role = if entry.role_user_set {
            None
        } else {
            role_for(&resp.gate, gate_score)
        };

        let mut tags = Vec::new();
        for vocab_tag in &vocabulary {
            let Some(set) = resp.tags.get(vocab_tag) else {
                continue;
            };
            let Some(&score) = set.scores.get(hash) else {
                continue;
            };
            let earned = set.trusted
                && set.threshold.is_some_and(|t| score >= t)
                && !has_tag(&entry.user_tags, vocab_tag)
                && !has_tag(&entry.rejected, vocab_tag);
            if earned {
                tags.push(AutoTag {
                    tag: vocab_tag.clone(),
                    score,
                });
            }
        }

        decisions.insert(entry.id.clone(), Decision { role, tags });
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
            role_user_set: false,
        }
    }

    /// The entry with a role the user set (`None` = the user cleared it).
    fn with_role(mut e: EntryFacts, role: Option<Role>) -> EntryFacts {
        e.user_role = role;
        e.role_user_set = true;
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
        let out = decide(&entries, &cfg, &resp);
        assert_eq!(tag_names(&out["a"]), vec!["brass"]);

        let req = build_score_request(&entries, &cfg, &HashSet::new());
        assert_eq!(req.tags.keys().collect::<Vec<_>>(), vec!["brass"]);
    }

    #[test]
    fn request_excludes_auto_tags_from_training() {
        // `EntryFacts` holds user tags and user roles only: an entry whose
        // tags and role are all auto has none, so it is no example at all.
        let entries = vec![
            with_role(entry("a", &["brass"]), Some(Role::Combat)),
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
        let mut not_music = with_role(entry("n", &[]), Some(Role::Combat));
        not_music.is_music = false;
        let entries = vec![
            with_role(entry("a", &[]), Some(Role::Combat)),
            with_role(entry("b", &[]), Some(Role::NonCombat)),
            with_role(entry("cleared", &[]), None),
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
        let mut not_music = with_role(entry("n", &[]), Some(Role::NonCombat));
        not_music.is_music = false;
        let entries = vec![
            with_role(entry("a", &["brass"]), Some(Role::Combat)),
            with_role(entry("b", &["Dark "]), Some(Role::NonCombat)),
            with_role(entry("cleared", &[]), None),
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

        let out = decide(&entries, &scene(), &response(gate(true, Some(0.6), Some(0.7), &scores), &[]));
        assert_eq!(out["a"].role, Some((Role::Combat, 0.9)));
        // 1 - 0.2 = 0.8 clears the Non-combat threshold; the score kept is p.
        assert_eq!(out["b"].role, Some((Role::NonCombat, 0.2)));
        // 0.4 is below 0.6, and 0.6 is below 0.7.
        assert_eq!(out["c"].role, None);

        // Without a Non-combat threshold only Combat can be set.
        let out = decide(&entries, &scene(), &response(gate(true, Some(0.6), None, &scores), &[]));
        assert_eq!(out["a"].role, Some((Role::Combat, 0.9)));
        assert_eq!(out["b"].role, None);

        // An untrusted gate sets no role.
        let out = decide(&entries, &scene(), &response(gate(false, Some(0.6), Some(0.7), &scores), &[]));
        assert!(out.values().all(|d| d.role.is_none()));
    }

    #[test]
    fn decide_skips_user_set_role() {
        let entries = vec![
            with_role(entry("a", &[]), Some(Role::NonCombat)),
            with_role(entry("cleared", &[]), None),
        ];
        let resp = response(
            gate(true, Some(0.6), Some(0.6), &[("h-a", 0.95), ("h-cleared", 0.95)]),
            &[],
        );
        let out = decide(&entries, &scene(), &resp);
        assert_eq!(out["a"].role, None);
        assert_eq!(out["cleared"].role, None);
    }

    #[test]
    fn decide_tags_need_no_gate() {
        let entries = vec![entry("a", &[]), with_role(entry("b", &[]), Some(Role::NonCombat))];
        // The gate is untrusted and scores low; tags are decided on their own.
        let resp = response(
            gate(false, None, None, &[("h-a", 0.1), ("h-b", 0.1)]),
            &[("brass", set(true, Some(0.5), &[("h-a", 0.7), ("h-b", 0.8)]))],
        );
        let out = decide(&entries, &scene(), &resp);
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
        let out = decide(&entries, &scene(), &resp);
        assert!(out["a"].tags.is_empty());
    }

    #[test]
    fn decide_respects_rejection_and_user_tags() {
        let mut rejected_tag = entry("b", &[]);
        rejected_tag.rejected = vec!["brass".into()];
        let user_has_tag = entry("c", &["Brass"]);
        let hashes = [("h-b", 0.9), ("h-c", 0.9)];
        let resp = response(
            gate(true, Some(0.6), None, &hashes),
            &[("brass", set(true, Some(0.5), &hashes))],
        );
        let out = decide(&[rejected_tag, user_has_tag], &scene(), &resp);
        assert!(out["b"].tags.is_empty());
        assert!(out["c"].tags.is_empty());
    }

    #[test]
    fn decide_omits_unscored_entries() {
        let mut no_hash = entry("n", &[]);
        no_hash.hash = None;
        let mut not_music = entry("m", &[]);
        not_music.is_music = false;
        let entries = vec![entry("scored", &[]), entry("unscored", &[]), no_hash, not_music];
        let resp = response(gate(true, Some(0.6), None, &[("h-scored", 0.9), ("h-m", 0.9)]), &[]);
        let out = decide(&entries, &scene(), &resp);
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
