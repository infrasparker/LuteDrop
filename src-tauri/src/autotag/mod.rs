//! Auto-tagging: a background worker that sends library entries to the
//! analyzer service and writes the tags they earn back to the database.
//!
//! `run_once` does one full pass (embed, score, write). `Worker` owns the
//! background thread that decides when to call it, and `Handle` is how the
//! rest of the app talks to that thread.

pub mod client;
pub mod plan;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use crate::database::autotag_db::{self, RunEntry};
use crate::database::{library, now_millis, settings};
use crate::models::{AnalysisStatus, AutoTagProgress, AutoTagReport, TagQuality};
use crate::state::AppStateContainer;
use client::{AnalyzerClient, HttpAnalyzer};
use plan::{EntryFacts, ScoreSet};

/// Most files sent to the analyzer in one `/embed` call.
const EMBED_BATCH: usize = 32;

/// How long a re-score waits after the last tag edit. Edits made in quick
/// succession lead to a single `/score` call.
const RESCORE_DELAY: Duration = Duration::from_secs(5);

/// Event carrying an `AutoTagProgress`, sent while a run is working and when
/// it ends (phase `"done"` or `"error"`).
pub const PROGRESS_EVENT: &str = "autotag-progress";
/// Event carrying the `AutoTagReport` of a run that just ended. The frontend
/// reloads app state when it arrives.
pub const COMPLETE_EVENT: &str = "autotag-complete";

/// Requests the app sends to the worker thread.
enum Message {
    /// Analyze these entries (newly imported) in a run that starts now.
    Queue(Vec<String>),
    /// Analyze every music entry that has no current analysis.
    AnalyzeLibrary,
    /// Re-score the library after a tag change, once edits settle.
    Rescore,
}

/// The app's side of the worker: it sends requests to the worker thread and
/// holds the report of the last run. Lives in `AppStateContainer`.
pub struct Handle {
    sender: Sender<Message>,
    /// The thread's end of the channel, until `Worker::spawn` takes it.
    receiver: Mutex<Option<Receiver<Message>>>,
    report: Mutex<AutoTagReport>,
}

impl Handle {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            report: Mutex::new(AutoTagReport::default()),
        }
    }

    /// Analyzes these entries in a run that starts right away.
    pub fn queue_entries(&self, ids: Vec<String>) {
        if !ids.is_empty() {
            self.send(Message::Queue(ids));
        }
    }

    /// Analyzes every music entry whose analysis is missing, unsuccessful,
    /// or recorded under a different file hash.
    pub fn analyze_library(&self) {
        self.send(Message::AnalyzeLibrary);
    }

    /// Re-scores the library 5 s after the last call.
    pub fn request_rescore(&self) {
        self.send(Message::Rescore);
    }

    /// The report of the most recent run, or an empty report before the first.
    pub fn last_report(&self) -> AutoTagReport {
        self.report.lock().map(|r| r.clone()).unwrap_or_default()
    }

    fn set_report(&self, report: AutoTagReport) {
        if let Ok(mut stored) = self.report.lock() {
            *stored = report;
        }
    }

    fn send(&self, message: Message) {
        // Sending only fails once the worker thread has exited.
        if self.sender.send(message).is_err() {
            log::warn!("Auto-tag worker is not running; request dropped");
        }
    }
}

impl Default for Handle {
    fn default() -> Self {
        Self::new()
    }
}

/// The background thread that runs auto-tagging. It sleeps until a message
/// arrives, gathers the work, and calls `run_once`. While auto-tagging is
/// turned off in settings, it drops every request.
pub struct Worker {
    container: Arc<AppStateContainer>,
    receiver: Receiver<Message>,
    /// Entry ids waiting to be embedded. Kept after a failed run so the next
    /// run tries them again.
    queued: Vec<String>,
    /// When a requested re-score is due, if one is waiting.
    rescore_at: Option<Instant>,
}

impl Worker {
    /// Starts the worker thread. Only the first call does anything.
    pub fn spawn(container: Arc<AppStateContainer>) {
        let receiver = container
            .autotag
            .receiver
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        let Some(receiver) = receiver else {
            log::warn!("Auto-tag worker already started");
            return;
        };
        let worker = Worker {
            container,
            receiver,
            queued: Vec::new(),
            rescore_at: None,
        };
        std::thread::spawn(move || worker.run());
    }

