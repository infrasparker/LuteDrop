//! Name-matching rules for the "Fix missing files" dialog.
//!
//! When a library entry's audio file is gone, we look through a folder for
//! files whose *names* look like the missing one. Everything here is pure
//! (no database, no disk reads besides what the caller already listed), so it
//! is easy to test. The rules, and why they are what they are:
//!
//! * **Extensions are ignored.** A track is often re-encoded (`song.mp3` ->
//!   `song.opus`), so the file type says nothing about whether it is the same
//!   song.
//! * **Apostrophes are deleted, not turned into spaces.** "Don't" should
//!   become "dont" (one word), the way people name files, not "don t".
//!   Every other run of punctuation/whitespace becomes a single space, so
//!   `03_dont-let` and `03 - Dont let` end up identical.
//! * **Token overlap instead of edit distance.** File names get track numbers
//!   added, words reordered or extra words appended. Comparing the *sets of
//!   words* (Jaccard overlap) copes with that, while edit distance would
//!   punish a long prefix like "01 " far more than a missing word.
//!
//! Matches come in three tiers, best first: Exact, Contains (one name sits
//! inside the other and the shorter one is at least `MIN_CONTAINS_LEN`
//! characters, so tiny names like "ab" do not match everything), and Similar
//! (word overlap of at least `MIN_SIMILARITY`).

// Nothing calls this module yet (the Tauri commands that use it come in a
// later step), so silence "never used" warnings until then.
#![allow(dead_code)]

use crate::models::{MatchKind, RelinkCandidate};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// Most candidates returned per missing entry, so the dialog stays readable.
pub const MAX_CANDIDATES: usize = 20;
/// A "contains" match only counts when the shorter name has at least this many characters.
pub const MIN_CONTAINS_LEN: usize = 4;
/// Minimum word-overlap (0.0 to 1.0) for a "similar" match.
pub const MIN_SIMILARITY: f64 = 0.5;

/// Turns a file name (without extension) into a comparable form: lowercase,
/// apostrophes (`'` and `’`) deleted, and every other run of non-letter,
/// non-digit characters collapsed into a single space.
pub fn normalize_stem(text: &str) -> String {
    let mut out = String::new();
    // True when a separator is waiting to be written before the next word.
    let mut pending_space = false;
    for ch in text.chars() {
        if ch == '\'' || ch == '\u{2019}' {
            continue;
        }
        if ch.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.extend(ch.to_lowercase());
        } else {
            pending_space = true;
        }
    }
    out
}

/// Returns the last path segment (split on either `\` or `/`) without its
/// final extension. Done by plain string splitting so Windows paths work no
/// matter which OS runs the code.
pub fn file_stem_of(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(idx) if idx > 0 => name[..idx].to_string(),
        _ => name.to_string(),
    }
}

/// A file found under the search folder, pre-processed for matching.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub file_path: String,
    pub relative_path: String,
    pub norm_stem: String,
}

/// Prepares a list of file paths for matching: computes each file's path
/// relative to `root` (falls back to the full path if it is outside `root`)
/// and its normalized name.
pub fn index_files(root: &Path, file_paths: &[String]) -> Vec<ScannedFile> {
    file_paths
        .iter()
        .map(|file_path| {
            let relative_path = Path::new(file_path)
                .strip_prefix(root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| file_path.clone());
            ScannedFile {
                file_path: file_path.clone(),
                relative_path,
                norm_stem: normalize_stem(&file_stem_of(file_path)),
            }
        })
        .collect()
}

/// The normalized names to search for on behalf of one missing entry: its old
/// file's name and its display name. Duplicates and empty results are dropped.
pub fn needles_for(file_path: &str, name: &str) -> Vec<String> {
    let mut needles: Vec<String> = Vec::new();
    for raw in [file_stem_of(file_path), name.to_string()] {
        let needle = normalize_stem(&raw);
        if !needle.is_empty() && !needles.contains(&needle) {
            needles.push(needle);
        }
    }
    needles
}

/// Word-overlap score: shared words divided by total distinct words.
fn jaccard(a: &str, b: &str) -> f64 {
    let a_words: HashSet<&str> = a.split_whitespace().collect();
    let b_words: HashSet<&str> = b.split_whitespace().collect();
    let union = a_words.union(&b_words).count();
    if union == 0 {
        return 0.0;
    }
    a_words.intersection(&b_words).count() as f64 / union as f64
}

