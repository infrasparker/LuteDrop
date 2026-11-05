//! Vibe browsing: similar tracks, text search and the map, built on the
//! analyzer's `/similar`, `/search` and `/map`.
//!
//! The analyzer knows files by content hash, the app knows entries by id.
//! Two entries can share a hash (the same file imported twice), so each hash
//! the analyzer returns expands to every music entry that has it, all with
//! the same score or position. Only music entries are ever sent or shown.
//!
//! The database lock is held only to read the entries, never during a call
//! to the analyzer.

use std::collections::HashMap;
use std::sync::Mutex;

use rusqlite::Connection;

use super::client::{AnalyzerClient, HashScore, HttpAnalyzer};
use crate::database::settings;
use crate::models::{VibeHit, VibePoint};

/// Largest result count the analyzer returns.
const MAX_LIMIT: u32 = 500;

/// Shown when auto-tagging is off or has no analyzer URL.
const NOT_CONFIGURED: &str = "Auto-tagging analyzer is not configured";

/// The music entries that have a content hash.
struct MusicIndex {
    /// Each distinct hash once, in entry-name order.
    hashes: Vec<String>,
    /// Entry ids per hash, in entry-name order.
    entries_by_hash: HashMap<String, Vec<String>>,
    /// Hash per entry id.
    hash_by_entry: HashMap<String, String>,
}

impl MusicIndex {
    /// Reads the music entries that have a hash. The lock is released before
    /// this returns.
    fn load(db: &Mutex<Connection>) -> Result<Self, String> {
        let rows: Vec<(String, String)> = {
            let conn = db.lock().map_err(|e| format!("Database lock poisoned: {e}"))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, content_hash FROM library_entries
                     WHERE entry_type = 'music' AND content_hash IS NOT NULL
                     ORDER BY name, id",
                )
                .map_err(|e| format!("Failed to prepare music query: {e}"))?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| format!("Failed to query music entries: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to read music entries: {e}"))?;
            rows
        };

        let mut index = MusicIndex {
            hashes: Vec::new(),
            entries_by_hash: HashMap::new(),
            hash_by_entry: HashMap::new(),
        };
        for (id, hash) in rows {
            let ids = index.entries_by_hash.entry(hash.clone()).or_default();
            if ids.is_empty() {
                index.hashes.push(hash.clone());
            }
            ids.push(id.clone());
            index.hash_by_entry.insert(id, hash);
        }
        Ok(index)
    }

    /// The entry ids that have this hash, or none if it is not a music hash.
    fn entries(&self, hash: &str) -> &[String] {
        self.entries_by_hash.get(hash).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Turns the analyzer's ranked hashes into hits, one per entry.
    fn hits(&self, ranked: Vec<HashScore>) -> Vec<VibeHit> {
        ranked
            .into_iter()
            .flat_map(|r| {
                self.entries(&r.hash)
                    .iter()
                    .map(move |id| VibeHit { entry_id: id.clone(), score: r.score })
            })
            .collect()
    }
}

/// The analyzer accepts 1 to 500 results.
fn clamp_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_LIMIT)
}

/// An analyzer client for the saved settings, or an error if auto-tagging is
/// off or has no URL.
pub fn client_for(db: &Mutex<Connection>) -> Result<HttpAnalyzer, String> {
    let settings = {
        let conn = db.lock().map_err(|e| format!("Database lock poisoned: {e}"))?;
        settings::load_autotag(&conn)?
    };
    if !settings.enabled || settings.analyzer_url.trim().is_empty() {
        return Err(NOT_CONFIGURED.to_string());
    }
    Ok(HttpAnalyzer::new(&settings.analyzer_url, &settings.token))
}

/// Entries most like the selected ones (their centroid), best first. The
/// selected entries are left out. Selected entries that are not music or have
/// no hash are ignored; if none remain, that is an error.
pub fn similar(
    db: &Mutex<Connection>,
    client: &dyn AnalyzerClient,
    entry_ids: &[String],
    limit: u32,
) -> Result<Vec<VibeHit>, String> {
    let index = MusicIndex::load(db)?;
    let mut query: Vec<String> = Vec::new();
    for id in entry_ids {
        if let Some(hash) = index.hash_by_entry.get(id) {
            if !query.contains(hash) {
                query.push(hash.clone());
            }
        }
    }
    if query.is_empty() {
        return Err("None of the selected tracks have been analyzed yet".to_string());
    }
    let ranked = client.similar(&query, &index.hashes, clamp_limit(limit))?;
    Ok(index.hits(ranked))
}