    /// The thread's main loop. Returns when the app drops the channel.
    fn run(mut self) {
        loop {
            // Wait for the next message. While a re-score is pending, wait
            // only until it is due; `None` means the wait ran out.
            let message = match self.rescore_at {
                Some(due) => {
                    let wait = due.saturating_duration_since(Instant::now());
                    match self.receiver.recv_timeout(wait) {
                        Ok(message) => Some(message),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match self.receiver.recv() {
                    Ok(message) => Some(message),
                    Err(_) => return,
                },
            };

            let mut run_now = false;
            if let Some(message) = message {
                run_now |= self.take(message);
            }
            // Take any other messages already waiting, so a burst of
            // requests leads to one run.
            while let Ok(message) = self.receiver.try_recv() {
                run_now |= self.take(message);
            }
            if self.rescore_at.is_some_and(|due| Instant::now() >= due) {
                run_now = true;
            }

            if run_now {
                self.run_now();
            }
        }
    }

    /// Records a message. Returns true if it asks for a run right away.
    fn take(&mut self, message: Message) -> bool {
        match message {
            Message::Queue(ids) => {
                self.add_to_queue(ids);
                true
            }
            Message::AnalyzeLibrary => {
                let ids = lock(&self.container.db).and_then(|conn| entries_needing_analysis(&conn));
                match ids {
                    Ok(ids) => self.add_to_queue(ids),
                    Err(e) => log::warn!("Could not list entries to analyze: {e}"),
                }
                true
            }
            Message::Rescore => {
                // Each new request pushes the due time back.
                self.rescore_at = Some(Instant::now() + RESCORE_DELAY);
                false
            }
        }
    }

    fn add_to_queue(&mut self, ids: Vec<String>) {
        for id in ids {
            if !self.queued.contains(&id) {
                self.queued.push(id);
            }
        }
    }

    fn run_now(&mut self) {
        // Every run scores the whole library, so it covers a pending re-score.
        self.rescore_at = None;

        let container = self.container.clone();
        let settings = match lock(&container.db).and_then(|conn| settings::load_autotag(&conn)) {
            Ok(settings) => settings,
            Err(e) => {
                log::warn!("Could not load auto-tag settings: {e}");
                return;
            }
        };
        if !settings.enabled {
            self.queued.clear();
            return;
        }

        let client = HttpAnalyzer::new(&settings.analyzer_url, &settings.token);
        let report = run_once(&container.db, &client, &self.queued, &|progress| {
            container.emit(PROGRESS_EVENT, progress)
        });
        match &report.error {
            None => self.queued.clear(),
            Some(e) => log::warn!("Auto-tag run failed: {e}"),
        }

        container.autotag.set_report(report.clone());
        // Remotes pick up the new tags on their next snapshot; the desktop
        // frontend reloads app state when it receives the complete event.
        container.mark_state_dirty();
        container.emit(COMPLETE_EVENT, report);
    }
}

/// Locks the database, turning a poisoned lock into an error.
fn lock(db: &Mutex<Connection>) -> Result<MutexGuard<'_, Connection>, String> {
    db.lock().map_err(|e| format!("Database lock poisoned: {e}"))
}

/// True if the analyzer holds an embedding for the entry's current file.
fn is_embedded(entry: &RunEntry) -> bool {
    matches!(&entry.analysis, Some((AnalysisStatus::Embedded, stored)) if *stored == entry.facts.hash)
}

/// True if the entry was embedded, but under a hash other than the file's
/// current one (the file was replaced or relinked since).
fn is_stale(entry: &RunEntry) -> bool {
    matches!(&entry.analysis, Some((AnalysisStatus::Embedded, stored)) if *stored != entry.facts.hash)
}

/// Ids of the music entries `Handle::analyze_library` queues: those with no
/// analysis, a status other than `Embedded`, or a stale embedding.
pub fn entries_needing_analysis(conn: &Connection) -> Result<Vec<String>, String> {
    Ok(autotag_db::load_run_entries(conn)?
        .into_iter()
        .filter(|e| e.facts.is_music && !is_embedded(e))
        .map(|e| e.facts.id)
        .collect())
}

fn quality(tag: &str, set: &ScoreSet) -> TagQuality {
    TagQuality {
        tag: tag.to_string(),
        auc: set.quality.auc,
        pos: set.quality.pos,
        neg: set.quality.neg,
        trusted: set.trusted,
        threshold: set.threshold,
    }
}

fn progress_update(phase: &str, processed: u32, total: u32, message: Option<String>) -> AutoTagProgress {
    AutoTagProgress {
        phase: phase.to_string(),
        processed,
        total,
        message,
    }
}

/// One auto-tagging pass:
/// 1. load settings, scene config and every entry;
/// 2. map the paths of the entries to embed (`queued`, plus any whose file
///    changed since it was embedded); unmappable ones become `UnreachablePath`;
/// 3. embed them in batches of 32, marking each `Embedded` or `Failed`;
/// 4. call `/score` once for every embedded music entry;
/// 5. decide each scored entry's auto tags;
/// 6. write those tags and the scores in one transaction.
///
/// The database lock is released while the analyzer works, so the app stays
/// usable. An analyzer or database error stops the run before step 6 commits,
/// so tags are left as they were, and the report carries the error.
pub fn run_once(
    db: &Mutex<Connection>,
    client: &dyn AnalyzerClient,
    queued: &[String],
    progress: &dyn Fn(AutoTagProgress),
) -> AutoTagReport {
    let mut report = AutoTagReport::default();
    if let Err(e) = run_steps(db, client, queued, progress, &mut report) {
        progress(progress_update("error", 0, 0, Some(e.clone())));
        report.error = Some(e);
    }
    report.finished_at = Some(now_millis());
    report
}

fn run_steps(
    db: &Mutex<Connection>,
    client: &dyn AnalyzerClient,
    queued: &[String],
    progress: &dyn Fn(AutoTagProgress),
    report: &mut AutoTagReport,
) -> Result<(), String> {
    // Step 1: read everything the run needs, then let go of the database.
    let (settings, scene, entries) = {
        let conn = lock(db)?;
        (
            settings::load_autotag(&conn)?,
            settings::load_scene_config(&conn)?,
            autotag_db::load_run_entries(&conn)?,
        )
    };

    // Hashes the analyzer already has embeddings for.
    let mut embedded: HashSet<String> = entries
        .iter()
        .filter(|e| e.facts.is_music && is_embedded(e))
        .filter_map(|e| e.facts.hash.clone())
        .collect();

    // Step 2: pick the entries to embed and map their paths.
    let queued: HashSet<&str> = queued.iter().map(String::as_str).collect();
    // (entry id, hash, path on the analyzer host)
    let mut to_embed: Vec<(&str, String, String)> = Vec::new();
    {
        let conn = lock(db)?;
        for entry in entries.iter().filter(|e| e.facts.is_music) {
            let id = entry.facts.id.as_str();
            if !queued.contains(id) && !is_stale(entry) {
                continue;
            }
            let Some(hash) = entry.facts.hash.clone() else {
                let error = Some("File has no content hash");
                autotag_db::set_analysis(&conn, id, None, AnalysisStatus::Failed, error)?;
                report.failed += 1;
                continue;
            };
            match plan::map_path(&entry.file_path, &settings.local_prefix, &settings.remote_prefix) {
                Some(remote) => to_embed.push((id, hash, remote)),
                None => {
                    autotag_db::set_analysis(&conn, id, Some(&hash), AnalysisStatus::UnreachablePath, None)?;
                    embedded.remove(&hash);
                    report.unreachable += 1;
                }
            }
        }
    }

    // Step 3: embed in batches, recording each item's outcome as it arrives.
    let total = to_embed.len() as u32;
    if total > 0 {
        progress(progress_update("embedding", 0, total, None));
    }
    let mut processed = 0;
    for batch in to_embed.chunks(EMBED_BATCH) {
        let items: Vec<(String, String)> = batch
            .iter()
            .map(|(_, hash, remote)| (hash.clone(), remote.clone()))
            .collect();
        let results = client.embed(&items)?;
        let by_hash: HashMap<&str, _> = results.iter().map(|r| (r.hash.as_str(), r)).collect();

        let conn = lock(db)?;
        for (id, hash, _) in batch {
            let error = match by_hash.get(hash.as_str()) {
                Some(result) if result.ok => None,
                Some(result) => Some(result.error.clone().unwrap_or_else(|| "Embedding failed".into())),
                None => Some("The analyzer returned no result for this file".to_string()),
            };
            match error {
                None => {
                    autotag_db::set_analysis(&conn, id, Some(hash), AnalysisStatus::Embedded, None)?;
                    embedded.insert(hash.clone());
                }
                Some(error) => {
                    autotag_db::set_analysis(&conn, id, Some(hash), AnalysisStatus::Failed, Some(&error))?;
                    embedded.remove(hash);
                    report.failed += 1;
                }
            }
        }
        drop(conn);
        processed += batch.len() as u32;
        progress(progress_update("embedding", processed, total, None));
    }

    // Step 4: score every embedded music entry.
    let facts: Vec<EntryFacts> = entries.iter().map(|e| e.facts.clone()).collect();
    let request = plan::build_score_request(&facts, &scene, &embedded);
    if request.targets.is_empty() {
        progress(progress_update("done", 0, 0, None));
        return Ok(());
    }
    let target_count = request.targets.len() as u32;
    progress(progress_update("scoring", 0, target_count, None));
    let response = client.score(&request)?;

    // Step 5: decide. Only scored entries appear in `decisions`.
    let decisions = plan::decide(&facts, &scene, &response);

    // Step 6: one transaction for every tag and score write. If anything
    // fails, `tx` is dropped without a commit and SQLite rolls it all back.
    let gate_tag = scene.gate_tag.trim().to_lowercase();
    let (mut analyzed, mut added, mut removed) = (0, 0, 0);
    {
        let conn = lock(db)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Failed to begin auto-tag transaction: {e}"))?;
        for entry in &entries {
            let id = &entry.facts.id;
            let (Some(tags), Some(hash)) = (decisions.get(id), &entry.facts.hash) else {
                continue;
            };
            // The entry may have been deleted while the analyzer worked.
            if !entry_exists(&conn, id)? {
                continue;
            }
            let (a, r) = library::apply_auto_tags(&conn, id, tags)?;
            added += a;
            removed += r;
            if let Some(mut scores) = plan::entry_scores(hash, &response) {
                // Shown in the UI under the gate tag's name (e.g. "encounter").
                scores.gate.tag = gate_tag.clone();
                autotag_db::save_scores(&conn, id, &scores)?;
            }
            analyzed += 1;
        }
        tx.commit()
            .map_err(|e| format!("Failed to commit auto-tag transaction: {e}"))?;
    }

    report.analyzed = analyzed;
    report.added = added;
    report.removed = removed;
    report.gate = Some(quality(&gate_tag, &response.gate));
    let mut tags: Vec<TagQuality> = response
        .tags
        .iter()
        .map(|(name, set)| quality(name, set))
        .collect();
    tags.sort_by(|a, b| a.tag.cmp(&b.tag));
    report.tags = tags;

    progress(progress_update("done", analyzed, target_count, None));
    Ok(())
}

fn entry_exists(conn: &Connection, id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_entries WHERE id = ?1)",
        params![id],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to check entry: {e}"))
}

