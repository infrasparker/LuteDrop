//! Pure planning for auto-tagging: which entries train the analyzer, and which
//! tags its scores earn. Nothing here touches the database or the network.

use crate::models::{AutoTag, EntryTagScores, SceneConfig, TagScore};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
}

/// Hashes of the entries that are examples for one tag (or for the gate).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Examples {
    pub positives: Vec<String>,
    pub negatives: Vec<String>,
}

/// Body of the analyzer's `POST /score`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreRequest {
    pub scene: String,
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

/// Tags are stored trimmed and lowercase, so compare them that way.
fn norm(tag: &str) -> String {
    tag.trim().to_lowercase()
}

fn has_tag(tags: &[String], wanted: &str) -> bool {
    tags.iter().any(|t| norm(t) == wanted)
}

/// Turns a local library path into the path the analyzer host sees.
///
/// The prefix match ignores ASCII case and treats `\` and `/` alike. Returns
/// `None` when `local_prefix` is empty or the path is not inside it.
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
    let remote = remote_prefix.trim_end_matches('/');
    Some(format!("{remote}/{rest}"))
}

/// Builds the `/score` request from the user's own tagging.
///
/// Auto tags never appear in `EntryFacts`, so the analyzer cannot train on its
/// own guesses. Only music entries with a hash can be examples.
pub fn build_score_request(
    entries: &[EntryFacts],
    scene: &SceneConfig,
    embedded: &HashSet<String>,
) -> ScoreRequest {
    let gate_tag = norm(&scene.gate_tag);
    let other_scenes: Vec<String> = scene.negative_scene_tags.iter().map(|t| norm(t)).collect();

    // Music entries with a hash, and whether the user gave each the gate tag.
    let hashed: Vec<(&EntryFacts, &String, bool)> = entries
        .iter()
        .filter(|e| e.is_music)
        .filter_map(|e| {
            let hash = e.hash.as_ref()?;
            Some((e, hash, has_tag(&e.user_tags, &gate_tag)))
        })
        .collect();

    let mut gate = Examples::default();
    for (entry, hash, has_gate) in &hashed {
        if *has_gate {
            gate.positives.push((*hash).clone());
        } else {
            let other_scene = other_scenes.iter().any(|t| has_tag(&entry.user_tags, t));
            if other_scene || has_tag(&entry.rejected, &gate_tag) {
                gate.negatives.push((*hash).clone());
            }
        }
    }

    let mut tags = HashMap::new();
    for vocab_tag in &scene.vocabulary {
        let vocab_tag = norm(vocab_tag);
        let mut examples = Examples::default();
        for (entry, hash, has_gate) in &hashed {
            if has_tag(&entry.user_tags, &vocab_tag) {
                // A tag on an entry outside the scene is not an example.
                if *has_gate {
                    examples.positives.push((*hash).clone());
                }
            } else if *has_gate || has_tag(&entry.rejected, &vocab_tag) {
                examples.negatives.push((*hash).clone());
            }
        }
        tags.insert(vocab_tag, examples);
    }

    let targets = hashed
        .iter()
        .filter(|(_, hash, _)| embedded.contains(*hash))
        .map(|(_, hash, _)| (*hash).clone())
        .collect();

    ScoreRequest {
        scene: scene.scene.clone(),
        gate,
        tags,
        targets,
    }
}

/// Decides which auto tags each scored entry should have.
///
/// Only entries the analyzer scored (music with a hash found in the gate's
/// scores) appear in the result; an empty list means "scored, and nothing
/// qualifies". Entries that were not scored are left out, so callers can tell
/// them apart from entries whose auto tags should be cleared.
pub fn decide(
    entries: &[EntryFacts],
    scene: &SceneConfig,
    resp: &ScoreResponse,
) -> HashMap<String, Vec<AutoTag>> {
    let gate_tag = norm(&scene.gate_tag);
    let other_scenes: Vec<String> = scene.negative_scene_tags.iter().map(|t| norm(t)).collect();

    let mut decisions = HashMap::new();
    for entry in entries.iter().filter(|e| e.is_music) {
        let Some(hash) = &entry.hash else { continue };
        let Some(&gate_score) = resp.gate.scores.get(hash) else {
            continue;
        };

        let user_gate = has_tag(&entry.user_tags, &gate_tag);
        let auto_gate = !user_gate
            && resp.gate.trusted
            && resp.gate.threshold.is_some_and(|t| gate_score >= t)
            && !has_tag(&entry.rejected, &gate_tag)
            && !other_scenes.iter().any(|t| has_tag(&entry.user_tags, t));

        let mut auto_tags = Vec::new();
        if auto_gate {
            auto_tags.push(AutoTag {
                tag: gate_tag.clone(),
                score: gate_score,
            });
        }

        // Vocabulary tags only go on entries that are in the scene.
        if user_gate || auto_gate {
            for vocab_tag in &scene.vocabulary {
                let vocab_tag = norm(vocab_tag);
                let Some(set) = resp.tags.get(&vocab_tag) else {
                    continue;
                };
                let Some(&score) = set.scores.get(hash) else {
                    continue;
                };
                let earned = set.trusted
                    && set.threshold.is_some_and(|t| score >= t)
                    && !has_tag(&entry.user_tags, &vocab_tag)
                    && !has_tag(&entry.rejected, &vocab_tag);
                if earned {
                    auto_tags.push(AutoTag {
                        tag: vocab_tag,
                        score,
                    });
                }
            }
        }

        decisions.insert(entry.id.clone(), auto_tags);
    }
    decisions
}