/// How one needle matches one file name, if at all.
fn classify(needle: &str, stem: &str) -> Option<(MatchKind, f64)> {
    if needle == stem {
        return Some((MatchKind::Exact, 1.0));
    }
    let (needle_len, stem_len) = (needle.chars().count(), stem.chars().count());
    let shorter = needle_len.min(stem_len);
    if shorter >= MIN_CONTAINS_LEN && (stem.contains(needle) || needle.contains(stem)) {
        // The closer the two lengths, the better the match.
        let longer = needle_len.max(stem_len);
        return Some((MatchKind::Contains, shorter as f64 / longer as f64));
    }
    let score = jaccard(needle, stem);
    if score >= MIN_SIMILARITY {
        return Some((MatchKind::Similar, score));
    }
    None
}

/// Position of a kind in the ranking (lower is better).
fn tier(kind: MatchKind) -> u8 {
    match kind {
        MatchKind::Exact => 0,
        MatchKind::Contains => 1,
        MatchKind::Similar => 2,
    }
}

/// Finds the files that look like a missing entry. Each file keeps its best
/// match across all `needles`. Results are ordered Exact, then Contains, then
/// Similar; inside a tier by higher score, then by relative path; and cut to
/// `MAX_CANDIDATES`. `used` maps a file path to the name of the library entry
/// already using it, so the dialog can flag (not hide) such files.
pub fn match_candidates(
    needles: &[String],
    files: &[ScannedFile],
    used: &HashMap<String, String>,
) -> Vec<RelinkCandidate> {
    let mut out: Vec<RelinkCandidate> = Vec::new();
    for file in files {
        let mut best: Option<(MatchKind, f64)> = None;
        for needle in needles {
            if let Some((kind, score)) = classify(needle, &file.norm_stem) {
                let better = match best {
                    None => true,
                    Some((best_kind, best_score)) => {
                        tier(kind) < tier(best_kind)
                            || (tier(kind) == tier(best_kind) && score > best_score)
                    }
                };
                if better {
                    best = Some((kind, score));
                }
            }
        }
        if let Some((kind, score)) = best {
            out.push(RelinkCandidate {
                file_path: file.file_path.clone(),
                relative_path: file.relative_path.clone(),
                kind,
                score,
                used_by_entry_name: used.get(&file.file_path).cloned(),
            });
        }
    }
    out.sort_by(|a, b| {
        tier(a.kind)
            .cmp(&tier(b.kind))
            .then(b.score.total_cmp(&a.score))
            .then(a.relative_path.cmp(&b.relative_path))
    });
    out.truncate(MAX_CANDIDATES);
    out
}