#[cfg(test)]
mod tests {
    use super::client::{AnalyzerClient, EmbedResult};
    use super::plan::{Quality, ScoreRequest, ScoreResponse, ScoreSet};
    use super::*;
    use crate::database::{autotag_db, library, open_memory_db, settings};
    use crate::models::{AnalysisStatus, AutoTagProgress, AutoTagSettings, SceneConfig, TagSource};
    use rusqlite::{params, Connection, OptionalExtension};
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    /// Analyzer stand-in. Every classifier is trusted with threshold 0.5;
    /// `/score` returns the configured score for each requested target.
    #[derive(Default)]
    struct FakeAnalyzer {
        /// When set, every call fails with this message.
        down: Option<String>,
        /// Hashes whose embed item fails, with the analyzer's error.
        embed_failures: HashMap<String, String>,
        gate: HashMap<String, f64>,
        tags: HashMap<String, HashMap<String, f64>>,
        /// Runs inside `score`, before it returns.
        on_score: Option<Box<dyn Fn() + Send + Sync>>,
        embedded: Mutex<Vec<String>>,
    }

    impl FakeAnalyzer {
        fn embedded_hashes(&self) -> Vec<String> {
            self.embedded.lock().unwrap().clone()
        }
    }

    fn trusted_set(scores: &HashMap<String, f64>, targets: &[String]) -> ScoreSet {
        ScoreSet {
            quality: Quality { auc: Some(0.9), pos: 10, neg: 10 },
            trusted: true,
            threshold: Some(0.5),
            scores: targets
                .iter()
                .filter_map(|h| scores.get(h).map(|s| (h.clone(), *s)))
                .collect(),
        }
    }