/// Music entries ranked by how well they match `text`, best first. Blank text
/// gives no results.
pub fn search(
    db: &Mutex<Connection>,
    client: &dyn AnalyzerClient,
    text: &str,
    limit: u32,
) -> Result<Vec<VibeHit>, String> {
    let text = text.trim();
    let index = MusicIndex::load(db)?;
    if text.is_empty() || index.hashes.is_empty() {
        return Ok(Vec::new());
    }
    let ranked = client.search(text, &index.hashes, clamp_limit(limit))?;
    Ok(index.hits(ranked))
}

/// A 2D position for every analyzed music entry.
pub fn map(db: &Mutex<Connection>, client: &dyn AnalyzerClient) -> Result<Vec<VibePoint>, String> {
    let index = MusicIndex::load(db)?;
    if index.hashes.is_empty() {
        return Ok(Vec::new());
    }
    let points = client.map(&index.hashes)?;
    Ok(points
        .into_iter()
        .flat_map(|p| {
            index
                .entries(&p.hash)
                .iter()
                .map(move |id| VibePoint { entry_id: id.clone(), x: p.x, y: p.y })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotag::client::{HashPoint, HashScore};
    use crate::autotag::tests::{add, setup, FakeAnalyzer};
    use crate::database::library;
    use crate::models::AutoTagSettings;

    fn ranked(items: &[(&str, f64)]) -> Vec<HashScore> {
        items.iter().map(|(h, s)| HashScore { hash: h.to_string(), score: *s }).collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn similar_maps_hashes_back_to_entries() {
        let db = setup();
        let seed = add(&db, "seed", "music");
        let near = add(&db, "near", "music");
        let far = add(&db, "far", "music");
        let fake = FakeAnalyzer { ranked: ranked(&[("h-near", 0.9), ("h-far", 0.4)]), ..Default::default() };

        let hits = similar(&db, &fake, &[seed], 10).unwrap();

        assert_eq!(
            hits,
            vec![
                VibeHit { entry_id: near, score: 0.9 },
                VibeHit { entry_id: far, score: 0.4 },
            ]
        );
        let (query, candidates, limit) = fake.last_call.lock().unwrap().clone().unwrap();
        assert_eq!(query, strings(&["h-seed"]));
        assert_eq!(limit, 10);
        // Candidates are every analyzed music hash.
        let mut candidates = candidates;
        candidates.sort();
        assert_eq!(candidates, strings(&["h-far", "h-near", "h-seed"]));
    }

    #[test]
    fn similar_without_any_hashed_seed_is_an_error() {
        let db = setup();
        let sfx = add(&db, "boom", "sfx");
        add(&db, "other", "music");
        let fake = FakeAnalyzer::default();

        let err = similar(&db, &fake, &[sfx, "missing".to_string()], 10).unwrap_err();

        assert_eq!(err, "None of the selected tracks have been analyzed yet");
        assert!(fake.last_call.lock().unwrap().is_none());
    }

    #[test]
    fn shared_hash_returns_all_entries() {
        let db = setup();
        let seed = add(&db, "seed", "music");
        // Two entries holding the same file.
        let twin_a = add(&db, "twin-a", "music");
        let twin_b = {
            let conn = db.lock().unwrap();
            library::insert_library_entry(&conn, "Z:\\media\\audio\\twin-b.opus", Some("h-twin-a"), "twin-b", None, None, None, "music")
                .unwrap()
        };
        let fake = FakeAnalyzer {
            ranked: ranked(&[("h-twin-a", 0.8)]),
            points: vec![HashPoint { hash: "h-twin-a".into(), x: 0.25, y: 0.75 }],
            ..Default::default()
        };

        let hits = similar(&db, &fake, &[seed], 10).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.entry_id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&twin_a.as_str()) && ids.contains(&twin_b.as_str()));
        assert!(hits.iter().all(|h| h.score == 0.8));

        let points = map(&db, &fake).unwrap();
        assert_eq!(points.len(), 2);
        assert!(points.iter().all(|p| p.x == 0.25 && p.y == 0.75));

        let (_, candidates, _) = {
            search(&db, &fake, "x", 5).unwrap();
            fake.last_call.lock().unwrap().clone().unwrap()
        };
        // The shared hash is sent once.
        assert_eq!(candidates.iter().filter(|h| *h == "h-twin-a").count(), 1);
    }

    #[test]
    fn search_passes_text_and_candidates() {
        let db = setup();
        let a = add(&db, "a", "music");
        add(&db, "b", "music");
        add(&db, "fx", "sfx");
        let fake = FakeAnalyzer { ranked: ranked(&[("h-a", 0.7)]), ..Default::default() };

        let hits = search(&db, &fake, "  dark brass  ", 25).unwrap();

        assert_eq!(hits, vec![VibeHit { entry_id: a, score: 0.7 }]);
        let (text, mut candidates, limit) = fake.last_call.lock().unwrap().clone().unwrap();
        assert_eq!(text, strings(&["dark brass"]));
        candidates.sort();
        assert_eq!(candidates, strings(&["h-a", "h-b"]));
        assert_eq!(limit, 25);
    }

    #[test]
    fn search_with_blank_text_or_no_music_skips_the_analyzer() {
        let db = setup();
        add(&db, "a", "music");
        let fake = FakeAnalyzer::default();
        assert_eq!(search(&db, &fake, "   ", 10).unwrap(), vec![]);
        assert!(fake.last_call.lock().unwrap().is_none());

        let empty = setup();
        assert_eq!(search(&empty, &fake, "dark", 10).unwrap(), vec![]);
        assert_eq!(map(&empty, &fake).unwrap(), vec![]);
        assert!(fake.last_call.lock().unwrap().is_none());
    }

    #[test]
    fn limit_is_kept_within_the_analyzers_range() {
        let db = setup();
        let seed = add(&db, "seed", "music");
        let fake = FakeAnalyzer::default();
        similar(&db, &fake, &[seed.clone()], 0).unwrap();
        assert_eq!(fake.last_call.lock().unwrap().clone().unwrap().2, 1);
        similar(&db, &fake, &[seed], 9999).unwrap();
        assert_eq!(fake.last_call.lock().unwrap().clone().unwrap().2, 500);
    }

    #[test]
    fn map_returns_points_for_music_only() {
        let db = setup();
        let a = add(&db, "a", "music");
        add(&db, "fx", "sfx");
        {
            // A music entry the analyzer has no hash for.
            let conn = db.lock().unwrap();
            library::insert_library_entry(&conn, "Z:\\media\\audio\\nohash.opus", None, "nohash", None, None, None, "music")
                .unwrap();
        }
        let fake = FakeAnalyzer {
            points: vec![
                HashPoint { hash: "h-a".into(), x: 0.1, y: 0.2 },
                // Not a music hash; it must not map to any entry.
                HashPoint { hash: "h-fx".into(), x: 0.9, y: 0.9 },
            ],
            ..Default::default()
        };

        let points = map(&db, &fake).unwrap();

        assert_eq!(points, vec![VibePoint { entry_id: a, x: 0.1, y: 0.2 }]);
        let (hashes, _, _) = fake.last_call.lock().unwrap().clone().unwrap();
        assert_eq!(hashes, strings(&["h-a"]));
    }

    #[test]
    fn unreachable_analyzer_returns_error() {
        let db = setup();
        let seed = add(&db, "seed", "music");
        let fake = FakeAnalyzer { down: Some("Could not reach the analyzer: refused".into()), ..Default::default() };

        let expected: Result<Vec<VibeHit>, String> = Err("Could not reach the analyzer: refused".to_string());
        assert_eq!(similar(&db, &fake, &[seed], 10), expected.clone());
        assert_eq!(search(&db, &fake, "dark", 10), expected.clone());
        assert_eq!(map(&db, &fake), Err("Could not reach the analyzer: refused".to_string()));
    }

    #[test]
    fn not_configured_returns_error() {
        let db = setup();
        let message = "Auto-tagging analyzer is not configured".to_string();
        let save = |enabled: bool, url: &str| {
            let conn = db.lock().unwrap();
            settings::save_autotag(
                &conn,
                &AutoTagSettings {
                    enabled,
                    analyzer_url: url.into(),
                    token: "t".into(),
                    local_prefix: String::new(),
                    remote_prefix: String::new(),
                },
            )
            .unwrap();
        };

        save(true, "http://fake");
        assert!(client_for(&db).is_ok());
        save(false, "http://fake");
        assert_eq!(client_for(&db).err(), Some(message.clone()));
        save(true, "   ");
        assert_eq!(client_for(&db).err(), Some(message));
    }
}