/// The gate score and every tag score the analyzer produced for one entry,
/// for showing in the UI. `applied` is left empty; it depends on the entry's
/// current tags and is filled in when the entry is loaded. The gate's
/// `tag` is `"gate"`.
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

    let gate = to_score("gate", &resp.gate)?;
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

    fn scene() -> SceneConfig {
        SceneConfig {
            scene: "encounter".into(),
            gate_tag: "encounter".into(),
            negative_scene_tags: vec!["tavern".into(), "mood".into()],
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
        }
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
            scores: scores.iter().map(|(h, s)| (h.to_string(), *s)).collect(),
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

    fn tag_names(tags: &[AutoTag]) -> Vec<&str> {
        tags.iter().map(|t| t.tag.as_str()).collect()
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
    fn map_path_empty_prefix_is_none() {
        assert_eq!(map_path("Z:/media/audio/a.opus", "", "/media/audio"), None);
    }

    #[test]
    fn request_excludes_auto_tags_from_training() {
        // `EntryFacts` holds user tags only: an entry whose `encounter` is
        // auto has none, so it is neither a gate positive nor a negative.
        let entries = vec![
            entry("a", &["encounter"]),
            entry("auto-only", &[]),
            entry("t", &["tavern"]),
        ];
        let req = build_score_request(&entries, &scene(), &HashSet::new());
        assert_eq!(req.gate.positives, vec!["h-a"]);
        assert_eq!(req.gate.negatives, vec!["h-t"]);
        assert!(!req.tags["brass"]
            .negatives
            .contains(&"h-auto-only".to_string()));
        assert!(!req.tags["brass"]
            .positives
            .contains(&"h-auto-only".to_string()));
    }

    #[test]
    fn request_builds_tag_examples_from_gate_entries() {
        let entries = vec![
            entry("a", &["encounter", "brass"]),
            entry("b", &["Encounter ", "dark"]),
            entry("t", &["tavern", "brass"]),
        ];
        let req = build_score_request(&entries, &scene(), &HashSet::new());
        assert_eq!(req.tags["brass"].positives, vec!["h-a"]);
        assert_eq!(req.tags["brass"].negatives, vec!["h-b"]);
        assert_eq!(req.tags["dark"].positives, vec!["h-b"]);
        assert_eq!(req.tags["dark"].negatives, vec!["h-a"]);
    }

    #[test]
    fn request_uses_rejections_as_negatives() {
        let mut rejected_gate = entry("r", &[]);
        rejected_gate.rejected = vec!["encounter".into()];
        let mut rejected_tag = entry("g", &["encounter"]);
        rejected_tag.rejected = vec!["brass".into()];
        let req = build_score_request(&[rejected_gate, rejected_tag], &scene(), &HashSet::new());
        assert_eq!(req.gate.negatives, vec!["h-r"]);
        assert_eq!(req.tags["brass"].negatives, vec!["h-g"]);
        assert!(req.tags["brass"].positives.is_empty());
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
        assert_eq!(req.targets, vec!["h-a"]);
    }

    #[test]
    fn decide_applies_gate_and_tags() {
        let entries = vec![entry("a", &[])];
        let resp = response(
            set(true, Some(0.6), &[("h-a", 0.9)]),
            &[("brass", set(true, Some(0.5), &[("h-a", 0.7)]))],
        );
        let out = decide(&entries, &scene(), &resp);
        assert_eq!(tag_names(&out["a"]), vec!["encounter", "brass"]);
    }

    #[test]
    fn decide_blocks_gate_on_user_scene_tag() {
        let entries = vec![entry("a", &["tavern"])];
        let resp = response(set(true, Some(0.6), &[("h-a", 0.99)]), &[]);
        let out = decide(&entries, &scene(), &resp);
        assert!(out["a"].is_empty());
    }

    #[test]
    fn decide_untrusted_tag_never_applied() {
        let entries = vec![entry("a", &["encounter"])];
        let resp = response(
            set(true, Some(0.6), &[("h-a", 0.9)]),
            &[("brass", set(false, Some(0.5), &[("h-a", 0.99)]))],
        );
        let out = decide(&entries, &scene(), &resp);
        assert!(out["a"].is_empty());
    }

    #[test]
    fn decide_encounter_tags_require_gate() {
        let entries = vec![entry("a", &[])];
        // Gate score is below its threshold, so no encounter and no tags.
        let resp = response(
            set(true, Some(0.6), &[("h-a", 0.2)]),
            &[("brass", set(true, Some(0.5), &[("h-a", 0.99)]))],
        );
        assert!(decide(&entries, &scene(), &resp)["a"].is_empty());

        // An untrusted gate never applies encounter either.
        let resp = response(
            set(false, Some(0.6), &[("h-a", 0.9)]),
            &[("brass", set(true, Some(0.5), &[("h-a", 0.99)]))],
        );
        assert!(decide(&entries, &scene(), &resp)["a"].is_empty());
    }

    #[test]
    fn decide_user_gate_tag_gets_tags_but_no_auto_gate() {
        let entries = vec![entry("a", &["encounter"])];
        let resp = response(
            set(false, None, &[("h-a", 0.1)]),
            &[("brass", set(true, Some(0.5), &[("h-a", 0.7)]))],
        );
        let out = decide(&entries, &scene(), &resp);
        assert_eq!(tag_names(&out["a"]), vec!["brass"]);
    }

    #[test]
    fn decide_respects_rejection() {
        let mut rejected_gate = entry("a", &[]);
        rejected_gate.rejected = vec!["encounter".into()];
        let mut rejected_tag = entry("b", &["encounter"]);
        rejected_tag.rejected = vec!["brass".into()];
        let user_has_tag = entry("c", &["encounter", "Brass"]);
        let hashes = [("h-a", 0.9), ("h-b", 0.9), ("h-c", 0.9)];
        let resp = response(
            set(true, Some(0.6), &hashes),
            &[("brass", set(true, Some(0.5), &hashes))],
        );
        let out = decide(
            &[rejected_gate, rejected_tag, user_has_tag],
            &scene(),
            &resp,
        );
        assert!(out["a"].is_empty());
        assert!(out["b"].is_empty());
        assert!(out["c"].is_empty());
    }

    #[test]
    fn decide_omits_unscored_entries() {
        let mut no_hash = entry("n", &[]);
        no_hash.hash = None;
        let entries = vec![entry("scored", &[]), entry("unscored", &[]), no_hash];
        let resp = response(set(true, Some(0.6), &[("h-scored", 0.9)]), &[]);
        let out = decide(&entries, &scene(), &resp);
        assert!(out.contains_key("scored"));
        assert!(!out.contains_key("unscored"));
        assert!(!out.contains_key("n"));
    }

    #[test]
    fn score_response_parses_analyzer_json() {
        let json = r#"{"model":"m","gate":{"quality":{"auc":null,"pos":1,"neg":0},
            "trusted":false,"threshold":null,"scores":{}},
            "tags":{"brass":{"quality":{"auc":0.88,"pos":21,"neg":15},
            "trusted":true,"threshold":0.58,"scores":{"h":0.83}}}}"#;
        let resp: ScoreResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.gate.quality.auc, None);
        assert_eq!(resp.tags["brass"].scores["h"], 0.83);
    }

    #[test]
    fn entry_scores_collects_gate_and_tags() {
        let resp = response(
            set(true, Some(0.6), &[("h", 0.9)]),
            &[
                ("dark", set(false, None, &[("h", 0.1)])),
                ("brass", set(true, Some(0.5), &[("h", 0.7)])),
            ],
        );
        let scores = entry_scores("h", &resp).unwrap();
        assert_eq!(scores.gate.score, 0.9);
        let names: Vec<&str> = scores.tags.iter().map(|t| t.tag.as_str()).collect();
        assert_eq!(names, vec!["brass", "dark"]);
        assert!(scores.tags.iter().all(|t| t.applied.is_none()));
        assert!(entry_scores("other", &resp).is_none());
    }
}