    impl AnalyzerClient for FakeAnalyzer {
        fn health(&self) -> Result<String, String> {
            match &self.down {
                Some(e) => Err(e.clone()),
                None => Ok("fake".into()),
            }
        }

        fn embed(&self, items: &[(String, String)]) -> Result<Vec<EmbedResult>, String> {
            if let Some(e) = &self.down {
                return Err(e.clone());
            }
            assert!(items.len() <= 32, "embed batches hold at most 32 items");
            Ok(items
                .iter()
                .map(|(hash, _)| {
                    self.embedded.lock().unwrap().push(hash.clone());
                    let error = self.embed_failures.get(hash).cloned();
                    EmbedResult { hash: hash.clone(), ok: error.is_none(), error }
                })
                .collect())
        }

        fn score(&self, req: &ScoreRequest) -> Result<ScoreResponse, String> {
            if let Some(e) = &self.down {
                return Err(e.clone());
            }
            if let Some(hook) = &self.on_score {
                hook();
            }
            Ok(ScoreResponse {
                model: "fake".into(),
                gate: trusted_set(&self.gate, &req.targets),
                tags: self
                    .tags
                    .iter()
                    .map(|(name, scores)| (name.clone(), trusted_set(scores, &req.targets)))
                    .collect(),
            })
        }
    }

