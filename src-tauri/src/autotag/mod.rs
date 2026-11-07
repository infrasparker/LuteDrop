//! Auto-tagging: a background worker that sends library entries to the
//! analyzer service and writes the role and tags they earn back to the
//! database.
//!
//! `run_once` does one full pass (embed, score, write). `Worker` owns the
//! background thread that decides when to call it, and `Handle` is how the
//! rest of the app talks to that thread.

pub mod case_import;
pub mod client;
pub mod plan;
pub mod scry;
pub mod tune;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};

use crate::database::autotag_db::{self, RunEntry};
use crate::database::{library, now_millis, settings};
use crate::models::{AnalysisStatus, AutoTagProgress, AutoTagReport, MoodVocabulary, TagQuality};
use crate::state::AppStateContainer;
use client::{AnalyzerClient, HttpAnalyzer};
use plan::{EntryFacts, ScoreSet};

/// Most files sent to the analyzer in one `/embed` call.
const EMBED_BATCH: usize = 32;

/// Most per-entry errors a report lists in `sample_errors`.
const MAX_SAMPLE_ERRORS: usize = 5;

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
    /// Analyze music entries. With `all` false, only those with no current
    /// analysis; with `all` true, every music entry.
    AnalyzeLibrary { all: bool },
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
        self.send(Message::AnalyzeLibrary { all: false });
    }

    /// Analyzes every music entry, including those already embedded. The
    /// analyzer answers `/embed` at once for hashes it has cached, and
    /// re-embeds the ones it lost.
    pub fn reanalyze_library(&self) {
        self.send(Message::AnalyzeLibrary { all: true });
    }

    /// Re-scores the library 5 s after the last call.
    pub fn request_rescore(&self) {
        self.send(Message::Rescore);
    }

    /// The report of the most recent run, or an empty report before the first.
    pub fn last_report(&self) -> AutoTagReport {
        self.report.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Drains the requests still waiting for the worker and returns how many
    /// were library analyses. Only meaningful before `Worker::spawn` runs.
    #[cfg(test)]
    pub fn pending_analyze_requests(&self) -> usize {
        let receiver = self.receiver.lock().unwrap();
        let receiver = receiver.as_ref().expect("worker not spawned");
        receiver.try_iter().filter(|m| matches!(m, Message::AnalyzeLibrary { .. })).count()
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
            Message::AnalyzeLibrary { all } => {
                let ids = lock(&self.container.db).and_then(|conn| {
                    if all {
                        all_music_entries(&conn)
                    } else {
                        entries_needing_analysis(&conn)
                    }
                });
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
        let mut report = run_once(&container.db, &client, &self.queued, &|progress| {
            container.emit(PROGRESS_EVENT, progress)
        });
        match &report.error {
            None => self.queued.clear(),
            Some(e) => log::warn!("Auto-tag run failed: {e}"),
        }
        keep_previous_quality(&container.autotag.last_report(), &mut report);

        container.autotag.set_report(report.clone());
        // Remotes pick up the new tags on their next snapshot; the desktop
        // frontend reloads app state when it receives the complete event.
        container.mark_state_dirty();
        container.emit(COMPLETE_EVENT, report);
    }
}

/// A failed run measures no tag quality. So that the quality table stays
/// visible next to the error, a failed report takes the `gate` and `tags` of
/// the previous report. A successful report is left as it is.
fn keep_previous_quality(previous: &AutoTagReport, report: &mut AutoTagReport) {
    if report.error.is_some() {
        report.gate = previous.gate.clone();
        report.tags = previous.tags.clone();
    }
}

/// Adds "entry name: error" to the report's sample errors, up to
/// `MAX_SAMPLE_ERRORS` of them.
fn note_error(report: &mut AutoTagReport, name: &str, error: &str) {
    if report.sample_errors.len() < MAX_SAMPLE_ERRORS {
        report.sample_errors.push(format!("{name}: {error}"));
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

/// Ids of the music entries `Handle::reanalyze_library` queues: every music
/// entry that has a content hash, plus those not yet hashed (as in
/// `entries_needing_analysis`, a run hashes them).
pub fn all_music_entries(conn: &Connection) -> Result<Vec<String>, String> {
    Ok(autotag_db::load_run_entries(conn)?
        .into_iter()
        .filter(|e| e.facts.is_music)
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
/// 4. call `/score` once for every embedded music entry, sending the mood
///    vocabulary's prompts;
/// 5. take the mood calibration: the analyzer's reference set when it offers
///    one for the current prompts (replacing a stored library calibration or
///    an older reference, unless identical), else, if none is stored yet and
///    this response covers enough music, the library's own. Then decide each
///    scored entry's analyzer layer (role, moods, tags); only moods whose
///    prompts match the calibration's are scored;
/// 6. write that layer, the scores and a new calibration in one transaction.
///    The user's layer is never written, so the run can't undo an edit made
///    meanwhile. A new calibration (library or reference) is saved in the same
///    transaction. If the stored calibration changed while the analyzer worked
///    (Recalibrate cleared it), neither it nor any mood is written: the run
///    Recalibrate queued does that.
///
/// The database lock is released while the analyzer works, so the app stays
/// usable. An analyzer or database error stops the run before step 6 commits,
/// so roles and tags are left as they were, and the report carries the error.
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
    let (settings, scene, entries, stored_calibration, mood_vocabulary) = {
        let conn = lock(db)?;
        (
            settings::load_autotag(&conn)?,
            settings::load_scene_config(&conn)?,
            autotag_db::load_run_entries(&conn)?,
            settings::load_mood_calibration(&conn)?,
            settings::load_mood_vocabulary(&conn)?,
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
    // (entry id, entry name, hash, path on the analyzer host)
    let mut to_embed: Vec<(&str, &str, String, String)> = Vec::new();
    {
        let conn = lock(db)?;
        for entry in entries.iter().filter(|e| e.facts.is_music) {
            let id = entry.facts.id.as_str();
            if !queued.contains(id) && !is_stale(entry) {
                continue;
            }
            let Some(hash) = entry.facts.hash.clone() else {
                let error = "File has no content hash";
                autotag_db::set_analysis(&conn, id, None, AnalysisStatus::Failed, Some(error))?;
                report.failed += 1;
                note_error(report, &entry.name, error);
                continue;
            };
            match plan::map_path(&entry.file_path, &settings.local_prefix, &settings.remote_prefix) {
                Some(remote) => to_embed.push((id, &entry.name, hash, remote)),
                None => {
                    autotag_db::set_analysis(&conn, id, Some(&hash), AnalysisStatus::UnreachablePath, None)?;
                    embedded.remove(&hash);
                    report.unreachable += 1;
                    let error = format!(
                        "{} is not under the local prefix \"{}\"",
                        entry.file_path, settings.local_prefix
                    );
                    note_error(report, &entry.name, &error);
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
            .map(|(_, _, hash, remote)| (hash.clone(), remote.clone()))
            .collect();
        let results = client.embed(&items)?;
        let by_hash: HashMap<&str, _> = results.iter().map(|r| (r.hash.as_str(), r)).collect();

        let conn = lock(db)?;
        for (id, name, hash, _) in batch {
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
                    note_error(report, name, &error);
                }
            }
        }
        drop(conn);
        processed += batch.len() as u32;
        progress(progress_update("embedding", processed, total, None));
    }

    // Step 4: score every embedded music entry.
    let facts: Vec<EntryFacts> = entries.iter().map(|e| e.facts.clone()).collect();
    let request = plan::build_score_request(&facts, &scene, &embedded).with_moods(&mood_vocabulary);
    if request.targets.is_empty() {
        progress(progress_update("done", 0, 0, None));
        return Ok(());
    }
    let target_count = request.targets.len() as u32;
    progress(progress_update("scoring", 0, target_count, None));
    let response = client.score(&request)?;

    // Step 5: decide. Only scored entries appear in `decisions`. The mood
    // calibration is frozen: once stored it is only replaced by a reference
    // calibration (below), never recomputed from the library (Recalibrate
    // clears it). Until one exists, no mood is scored; after, only the moods
    // whose prompts it was taken with.
    //
    // `offered` is the reference calibration the analyzer sent for the
    // prompts of this run, if it sent one we can use. The `match` compares
    // it with what is stored, and the first arm that fits wins.
    let offered = response
        .mood_reference
        .as_ref()
        .and_then(|r| plan::reference_calibration(r, &mood_vocabulary, now_millis()));
    let new_calibration = match (offered, &stored_calibration) {
        // The reference is what we already have: leave it (and its date).
        (Some(r), Some(s)) if plan::same_calibration(&r, s) => None,
        // A reference replaces whatever is stored: a library calibration, or
        // a reference for older prompts or another set.
        (Some(r), _) => Some(r),
        // No reference offered (older analyzer or no prompts match): keep
        // what is stored; a library calibration gains statistics for moods
        // whose lines changed, so those are rescored and nothing else moves.
        (None, Some(s)) => plan::extend_library_calibration(s, &response.mood, &mood_vocabulary),
        // Nothing stored and no reference: take the library's own, if it
        // covers enough music.
        (None, None) => plan::calibrate(&response.mood, &mood_vocabulary, now_millis()),
    };
    let calibration = new_calibration.as_ref().or(stored_calibration.as_ref());
    let mood_ids = calibration.map(|c| plan::scorable_ids(c, &mood_vocabulary)).unwrap_or_default();
    let decisions = plan::decide(&facts, &scene, &response, calibration, &mood_ids);

    // Step 6: one transaction for every role, tag and score write. If
    // anything fails, `tx` is dropped without a commit and SQLite rolls it
    // all back.
    let (mut analyzed, mut added, mut removed) = (0, 0, 0);
    {
        let conn = lock(db)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("Failed to begin auto-tag transaction: {e}"))?;
        // Recalibrate (or a vocabulary save) may have run while the analyzer
        // worked: write moods only if the calibration is still the one this
        // run started with and every id still has the prompts the analyzer
        // scored. A changed vocabulary means a calibration taken here would
        // be signed with stale prompts, so the run skips moods and leaves
        // them to the next run.
        let vocabulary_now = settings::load_mood_vocabulary(&conn)?;
        let moods_current = settings::load_mood_calibration(&conn)? == stored_calibration
            && same_prompts(&vocabulary_now, &mood_vocabulary);
        if let Some(calibration) = new_calibration.as_ref().filter(|_| moods_current) {
            settings::save_mood_calibration(&conn, calibration)?;
            match (&calibration.source, &calibration.reference_set) {
                (plan::CalibrationSource::Reference, Some(set)) => log::info!(
                    "Mood calibration taken from the reference set {set} ({} tracks)",
                    calibration.tracks
                ),
                _ => log::info!("Mood calibration taken from {} tracks", calibration.tracks),
            }
        }
        for entry in &entries {
            let id = &entry.facts.id;
            let (Some(decision), Some(hash)) = (decisions.get(id), &entry.facts.hash) else {
                continue;
            };
            // The entry may have been deleted, or changed to another type,
            // while the analyzer worked.
            if !entry_is_music(&conn, id)? {
                continue;
            }
            // Only the analyzer layer is written; the user's choices live in
            // their own columns, so edits made while the analyzer worked stay.
            let (role, role_score) = match decision.role {
                Some((role, score)) => (Some(role), score),
                None => (None, 0.0),
            };
            library::apply_auto_role(&conn, id, role, role_score)?;
            // Moods: only entries with fresh values change. A run without
            // mood (model not loaded), a calibration or the current mood
            // shape leaves stored values alone. The user's levels are never
            // written here.
            if let Some(moods) = decision.moods.as_ref().filter(|_| moods_current) {
                let moods: Vec<(String, f64)> =
                    moods.iter().filter(|(m, _)| vocabulary_now.contains(m)).cloned().collect();
                library::apply_auto_moods(&conn, id, &moods)?;
            }
            // Counts changes to the tags the entry shows (see apply_auto_tags).
            let (a, r) = library::apply_auto_tags(&conn, id, &decision.tags)?;
            added += a;
            removed += r;
            if let Some(scores) = plan::entry_scores(hash, &response) {
                autotag_db::save_scores(&conn, id, &scores)?;
            }
            analyzed += 1;
        }
        // Auto tags dropped above may have left tag rows no entry uses.
        library::sweep_orphan_tags(&conn)?;
        tx.commit()
            .map_err(|e| format!("Failed to commit auto-tag transaction: {e}"))?;
    }

    report.analyzed = analyzed;
    report.added = added;
    report.removed = removed;
    report.gate = Some(quality(plan::GATE_NAME, &response.gate));
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

/// True if both vocabularies have the same ids with the same prompt
/// signature each (names and order may differ: they do not affect scores).
fn same_prompts(a: &MoodVocabulary, b: &MoodVocabulary) -> bool {
    let signatures = |v: &MoodVocabulary| -> BTreeMap<String, Option<Vec<String>>> {
        v.ids().into_iter().map(|id| { let sig = v.prompt_signature(&id); (id, sig) }).collect()
    };
    signatures(a) == signatures(b)
}

/// True if the entry exists and is currently a music entry.
fn entry_is_music(conn: &Connection, id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_entries WHERE id = ?1 AND entry_type = 'music')",
        params![id],
        |row| row.get(0),
    )
    .map_err(|e| format!("Failed to check entry: {e}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::client::{AnalyzerClient, EmbedResult, HashPoint, HashScore};
    use super::plan::{MoodRaw, Quality, ScoreRequest, ScoreResponse, ScoreSet};
    use super::*;
    use crate::database::{autotag_db, library, open_memory_db, settings};
    use crate::models::{AnalysisStatus, AutoTagProgress, AutoTagSettings, MoodVocabulary, Role, SceneConfig, TagSource};
    use rusqlite::{params, Connection, OptionalExtension};
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};

    /// Analyzer stand-in. Every classifier is trusted (unless `untrusted_gate`
    /// is set for the gate), with threshold and Non-combat threshold 0.5;
    /// `/score` returns the configured score for each requested target.
    #[derive(Default)]
    pub(crate) struct FakeAnalyzer {
        /// When set, every call fails with this message.
        pub(crate) down: Option<String>,
        /// Hashes whose embed item fails, with the analyzer's error.
        pub(crate) embed_failures: HashMap<String, String>,
        pub(crate) gate: HashMap<String, f64>,
        pub(crate) untrusted_gate: bool,
        pub(crate) tags: HashMap<String, HashMap<String, f64>>,
        /// The request of the last `score` call.
        pub(crate) last_score: Mutex<Option<ScoreRequest>>,
        /// Raw mood scores per hash; empty omits mood.
        pub(crate) mood: HashMap<String, MoodRaw>,
        /// The reference-set statistics `score` offers; `None` omits them.
        pub(crate) mood_reference: Option<plan::MoodReference>,
        /// Runs inside `score`, before it returns.
        pub(crate) on_score: Option<Box<dyn Fn() + Send + Sync>>,
        pub(crate) embedded: Mutex<Vec<String>>,
        /// What `similar` and `search` return, whatever the request.
        pub(crate) ranked: Vec<HashScore>,
        /// What `map` returns, whatever the request.
        pub(crate) points: Vec<HashPoint>,
        /// The arguments of the last `similar`, `search` or `map` call: the
        /// query (or text, or hashes), the candidates, and the limit.
        pub(crate) last_call: Mutex<Option<(Vec<String>, Vec<String>, u32)>>,
        /// What `tune` returns; `None` returns an empty response.
        pub(crate) tune: Option<super::tune::TuneResponse>,
        /// The request of the last `tune` call.
        pub(crate) last_tune: Mutex<Option<super::tune::TuneRequest>>,
    }

    impl FakeAnalyzer {
        /// Notes the call for `last_call`, or fails if the analyzer is down.
        fn record(&self, first: Vec<String>, candidates: &[String], limit: u32) -> Result<(), String> {
            if let Some(e) = &self.down {
                return Err(e.clone());
            }
            *self.last_call.lock().unwrap() = Some((first, candidates.to_vec(), limit));
            Ok(())
        }

        fn embedded_hashes(&self) -> Vec<String> {
            self.embedded.lock().unwrap().clone()
        }
    }

    fn trusted_set(scores: &HashMap<String, f64>, targets: &[String]) -> ScoreSet {
        ScoreSet {
            quality: Quality { auc: Some(0.9), pos: 10, neg: 10 },
            trusted: true,
            threshold: Some(0.5),
            neg_threshold: Some(0.5),
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
            *self.last_score.lock().unwrap() = Some(req.clone());
            if let Some(hook) = &self.on_score {
                hook();
            }
            Ok(ScoreResponse {
                mood_reference: self.mood_reference.clone(),
                model: "fake".into(),
                gate: ScoreSet {
                    trusted: !self.untrusted_gate,
                    ..trusted_set(&self.gate, &req.targets)
                },
                tags: self
                    .tags
                    .iter()
                    .map(|(name, scores)| (name.clone(), trusted_set(scores, &req.targets)))
                    .collect(),
                mood: self
                    .mood
                    .iter()
                    .filter(|(h, _)| req.targets.contains(h))
                    .map(|(h, m)| (h.clone(), m.clone()))
                    .collect(),
            })
        }

        fn similar(&self, query: &[String], candidates: &[String], limit: u32) -> Result<Vec<HashScore>, String> {
            self.record(query.to_vec(), candidates, limit)?;
            Ok(self.ranked.clone())
        }

        fn search(&self, text: &str, candidates: &[String], limit: u32) -> Result<Vec<HashScore>, String> {
            self.record(vec![text.to_string()], candidates, limit)?;
            Ok(self.ranked.clone())
        }

        fn map(&self, hashes: &[String]) -> Result<Vec<HashPoint>, String> {
            self.record(hashes.to_vec(), &[], 0)?;
            Ok(self.points.clone())
        }

        /// Returns `ranked` as a map for the targets it names, recording the
        /// prompts as the query and the targets as the candidates.
        fn preview(&self, prompts: &[String], targets: &[String]) -> Result<HashMap<String, f64>, String> {
            self.record(prompts.to_vec(), targets, 0)?;
            Ok(self.ranked.iter().filter(|r| targets.contains(&r.hash)).map(|r| (r.hash.clone(), r.score)).collect())
        }

        fn tune(&self, req: &super::tune::TuneRequest) -> Result<super::tune::TuneResponse, String> {
            if let Some(e) = &self.down {
                return Err(e.clone());
            }
            *self.last_tune.lock().unwrap() = Some(req.clone());
            Ok(self.tune.clone().unwrap_or_default())
        }
    }

    pub(crate) fn setup() -> Arc<Mutex<Connection>> {
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
                vocabulary: vec!["brass".into(), "dark".into()],
            },
        )
        .unwrap();
        Arc::new(Mutex::new(conn))
    }

    /// Adds an entry at `Z:\media\audio\<name>.opus` with hash `h-<name>`.
    pub(crate) fn add(db: &Mutex<Connection>, name: &str, entry_type: &str) -> String {
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

    /// Returns (auto_score, user_vote) of the entry's row for `tag`, if any.
    fn row_of(db: &Mutex<Connection>, entry: &str, tag: &str) -> Option<(Option<f64>, Option<i64>)> {
        db.lock()
            .unwrap()
            .query_row(
                "SELECT lt.auto_score, lt.user_vote FROM library_entry_tags lt
                 JOIN tags t ON t.id = lt.tag_id
                 WHERE lt.library_entry_id = ?1 AND t.name = ?2",
                params![entry, tag],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    fn set_user_role(db: &Mutex<Connection>, entry: &str, role: Option<Role>) {
        library::set_user_role(&db.lock().unwrap(), &[entry.to_string()], role).unwrap();
    }

    /// A facet as stored: (auto value, auto score, user value).
    type FacetRow = (Option<String>, Option<f64>, Option<String>);

    /// Returns the entry's (role_auto, role_auto_score, role_user).
    fn role_of(db: &Mutex<Connection>, entry: &str) -> FacetRow {
        db.lock()
            .unwrap()
            .query_row(
                "SELECT role_auto, role_auto_score, role_user FROM library_entries WHERE id = ?1",
                params![entry],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    fn auto_combat(score: f64) -> FacetRow {
        (Some("combat".into()), Some(score), None)
    }

    fn no_role() -> FacetRow {
        (None, None, None)
    }

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
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
        let c = add(&db, "c", "music");
        set_user_role(&db, &a, Some(Role::NonCombat));
        let fake = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9), ("h-b", 0.8), ("h-c", 0.1)]),
            tags: HashMap::from([("brass".to_string(), scores(&[("h-a", 0.7), ("h-b", 0.9), ("h-c", 0.2)]))]),
            ..Default::default()
        };

        let report = run_once(&db, &fake, &[a.clone(), b.clone(), c.clone()], &no_progress);

        assert_eq!(report.error, None);
        assert_eq!(report.analyzed, 3);
        assert_eq!(report.added, 2);
        assert_eq!(report.removed, 0);
        assert!(report.finished_at.is_some());
        assert_eq!(report.gate.as_ref().unwrap().tag, "combat");
        // Every entry gets an analyzer role scored with p(Combat), including
        // the one the user set; the user's role stays beside it.
        assert_eq!(role_of(&db, &a), (s("combat"), Some(0.9), s("non_combat")));
        assert_eq!(role_of(&db, &b), auto_combat(0.8));
        assert_eq!(role_of(&db, &c), (s("non_combat"), Some(0.1), None));
        assert_eq!(row_of(&db, &a, "brass"), Some((Some(0.7), None)));
        assert_eq!(row_of(&db, &b, "brass"), Some((Some(0.9), None)));
        assert_eq!(row_of(&db, &c, "brass"), None);
        assert_eq!(status_of(&db, &a), Some((AnalysisStatus::Embedded, None)));

        let saved = autotag_db::load_scores(&db.lock().unwrap(), &b).unwrap().unwrap();
        assert_eq!(saved.gate.tag, "combat");
        assert_eq!(saved.gate.applied, Some(TagSource::Auto), "gate applied follows the role layer");
        assert_eq!(saved.tags[0].tag, "brass");
    }

    #[test]
    fn run_rescores_embedded_entries_without_embedding_again() {
        let db = setup();
        let a = add(&db, "a", "music");
        let fake = FakeAnalyzer { gate: scores(&[("h-a", 0.9)]), ..Default::default() };
        run_once(&db, &fake, std::slice::from_ref(&a), &no_progress);
        assert_eq!(fake.embedded_hashes(), vec!["h-a"]);

        let report = run_once(&db, &fake, &[], &no_progress);
        assert_eq!(fake.embedded_hashes(), vec!["h-a"], "no second /embed");
        assert_eq!(report.analyzed, 1);
        assert_eq!(role_of(&db, &a), auto_combat(0.9));
    }

    #[test]
    fn run_with_unreachable_analyzer_keeps_existing_auto_tags() {
        let db = setup();
        let a = add(&db, "a", "music");
        let ok = FakeAnalyzer { gate: scores(&[("h-a", 0.9)]), ..Default::default() };
        run_once(&db, &ok, std::slice::from_ref(&a), &no_progress);
        assert_eq!(role_of(&db, &a), auto_combat(0.9));

        let down = FakeAnalyzer { down: Some("connection refused".into()), ..Default::default() };
        let phases = Mutex::new(Vec::new());
        let report = run_once(&db, &down, std::slice::from_ref(&a), &|p: AutoTagProgress| {
            phases.lock().unwrap().push(p.phase)
        });

        assert!(report.error.as_deref().unwrap().contains("connection refused"));
        assert_eq!(phases.lock().unwrap().last().map(String::as_str), Some("error"));
        assert_eq!(role_of(&db, &a), auto_combat(0.9));
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

        let report = run_once(&db, &fake, std::slice::from_ref(&outside), &no_progress);

        assert_eq!(report.unreachable, 1);
        assert_eq!(report.error, None);
        assert_eq!(report.sample_errors.len(), 1);
        assert!(report.sample_errors[0].starts_with("x: D:\\other\\x.opus is not under the local prefix"));
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
        assert_eq!(report.sample_errors, vec!["b: decode failed".to_string()]);
        assert_eq!(
            status_of(&db, &b),
            Some((AnalysisStatus::Failed, Some("decode failed".to_string())))
        );
        assert_eq!(role_of(&db, &a), auto_combat(0.9));
        assert_eq!(role_of(&db, &b), no_role());
    }

    #[test]
    fn sample_errors_stop_at_five() {
        let db = setup();
        let names = ["a", "b", "c", "d", "e", "f", "g"];
        let ids: Vec<String> = names.iter().map(|n| add(&db, n, "music")).collect();
        let fake = FakeAnalyzer {
            embed_failures: names.iter().map(|n| (format!("h-{n}"), "decode failed".to_string())).collect(),
            ..Default::default()
        };

        let report = run_once(&db, &fake, &ids, &no_progress);

        assert_eq!(report.failed, 7);
        assert_eq!(report.sample_errors.len(), 5);
    }

    #[test]
    fn failed_report_keeps_previous_tag_quality() {
        let db = setup();
        let a = add(&db, "a", "music");
        let ok = FakeAnalyzer { gate: scores(&[("h-a", 0.9)]), ..Default::default() };
        let good = run_once(&db, &ok, std::slice::from_ref(&a), &no_progress);
        assert!(good.gate.is_some());

        let down = FakeAnalyzer { down: Some("connection refused".into()), ..Default::default() };
        let mut failed = run_once(&db, &down, std::slice::from_ref(&a), &no_progress);
        assert_eq!(failed.gate, None);
        keep_previous_quality(&good, &mut failed);
        assert_eq!(failed.gate, good.gate);
        assert_eq!(failed.tags, good.tags);
        assert!(failed.error.is_some());

        // A successful report keeps its own figures.
        let mut next = run_once(&db, &ok, &[], &no_progress);
        let before = next.clone();
        keep_previous_quality(&failed, &mut next);
        assert_eq!(next, before);
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
        assert_eq!(role_of(&db, &amb), no_role());
    }

    #[test]
    fn user_tag_added_during_scoring_keeps_its_vote() {
        let db = setup();
        let a = add(&db, "a", "music");
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

        let report = run_once(&db, &fake, std::slice::from_ref(&a), &no_progress);

        assert_eq!(report.error, None);
        // The analyzer's score lands beside the user's yes vote.
        assert_eq!(row_of(&db, &a, "brass"), Some((Some(0.9), Some(1))));
        assert_eq!(report.added, 0, "the entry already showed the tag");
    }

    #[test]
    fn run_scores_tags_the_user_voted_on() {
        let db = setup();
        let a = add(&db, "a", "music");
        {
            let conn = db.lock().unwrap();
            library::add_tag_to_entry(&conn, &a, "brass").unwrap();
            library::add_tag_to_entry(&conn, &a, "dark").unwrap();
            library::set_entry_tags(&conn, &a, &["brass".to_string()], None).unwrap();
        }
        let fake = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9)]),
            tags: HashMap::from([
                ("brass".to_string(), scores(&[("h-a", 0.8)])),
                ("dark".to_string(), scores(&[("h-a", 0.7)])),
            ]),
            ..Default::default()
        };

        let report = run_once(&db, &fake, std::slice::from_ref(&a), &no_progress);

        assert_eq!(report.error, None);
        assert_eq!(row_of(&db, &a, "brass"), Some((Some(0.8), Some(1))));
        assert_eq!(row_of(&db, &a, "dark"), Some((Some(0.7), Some(0))));
        // Neither changed what the entry shows.
        assert_eq!((report.added, report.removed), (0, 0));
        // The yes and no votes are what the analyzer trains on.
        let request = fake.last_score.lock().unwrap().clone().unwrap();
        assert_eq!(request.tags["brass"].positives, vec!["h-a".to_string()]);
        assert_eq!(request.tags["dark"].negatives, vec!["h-a".to_string()]);
    }

    #[test]
    fn run_skips_entry_that_stopped_being_music_during_scoring() {
        let db = setup();
        let a = add(&db, "a", "music");
        let hook_db = db.clone();
        let hook_entry = a.clone();
        let fake = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9)]),
            tags: HashMap::from([("brass".to_string(), scores(&[("h-a", 0.9)]))]),
            on_score: Some(Box::new(move || {
                let conn = hook_db.try_lock().expect("database is free during /score");
                library::update_library_entry_type(&conn, &hook_entry, "ambiance").unwrap();
            })),
            ..Default::default()
        };

        let report = run_once(&db, &fake, std::slice::from_ref(&a), &no_progress);

        assert_eq!(report.error, None);
        assert_eq!(row_of(&db, &a, "brass"), None);
        assert_eq!(role_of(&db, &a), no_role());
        assert_eq!(report.added, 0);
    }

    #[test]
    fn run_does_not_overwrite_role_set_during_scoring() {
        let db = setup();
        let a = add(&db, "a", "music");
        let hook_db = db.clone();
        let hook_a = a.clone();
        let fake = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9)]),
            on_score: Some(Box::new(move || {
                let conn = hook_db.try_lock().expect("database is free during /score");
                library::set_user_role(&conn, std::slice::from_ref(&hook_a), Some(Role::NonCombat)).unwrap();
            })),
            ..Default::default()
        };

        let report = run_once(&db, &fake, std::slice::from_ref(&a), &no_progress);

        assert_eq!(report.error, None);
        assert_eq!(role_of(&db, &a), (s("combat"), Some(0.9), s("non_combat")));
    }

    /// Raw mood scores: `heroic` and `wondrous` as given, every other mood
    /// and energy at `other`.
    fn raw_mood(heroic: f64, wondrous: f64, other: f64) -> MoodRaw {
        let mut moods: HashMap<String, f64> =
            MoodVocabulary::defaults().moods.into_iter().map(|m| (m.id, other)).collect();
        moods.insert("heroic".into(), heroic);
        moods.insert("wondrous".into(), wondrous);
        MoodRaw { moods, energy: other }
    }

    /// The entry's stored (auto_score, user_level) of one mood, `None`
    /// without a row.
    fn mood_of(db: &Mutex<Connection>, entry: &str, mood: &str) -> Option<(Option<f64>, Option<f64>)> {
        db.lock()
            .unwrap()
            .query_row(
                "SELECT auto_score, user_level FROM library_entry_moods WHERE library_entry_id = ?1 AND mood = ?2",
                params![entry, mood],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .unwrap()
    }

    fn mood_row_count(db: &Mutex<Connection>, entry: &str) -> i64 {
        db.lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM library_entry_moods WHERE library_entry_id = ?1", params![entry], |r| {
                r.get(0)
            })
            .unwrap()
    }

    /// `n` music entries "e0".. with hashes "h-e0"..; every fourth has heroic
    /// raised and the one after it wondrous, while every other id alternates
    /// between two values, so every id has spread.
    fn mood_library(db: &Mutex<Connection>, n: usize) -> (Vec<String>, HashMap<String, f64>, HashMap<String, MoodRaw>) {
        let mut ids = Vec::new();
        let (mut gate, mut mood) = (HashMap::new(), HashMap::new());
        for k in 0..n {
            ids.push(add(db, &format!("e{k}"), "music"));
            let hash = format!("h-e{k}");
            gate.insert(hash.clone(), 0.9);
            let heroic = if k % 4 == 0 { 0.3 } else { 0.2 };
            let wondrous = if k % 4 == 1 { 0.3 } else { 0.2 };
            mood.insert(hash, raw_mood(heroic, wondrous, 0.2 + (k % 2) as f64 * 0.01));
        }
        (ids, gate, mood)
    }

    #[test]
    fn run_takes_a_frozen_mood_calibration_and_scores_moods() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let fake = FakeAnalyzer { gate, mood, ..Default::default() };

        let report = run_once(&db, &fake, &ids, &no_progress);
        assert_eq!(report.error, None);

        let cal = settings::load_mood_calibration(&db.lock().unwrap()).unwrap().unwrap();
        assert_eq!(cal.stats.len(), 10);
        assert_eq!(cal.tracks, plan::CALIBRATION_MIN_TRACKS);
        // heroic: 13 of 50 at 0.3, the rest at 0.2.
        let heroic = cal.stats["heroic"];
        assert!((heroic.mean - 0.226).abs() < 1e-12);
        assert!((heroic.sd - 0.1 * (0.26f64 * 0.74).sqrt()).abs() < 1e-12);
        // Every id gets an analyzer value and no user level.
        assert_eq!(mood_row_count(&db, &ids[0]), 10);
        let (auto, user) = mood_of(&db, &ids[0], "heroic").unwrap();
        assert!((auto.unwrap() - plan::mood_score("heroic", 0.3, heroic, plan::CalibrationSource::Library)).abs() < 1e-12);
        // z 1.69 for heroic, centred half an sd up: clearly in.
        assert!(auto.unwrap() > 0.8);
        assert_eq!(user, None);
        assert!(mood_of(&db, &ids[1], "heroic").unwrap().0.unwrap() < 0.5);
        assert!(mood_of(&db, &ids[1], "wondrous").unwrap().0.unwrap() > 0.7);

        // A later run with a different spread keeps the frozen calibration.
        let mood: HashMap<String, MoodRaw> = (0..plan::CALIBRATION_MIN_TRACKS)
            .map(|k| (format!("h-e{k}"), raw_mood(0.9, 0.1 + k as f64 * 0.001, 0.2)))
            .collect();
        let gate = mood.keys().map(|h| (h.clone(), 0.9)).collect();
        let fake = FakeAnalyzer { gate, mood, ..Default::default() };
        run_once(&db, &fake, &[], &no_progress);
        assert_eq!(settings::load_mood_calibration(&db.lock().unwrap()).unwrap().unwrap(), cal);
        // Scored against the frozen calibration, heroic is near the top.
        let (auto, _) = mood_of(&db, &ids[1], "heroic").unwrap();
        assert!((auto.unwrap() - plan::mood_score("heroic", 0.9, heroic, plan::CalibrationSource::Library)).abs() < 1e-12);
    }

    /// Reference statistics for every id of the default vocabulary (Energy
    /// included), all with the same mean and an sd of 0.1.
    fn reference(set: &str, mean: f64) -> plan::MoodReference {
        plan::MoodReference {
            set: set.into(),
            tracks: 300,
            stats: MoodVocabulary::defaults()
                .ids()
                .into_iter()
                .map(|id| (id, plan::MoodStat { mean, sd: 0.1 }))
                .collect(),
        }
    }

    fn stored_calibration(db: &Mutex<Connection>) -> plan::MoodCalibration {
        settings::load_mood_calibration(&db.lock().unwrap()).unwrap().unwrap()
    }

    #[test]
    fn run_replaces_a_library_calibration_with_the_reference() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let fake = FakeAnalyzer { gate: gate.clone(), mood: mood.clone(), ..Default::default() };
        run_once(&db, &fake, &ids, &no_progress);
        assert_eq!(stored_calibration(&db).source, plan::CalibrationSource::Library);

        let fake = FakeAnalyzer { gate, mood, mood_reference: Some(reference("set-1", 0.2)), ..Default::default() };
        let report = run_once(&db, &fake, &[], &no_progress);
        assert_eq!(report.error, None);
        let cal = stored_calibration(&db);
        assert_eq!(cal.source, plan::CalibrationSource::Reference);
        assert_eq!(cal.reference_set.as_deref(), Some("set-1"));
        assert_eq!(cal.tracks, 300);
        // e0 has heroic raw 0.3; scored with the reference mapping.
        let (auto, _) = mood_of(&db, &ids[0], "heroic").unwrap();
        let expected = plan::mood_score("heroic", 0.3, cal.stats["heroic"], plan::CalibrationSource::Reference);
        assert!((auto.unwrap() - expected).abs() < 1e-12);
    }

    #[test]
    fn run_takes_the_reference_even_with_too_little_music_for_a_library_calibration() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, 5);
        let fake = FakeAnalyzer { gate, mood, mood_reference: Some(reference("set-1", 0.2)), ..Default::default() };
        run_once(&db, &fake, &ids, &no_progress);
        assert_eq!(stored_calibration(&db).source, plan::CalibrationSource::Reference);
        assert_eq!(mood_row_count(&db, &ids[0]), 10);
    }

    #[test]
    fn run_keeps_a_stored_reference_calibration_when_the_analyzer_sends_none() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let fake = FakeAnalyzer {
            gate: gate.clone(),
            mood: mood.clone(),
            mood_reference: Some(reference("set-1", 0.2)),
            ..Default::default()
        };
        run_once(&db, &fake, &ids, &no_progress);
        let cal = stored_calibration(&db);

        // An older analyzer (no reference) must not swap in a library calibration.
        let bare = FakeAnalyzer { gate, mood, ..Default::default() };
        let report = run_once(&db, &bare, &[], &no_progress);
        assert_eq!(report.error, None);
        assert_eq!(stored_calibration(&db), cal);
        let (auto, _) = mood_of(&db, &ids[0], "heroic").unwrap();
        let expected = plan::mood_score("heroic", 0.3, cal.stats["heroic"], plan::CalibrationSource::Reference);
        assert!((auto.unwrap() - expected).abs() < 1e-12);
    }

    #[test]
    fn run_rescores_edited_prompts_against_the_reference() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let fake = FakeAnalyzer {
            gate: gate.clone(),
            mood: mood.clone(),
            mood_reference: Some(reference("set-1", 0.2)),
            ..Default::default()
        };
        run_once(&db, &fake, &ids, &no_progress);

        let mut edited = MoodVocabulary::defaults();
        edited.moods[7].prompts = vec!["fanfare".into()];
        library::save_mood_vocabulary(&db.lock().unwrap(), &edited).unwrap();
        // The analyzer now computed the reference for the edited prompts.
        let fake = FakeAnalyzer { gate, mood, mood_reference: Some(reference("set-1", 0.25)), ..Default::default() };
        run_once(&db, &fake, &[], &no_progress);

        let cal = stored_calibration(&db);
        assert_eq!(cal.prompts["heroic"], ["fanfare"]);
        assert_eq!(cal.stats["heroic"].mean, 0.25);
        let (auto, _) = mood_of(&db, &ids[0], "heroic").unwrap();
        let expected = plan::mood_score("heroic", 0.3, cal.stats["heroic"], plan::CalibrationSource::Reference);
        assert!((auto.unwrap() - expected).abs() < 1e-12);
        assert!(!plan::needs_recalibration(Some(&cal), &edited.normalized().unwrap()));
    }

    #[test]
    fn run_with_an_unchanged_reference_does_not_rewrite_the_calibration() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let fake = FakeAnalyzer { gate, mood, mood_reference: Some(reference("set-1", 0.2)), ..Default::default() };
        run_once(&db, &fake, &ids, &no_progress);
        let first = stored_calibration(&db);
        std::thread::sleep(std::time::Duration::from_millis(5));
        run_once(&db, &fake, &[], &no_progress);
        assert_eq!(stored_calibration(&db).calibrated_at, first.calibrated_at);
    }

    #[test]
    fn run_without_enough_music_scores_no_moods() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS - 1);
        let fake = FakeAnalyzer { gate, mood, ..Default::default() };

        let report = run_once(&db, &fake, &ids, &no_progress);
        assert_eq!(report.error, None);
        assert_eq!(settings::load_mood_calibration(&db.lock().unwrap()).unwrap(), None);
        assert!(ids.iter().all(|id| mood_row_count(&db, id) == 0));
    }

    #[test]
    fn run_never_touches_the_users_levels() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        library::set_user_mood_level(&db.lock().unwrap(), &ids[..1], "grim", Some(1.0)).unwrap();
        let reviewed = |db: &Mutex<Connection>| -> Option<String> {
            db.lock()
                .unwrap()
                .query_row("SELECT mood_reviewed_at FROM library_entries WHERE id = ?1", params![ids[0]], |r| r.get(0))
                .unwrap()
        };
        let before = reviewed(&db);
        assert!(before.is_some());
        let fake = FakeAnalyzer { gate, mood, ..Default::default() };
        run_once(&db, &fake, &ids, &no_progress);
        assert_eq!(reviewed(&db), before);
        let (auto, user) = mood_of(&db, &ids[0], "grim").unwrap();
        assert!(auto.is_some());
        assert_eq!(user, Some(1.0));
    }

    #[test]
    fn run_keeps_moods_without_mood() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let fake = FakeAnalyzer { gate: gate.clone(), mood, ..Default::default() };
        run_once(&db, &fake, &ids, &no_progress);
        let before = mood_of(&db, &ids[0], "heroic");
        assert!(before.unwrap().0.is_some());

        // No mood in the response (model not loaded): the values stay.
        let bare = FakeAnalyzer { gate, ..Default::default() };
        let report = run_once(&db, &bare, &[], &no_progress);
        assert_eq!(report.error, None);
        assert_eq!(mood_of(&db, &ids[0], "heroic"), before);
        assert_eq!(mood_row_count(&db, &ids[0]), 10);
    }

    #[test]
    fn run_sends_the_vocabulary_and_skips_moods_with_changed_prompts() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let fake = FakeAnalyzer { gate: gate.clone(), mood: mood.clone(), ..Default::default() };
        run_once(&db, &fake, &ids, &no_progress);
        let request = fake.last_score.lock().unwrap().clone().unwrap();
        let vocab = MoodVocabulary::defaults();
        assert_eq!(request.mood_prompts.unwrap()["grim"], vocab.moods[6].prompts);
        assert_eq!(request.energy_prompts, Some(vocab.energy.clone()));
        let heroic_before = mood_of(&db, &ids[1], "heroic");
        let grim_before = mood_of(&db, &ids[1], "grim");

        // Heroic's prompts change; the next run adds Heroic's statistics
        // from the library and rescores it, and the others keep theirs.
        let mut edited = vocab.clone();
        edited.moods[7].prompts = vec!["fanfare".into()];
        library::save_mood_vocabulary(&db.lock().unwrap(), &edited).unwrap();
        let mood: HashMap<String, MoodRaw> = mood
            .keys()
            .enumerate()
            .map(|(k, h)| (h.clone(), raw_mood(0.5 + (k % 2) as f64 * 0.1, 0.5, 0.5)))
            .collect();
        let fake = FakeAnalyzer { gate, mood, ..Default::default() };
        run_once(&db, &fake, &[], &no_progress);
        let sent = fake.last_score.lock().unwrap().clone().unwrap();
        assert_eq!(sent.mood_prompts.unwrap()["heroic"], ["fanfare"]);
        assert_ne!(mood_of(&db, &ids[1], "heroic"), heroic_before);
        assert_ne!(mood_of(&db, &ids[1], "grim"), grim_before);
        let cal = stored_calibration(&db);
        assert_eq!(cal.prompts["heroic"], ["fanfare"]);
        assert!(!plan::needs_recalibration(Some(&cal), &edited.normalized().unwrap()));
    }

    #[test]
    fn run_writes_no_moods_when_recalibrate_ran_meanwhile() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let hook_db = db.clone();
        let fake = FakeAnalyzer {
            gate,
            mood,
            on_score: Some(Box::new(move || {
                library::clear_mood_analysis(&hook_db.try_lock().expect("database is free during /score")).unwrap();
            })),
            ..Default::default()
        };
        let report = run_once(&db, &fake, &ids, &no_progress);
        assert_eq!(report.error, None);
        // Calibration-free at start and cleared meanwhile still compares
        // equal (both none), so this run takes the calibration as usual.
        assert!(settings::load_mood_calibration(&db.lock().unwrap()).unwrap().is_some());

        // With a calibration stored, a Recalibrate during /score wins: the
        // run neither writes moods nor restores a calibration.
        let fake = FakeAnalyzer {
            gate: fake.gate.clone(),
            mood: fake.mood.clone(),
            on_score: fake.on_score,
            ..Default::default()
        };
        run_once(&db, &fake, &[], &no_progress);
        assert_eq!(settings::load_mood_calibration(&db.lock().unwrap()).unwrap(), None);
        assert!(ids.iter().all(|id| mood_row_count(&db, id) == 0));
    }

    #[test]
    fn recalibrate_with_new_prompts_during_an_uncalibrated_run_is_not_lost() {
        let db = setup();
        let (ids, gate, mood) = mood_library(&db, plan::CALIBRATION_MIN_TRACKS);
        let mut edited = MoodVocabulary::defaults();
        edited.moods[7].prompts = vec!["fanfare".into()];
        // The run starts with no calibration; during /score the user saves
        // new prompts and recalibrates (a no-op on the calibration).
        let hook_db = db.clone();
        let hook_vocab = edited.clone();
        let fake = FakeAnalyzer {
            gate: gate.clone(),
            mood: mood.clone(),
            on_score: Some(Box::new(move || {
                let conn = hook_db.try_lock().expect("database is free during /score");
                library::save_mood_vocabulary(&conn, &hook_vocab).unwrap();
                library::clear_mood_analysis(&conn).unwrap();
            })),
            ..Default::default()
        };
        let report = run_once(&db, &fake, &ids, &no_progress);
        assert_eq!(report.error, None);
        // Scored with the old prompts: no calibration and no moods saved.
        assert_eq!(settings::load_mood_calibration(&db.lock().unwrap()).unwrap(), None);
        assert!(ids.iter().all(|id| mood_row_count(&db, id) == 0));

        // The next run calibrates with the new prompts; nothing needs
        // recalibrating.
        let fake = FakeAnalyzer { gate, mood, ..Default::default() };
        run_once(&db, &fake, &[], &no_progress);
        let cal = settings::load_mood_calibration(&db.lock().unwrap()).unwrap().unwrap();
        assert_eq!(cal.prompts["heroic"], ["fanfare"]);
        assert!(!plan::needs_recalibration(Some(&cal), &edited.normalized().unwrap()));
        assert_eq!(mood_row_count(&db, &ids[0]), 10);
    }

    #[test]
    fn run_sends_no_pace_examples_to_analyzer() {
        let db = setup();
        let a = add(&db, "a", "music");
        let fake = FakeAnalyzer::default();

        run_once(&db, &fake, &[a], &no_progress);

        let request = fake.last_score.lock().unwrap().clone().expect("score was called");
        assert!(serde_json::to_value(&request).unwrap().get("pace").is_none());
    }

    #[test]
    fn old_analyzer_without_mood_profile_still_works() {
        // An analyzer from before pace and mood sends neither key; one from
        // v0.12 sends a pace head and mood with only `bright` and `light`;
        // one from before pools sends only the three scales; one from v0.15
        // sends pools.
        let gate = r#""gate":{"quality":{"auc":0.9,"pos":3,"neg":3},
            "trusted":true,"threshold":0.5,"neg_threshold":0.5,"scores":{"h-a":0.9}},"tags":{}"#;
        let bare = format!(r#"{{"model":"m",{gate}}}"#);
        let two_axis = format!(
            r#"{{"model":"m",{gate},
                "pace":{{"quality":{{"auc":null,"pos":0,"neg":0}},"trusted":false,"threshold":null,"scores":{{}}}},
                "mood":{{"h-a":{{"bright":0.3,"light":0.1}}}}}}"#
        );
        let scales_only = format!(
            r#"{{"model":"m",{gate},"mood":{{"h-a":{{"bright":0.3,"grand":0.1,"energy":-0.2}}}}}}"#
        );

        struct Canned(ScoreResponse, FakeAnalyzer);
        impl AnalyzerClient for Canned {
            fn health(&self) -> Result<String, String> { self.1.health() }
            fn embed(&self, items: &[(String, String)]) -> Result<Vec<EmbedResult>, String> { self.1.embed(items) }
            fn score(&self, _: &ScoreRequest) -> Result<ScoreResponse, String> { Ok(self.0.clone()) }
            fn similar(&self, q: &[String], c: &[String], l: u32) -> Result<Vec<HashScore>, String> { self.1.similar(q, c, l) }
            fn search(&self, t: &str, c: &[String], l: u32) -> Result<Vec<HashScore>, String> { self.1.search(t, c, l) }
            fn map(&self, h: &[String]) -> Result<Vec<HashPoint>, String> { self.1.map(h) }
            fn preview(&self, p: &[String], t: &[String]) -> Result<HashMap<String, f64>, String> { self.1.preview(p, t) }
            fn tune(&self, r: &super::tune::TuneRequest) -> Result<super::tune::TuneResponse, String> { self.1.tune(r) }
        }

        let pools = format!(
            r#"{{"model":"m",{gate},"mood":{{"h-a":{{"pools":{{"peaceful":0.3,"heroic":0.1,"awe":0.2}}}}}}}}"#
        );

        for json in [bare, two_axis, scales_only, pools] {
            let response: ScoreResponse = serde_json::from_str(&json).unwrap();
            let db = setup();
            let a = add(&db, "a", "music");
            let client = Canned(response, FakeAnalyzer::default());
            let report = run_once(&db, &client, std::slice::from_ref(&a), &no_progress);

            assert_eq!(report.error, None);
            assert_eq!(report.analyzed, 1);
            assert_eq!(role_of(&db, &a), auto_combat(0.9));
            assert_eq!(mood_row_count(&db, &a), 0);
        }
    }

    #[test]
    fn run_clears_auto_role_when_gate_no_longer_qualifies() {
        let db = setup();
        let a = add(&db, "a", "music");
        let first = FakeAnalyzer { gate: scores(&[("h-a", 0.9)]), ..Default::default() };
        run_once(&db, &first, std::slice::from_ref(&a), &no_progress);
        assert_eq!(role_of(&db, &a), auto_combat(0.9));

        let untrusted = FakeAnalyzer {
            gate: scores(&[("h-a", 0.9)]),
            untrusted_gate: true,
            ..Default::default()
        };
        run_once(&db, &untrusted, &[], &no_progress);
        assert_eq!(role_of(&db, &a), no_role());
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

    #[test]
    fn manual_analyze_queues_embedded_entries_too() {
        let db = setup();
        let fresh = add(&db, "fresh", "music");
        let done = add(&db, "done", "music");
        add(&db, "rain", "ambiance");
        {
            let conn = db.lock().unwrap();
            autotag_db::set_analysis(&conn, &done, Some("h-done"), AnalysisStatus::Embedded, None).unwrap();
        }
        let ids: HashSet<String> = all_music_entries(&db.lock().unwrap()).unwrap().into_iter().collect();
        assert_eq!(ids, HashSet::from([fresh, done]));
    }
}