/// Guesses a search folder: the deepest folder that contains all of `paths`
/// (the common prefix of their parent folders). Returns `None` when there is
/// nothing shared beyond a drive or filesystem root, e.g. for an empty list or
/// paths on different drives.
pub fn common_root(paths: &[String]) -> Option<PathBuf> {
    let mut shared: Option<Vec<Component>> = None;
    for path in paths {
        let parent = Path::new(path).parent()?;
        let components: Vec<Component> = parent.components().collect();
        shared = Some(match shared {
            None => components,
            Some(prev) => prev
                .into_iter()
                .zip(components)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a)
                .collect(),
        });
    }
    let shared = shared?;
    let has_real_folder = shared
        .iter()
        .any(|c| !matches!(c, Component::Prefix(_) | Component::RootDir));
    if !has_real_folder {
        return None;
    }
    Some(shared.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MatchKind;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// Builds a ScannedFile for tests from a relative path like "a/b.opus".
    fn scanned(rel: &str) -> ScannedFile {
        // Join piece by piece so the separator matches the OS.
        let mut full = PathBuf::from("root");
        full.extend(rel.split('/'));
        let full = full.to_string_lossy().into_owned();
        index_files(Path::new("root"), &[full]).into_iter().next().unwrap()
    }

    #[test]
    fn normalize_collapses_punctuation_and_case() {
        assert_eq!(normalize_stem("03 - Don't Let Your Guard Down"), "03 dont let your guard down");
        assert_eq!(normalize_stem("03_dont_let_your_guard_down"), "03 dont let your guard down");
        assert_eq!(normalize_stem("  Hello---World  "), "hello world");
        assert_eq!(normalize_stem("Don\u{2019}t"), "dont");
    }

    #[test]
    fn stem_handles_slashes_and_dots() {
        assert_eq!(file_stem_of("/old/dir/song"), "song");
        assert_eq!(file_stem_of(r"C:\x\song.final.opus"), "song.final");
        assert_eq!(file_stem_of("track.mp3"), "track");
    }

    #[test]
    fn needles_dedup_and_drop_empty() {
        assert_eq!(needles_for(r"C:\a\Song.mp3", "song"), vec!["song"]);
        assert_eq!(needles_for(r"C:\a\Song.mp3", "Battle Theme"), vec!["song", "battle theme"]);
        assert_eq!(needles_for(r"C:\a\---.mp3", "---"), Vec::<String>::new());
    }

    #[test]
    fn exact_beats_contains_beats_similar() {
        let files = vec![
            scanned("theme of battle.opus"),
            scanned("01 battle theme.opus"),
            scanned("battle theme.opus"),
        ];
        let out = match_candidates(&["battle theme".to_string()], &files, &HashMap::new());
        let kinds: Vec<MatchKind> = out.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, vec![MatchKind::Exact, MatchKind::Contains, MatchKind::Similar]);
        assert_eq!(out[0].score, 1.0);
        assert!((out[1].score - 0.8).abs() < 1e-9);
        assert!((out[2].score - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn contains_requires_four_chars() {
        let files = vec![scanned("ab cd.opus")];
        let out = match_candidates(&["ab".to_string()], &files, &HashMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MatchKind::Similar);
        assert!((out[0].score - 0.5).abs() < 1e-9);
    }

    #[test]
    fn similar_requires_half_overlap() {
        let files = vec![scanned("blue red.opus"), scanned("red yellow purple.opus")];
        let out = match_candidates(&["red blue green".to_string()], &files, &HashMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MatchKind::Similar);
        assert!((out[0].score - 2.0 / 3.0).abs() < 1e-9);
        assert!(out[0].file_path.ends_with("blue red.opus"));
    }

    #[test]
    fn extension_is_ignored() {
        let needles = needles_for(r"C:\old\song.mp3", "");
        let files = vec![scanned("song.opus")];
        let out = match_candidates(&needles, &files, &HashMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MatchKind::Exact);
    }

    #[test]
    fn in_use_candidates_are_flagged_not_hidden() {
        let files = vec![scanned("song.opus"), scanned("other/song.opus")];
        let mut used = HashMap::new();
        used.insert(files[0].file_path.clone(), "Other".to_string());
        let out = match_candidates(&["song".to_string()], &files, &used);
        assert_eq!(out.len(), 2);
        let flagged = out.iter().find(|c| c.file_path == files[0].file_path).unwrap();
        assert_eq!(flagged.used_by_entry_name, Some("Other".to_string()));
        let free = out.iter().find(|c| c.file_path == files[1].file_path).unwrap();
        assert_eq!(free.used_by_entry_name, None);
    }

    #[test]
    fn capped_at_twenty_sorted_by_relative_path_within_tier() {
        // Built in reverse so sorting (not input order) decides the result.
        let files: Vec<ScannedFile> = (0..25)
            .rev()
            .map(|i| scanned(&format!("d{i:02}/song.opus")))
            .collect();
        let out = match_candidates(&["song".to_string()], &files, &HashMap::new());
        assert_eq!(out.len(), MAX_CANDIDATES);
        let first = Path::new("d00").join("song.opus").to_string_lossy().into_owned();
        assert_eq!(out[0].relative_path, first);
    }

    #[test]
    fn two_needles_keep_best_match_per_file() {
        let files = vec![scanned("battle theme.opus")];
        // "theme battle" is only Similar (word overlap); "battle theme" is Exact.
        let needles = vec!["theme battle x".to_string(), "battle theme".to_string()];
        let out = match_candidates(&needles, &files, &HashMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MatchKind::Exact);
    }

    #[test]
    fn empty_needles_yield_no_candidates() {
        let files = vec![scanned("song.opus")];
        assert!(match_candidates(&[], &files, &HashMap::new()).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn common_root_of_sibling_albums() {
        let r = common_root(&[
            r"Z:\media\audio\music\A\x\1.opus".into(),
            r"Z:\media\audio\music\B\2.opus".into(),
        ])
        .unwrap();
        assert_eq!(r, PathBuf::from(r"Z:\media\audio\music"));
    }

    #[cfg(windows)]
    #[test]
    fn common_root_none_for_empty_or_different_drives() {
        assert!(common_root(&[]).is_none());
        assert!(common_root(&[r"C:\a\1.mp3".into(), r"Z:\b\2.mp3".into()]).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn common_root_single_path_is_its_parent() {
        let r = common_root(&[r"Z:\media\a\1.opus".into()]).unwrap();
        assert_eq!(r, PathBuf::from(r"Z:\media\a"));
    }

    #[cfg(windows)]
    #[test]
    fn index_files_relative_paths() {
        let files = index_files(
            Path::new(r"Z:\m"),
            &[r"Z:\m\A\x.opus".to_string(), r"Y:\other\y.opus".to_string()],
        );
        assert_eq!(files[0].relative_path, r"A\x.opus");
        assert_eq!(files[0].norm_stem, "x");
        assert_eq!(files[1].relative_path, r"Y:\other\y.opus");
    }
}