    fn setup() -> Arc<Mutex<Connection>> {
        let conn = open_memory_db();
        settings::save_autotag(
            &conn,
            &AutoTagSettings {
                enabled: true,
                analyzer_url: "http://fake".into(),
                token: "t".into(),
                local_prefix: "Z:\\media\\audio".into(),
                remote_prefix: "/media/audio".into(),
            },
        )
        .unwrap();
        settings::save_scene_config(
            &conn,
            &SceneConfig {
                scene: "encounter".into(),
                gate_tag: "encounter".into(),
                negative_scene_tags: vec!["tavern".into()],
                vocabulary: vec!["brass".into(), "dark".into()],
            },
        )
        .unwrap();
        Arc::new(Mutex::new(conn))
    }

    /// Adds an entry at `Z:\media\audio\<name>.opus` with hash `h-<name>`.
    fn add(db: &Mutex<Connection>, name: &str, entry_type: &str) -> String {
        let conn = db.lock().unwrap();
        library::insert_library_entry(
            &conn,
            &format!("Z:\\media\\audio\\{name}.opus"),
            Some(&format!("h-{name}")),
            name,
            None,
            None,
            None,
            entry_type,
        )
        .unwrap()
    }

    fn add_user_tag(db: &Mutex<Connection>, entry: &str, tag: &str) {
        library::add_tag_to_entry(&db.lock().unwrap(), entry, tag).unwrap();
    }

