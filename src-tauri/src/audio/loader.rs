/// What to do with a completion when it arrives at the poll tick.
#[derive(Debug, PartialEq, Eq)]
pub enum Routing {
    /// The pending entry's seq matches — keep the handle.
    Accept,
    /// No pending entry for this target (invalidated / target gone) or a newer
    /// request superseded it — drop the handle (safe: it just falls out of scope).
    DropStale,
}

/// Pure routing decision (D5). `pending_seq` is the seq currently recorded for
/// the completion's target, or `None` if there is no pending entry. Seqs are
/// globally monotonic, so a superseded or invalidated request never equals the
/// current entry's seq.
pub fn route_completion(pending_seq: Option<u64>, completion_seq: u64) -> Routing {
    match pending_seq {
        Some(seq) if seq == completion_seq => Routing::Accept,
        _ => Routing::DropStale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_completion_accepts_only_the_current_seq() {
        assert_eq!(route_completion(Some(5), 5), Routing::Accept);
        // A newer request bumped the entry to seq 6; the seq-5 completion is stale.
        assert_eq!(route_completion(Some(6), 5), Routing::DropStale);
        // Target was invalidated / removed entirely.
        assert_eq!(route_completion(None, 5), Routing::DropStale);
    }
}

use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

use super::ffi::EngineRef;
use super::sound::{self, PlayParams, SoundHandle};

/// Whether a preload is the next loop instance (ping-pong, D6) or the pre-picked
/// next clip of a transition (D7). Determines the poll-tick handoff.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PreloadKind {
    LoopInstance,
    NextClip,
}

/// A preload request sent to the loader thread.
pub struct LoadRequest {
    /// Target this preload belongs to: a track id, or `"__library"` for the
    /// preview voice.
    pub target: String,
    /// Monotonic token; the completion carries it back for stale detection.
    pub seq: u64,
    /// Everything needed to open + pre-seek the sound. `start_position`
    /// (already max'd with trim_start by the caller) is the seek point.
    pub params: PlayParams,
}

/// A finished preload returned on the completion channel.
pub struct LoadComplete {
    pub target: String,
    pub seq: u64,
    pub result: Result<SoundHandle, String>,
}

/// Per-target preload state held by `AudioEngine` (D5). At most one entry per
/// target; a new request overwrites it with a fresh seq.
pub struct PreloadTarget {
    pub seq: u64,
    pub kind: PreloadKind,
    /// The clip index this preload is for (NextClip only; None for LoopInstance).
    pub clip_index: Option<usize>,
    /// The params the sound was opened with — needed to rebuild loop-restart /
    /// know the intended volume at handoff.
    pub params: PlayParams,
    /// `Some` once the loader completes and the completion is accepted. For a
    /// LoopInstance this is handed straight to the Voice and never stored here;
    /// for a NextClip it waits here until the transition consumes it.
    pub ready: Option<SoundHandle>,
}

/// Owns the loader thread and its channels. Dropping it joins the thread (see
/// EngineRef safety (b)).
pub struct Loader {
    tx: Option<Sender<LoadRequest>>,
    /// Drained by `AudioEngine::poll` via `try_recv`.
    pub rx: Receiver<LoadComplete>,
    handle: Option<JoinHandle<()>>,
}

impl Loader {
    pub fn spawn(engine: EngineRef) -> Self {
        let (req_tx, req_rx) = std::sync::mpsc::channel::<LoadRequest>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<LoadComplete>();
        let handle = std::thread::Builder::new()
            .name("ld-audio-loader".to_string())
            .spawn(move || loader_main(engine, req_rx, done_tx))
            .expect("failed to spawn audio loader thread");
        Self { tx: Some(req_tx), rx: done_rx, handle: Some(handle) }
    }

    /// Queue a preload. A send failure means the loader thread is gone
    /// (shutting down); the completion simply never arrives.
    pub fn request(&self, req: LoadRequest) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(req);
        }
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        // Close the request channel so the loader thread's `recv()` returns Err
        // and it exits, THEN join — so no in-flight `ma_sound_init`/`uninit`
        // outlives the engine (EngineRef safety (b)).
        self.tx.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Loader thread body: open + pre-seek each requested sound, off the audio
/// mutex. Exits when the request channel closes (Loader dropped) or the
/// completion channel closes (engine torn down).
fn loader_main(engine: EngineRef, rx: Receiver<LoadRequest>, tx: Sender<LoadComplete>) {
    while let Ok(req) = rx.recv() {
        let result = sound::preload(&engine, &req.params);
        if tx.send(LoadComplete { target: req.target, seq: req.seq, result }).is_err() {
            break;
        }
    }
}