    /// Returns (source, score) of the entry's row for `tag`, if any.
    fn row_of(db: &Mutex<Connection>, entry: &str, tag: &str) -> Option<(TagSource, Option<f64>)> {
        db.lock()
            .unwrap()
            .query_row(
                "SELECT lt.source, lt.score FROM library_entry_tags lt
                 JOIN tags t ON t.id = lt.tag_id
                 WHERE lt.library_entry_id = ?1 AND t.name = ?2",
                params![entry, tag],
                |r| Ok((TagSource::from_str(&r.get::<_, String>(0)?), r.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    fn status_of(db: &Mutex<Connection>, entry: &str) -> Option<(AnalysisStatus, Option<String>)> {
        autotag_db::get_analysis(&db.lock().unwrap(), entry)
            .unwrap()
            .map(|(a, _)| (a.status, a.error))
    }

    fn scores(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(h, s)| (h.to_string(), *s)).collect()
    }

    fn no_progress(_: AutoTagProgress) {}

    #[test]
    fn run_applies_auto_tags_and_saves_scores() {
        let db = setup();
        let a = add(&db, "a", "music");
        let b = add(&db, "b", "music");
        add_user_tag(&db, &a, "encounter");
        let fake = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9), ("h-b", 0.8)]),
            tags: HashMap::from([("brass".to_string(), scores(&[("h-a", 0.7), ("h-b", 0.9)]))]),
            ..Default::default()
        };

        let report = run_once(&db, &fake, &[a.clone(), b.clone()], &no_progress);

        assert_eq!(report.error, None);
        assert_eq!(report.analyzed, 2);
        assert_eq!(report.added, 3);
        assert_eq!(report.removed, 0);
        assert!(report.finished_at.is_some());
        assert_eq!(report.gate.as_ref().unwrap().tag, "encounter");
        assert_eq!(row_of(&db, &a, "encounter"), Some((TagSource::User, None)));
        assert_eq!(row_of(&db, &a, "brass"), Some((TagSource::Auto, Some(0.7))));
        assert_eq!(row_of(&db, &b, "encounter"), Some((TagSource::Auto, Some(0.8))));
        assert_eq!(row_of(&db, &b, "brass"), Some((TagSource::Auto, Some(0.9))));
        assert_eq!(status_of(&db, &a), Some((AnalysisStatus::Embedded, None)));

        let saved = autotag_db::load_scores(&db.lock().unwrap(), &b).unwrap().unwrap();
        assert_eq!(saved.gate.tag, "encounter", "gate score is labeled with the gate tag");
        assert_eq!(saved.gate.applied, Some(TagSource::Auto));
        assert_eq!(saved.tags[0].tag, "brass");
    }

    #[test]
    fn run_rescores_embedded_entries_without_embedding_again() {
        let db = setup();
        let a = add(&db, "a", "music");
        let fake = FakeAnalyzer { gate: scores(&[("h-a", 0.9)]), ..Default::default() };
        run_once(&db, &fake, &[a.clone()], &no_progress);
        assert_eq!(fake.embedded_hashes(), vec!["h-a"]);

        let report = run_once(&db, &fake, &[], &no_progress);
        assert_eq!(fake.embedded_hashes(), vec!["h-a"], "no second /embed");
        assert_eq!(report.analyzed, 1);
        assert_eq!(row_of(&db, &a, "encounter"), Some((TagSource::Auto, Some(0.9))));
    }

    #[test]
    fn run_with_unreachable_analyzer_keeps_existing_auto_tags() {
        let db = setup();
        let a = add(&db, "a", "music");
        let ok = FakeAnalyzer { gate: scores(&[("h-a", 0.9)]), ..Default::default() };
        run_once(&db, &ok, &[a.clone()], &no_progress);
        assert_eq!(row_of(&db, &a, "encounter"), Some((TagSource::Auto, Some(0.9))));

        let down = FakeAnalyzer { down: Some("connection refused".into()), ..Default::default() };
        let phases = Mutex::new(Vec::new());
        let report = run_once(&db, &down, &[a.clone()], &|p: AutoTagProgress| {
            phases.lock().unwrap().push(p.phase)
        });

        assert!(report.error.as_deref().unwrap().contains("connection refused"));
        assert_eq!(phases.lock().unwrap().last().map(String::as_str), Some("error"));
        assert_eq!(row_of(&db, &a, "encounter"), Some((TagSource::Auto, Some(0.9))));
    }

    #[test]
    fn run_marks_unmappable_paths_unreachable() {
        let db = setup();
        let outside = {
            let conn = db.lock().unwrap();
            library::insert_library_entry(&conn, "D:\\other\\x.opus", Some("h-x"), "x", None, None, None, "music")
                .unwrap()
        };
        let fake = FakeAnalyzer::default();

        let report = run_once(&db, &fake, &[outside.clone()], &no_progress);

        assert_eq!(report.unreachable, 1);
        assert_eq!(report.error, None);
        assert_eq!(status_of(&db, &outside).map(|s| s.0), Some(AnalysisStatus::UnreachablePath));
        assert!(fake.embedded_hashes().is_empty());
    }

    #[test]
    fn run_marks_failed_embeds_and_continues() {
        let db = setup();
        let a = add(&db, "a", "music");
        let b = add(&db, "b", "music");
        let fake = FakeAnalyzer {
            embed_failures: HashMap::from([("h-b".to_string(), "decode failed".to_string())]),
            gate: scores(&[("h-a", 0.9), ("h-b", 0.9)]),
            ..Default::default()
        };

        let report = run_once(&db, &fake, &[a.clone(), b.clone()], &no_progress);

        assert_eq!(report.error, None);
        assert_eq!(report.failed, 1);
        assert_eq!(report.analyzed, 1);
        assert_eq!(
            status_of(&db, &b),
            Some((AnalysisStatus::Failed, Some("decode failed".to_string())))
        );
        assert_eq!(row_of(&db, &a, "encounter"), Some((TagSource::Auto, Some(0.9))));
        assert_eq!(row_of(&db, &b, "encounter"), None);
    }

    #[test]
    fn run_skips_ambiance_entries() {
        let db = setup();
        let a = add(&db, "a", "music");
        let amb = add(&db, "rain", "ambiance");
        let fake = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9), ("h-rain", 0.9)]),
            ..Default::default()
        };

        let report = run_once(&db, &fake, &[a.clone(), amb.clone()], &no_progress);

        assert_eq!(fake.embedded_hashes(), vec!["h-a"]);
        assert_eq!(report.analyzed, 1);
        assert_eq!(status_of(&db, &amb), None);
        assert_eq!(row_of(&db, &amb, "encounter"), None);
    }

    #[test]
    fn user_tag_added_during_scoring_is_not_overwritten() {
        let db = setup();
        let a = add(&db, "a", "music");
        add_user_tag(&db, &a, "encounter");
        let hook_db = db.clone();
        let hook_entry = a.clone();
        let fake = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9)]),
            tags: HashMap::from([("brass".to_string(), scores(&[("h-a", 0.9)]))]),
            on_score: Some(Box::new(move || {
                // The run must not hold the database while waiting on /score.
                let conn = hook_db.try_lock().expect("database is free during /score");
                library::add_tag_to_entry(&conn, &hook_entry, "brass").unwrap();
            })),
            ..Default::default()
        };

        let report = run_once(&db, &fake, &[a.clone()], &no_progress);

        assert_eq!(report.error, None);
        assert_eq!(row_of(&db, &a, "brass"), Some((TagSource::User, None)));
        assert_eq!(report.added, 0);
    }

    #[test]
    fn entries_needing_analysis_are_missing_failed_or_stale() {
        let db = setup();
        let fresh = add(&db, "fresh", "music");
        let done = add(&db, "done", "music");
        let failed = add(&db, "failed", "music");
        let stale = add(&db, "stale", "music");
        add(&db, "rain", "ambiance");
        {
            let conn = db.lock().unwrap();
            autotag_db::set_analysis(&conn, &done, Some("h-done"), AnalysisStatus::Embedded, None).unwrap();
            autotag_db::set_analysis(&conn, &failed, Some("h-failed"), AnalysisStatus::Failed, Some("x"))
                .unwrap();
            autotag_db::set_analysis(&conn, &stale, Some("h-old"), AnalysisStatus::Embedded, None).unwrap();
        }
        let ids: HashSet<String> = entries_needing_analysis(&db.lock().unwrap()).unwrap().into_iter().collect();
        assert_eq!(ids, HashSet::from([fresh, failed, stale]));
    }
}
