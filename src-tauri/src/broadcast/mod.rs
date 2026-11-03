//! Live broadcast of the program bus to remote listeners.
//!
//! Two pieces, each on its own OS thread:
//! - the **encoder thread** repeatedly asks the Ogg/Opus [`Encoder`] for
//!   finished pages and publishes each batch on a broadcast channel;
//! - the **listener server** ([`server::ListenerServer`]) hands every
//!   connected listener the header pages, then whatever arrives on that
//!   channel.
//!
//! The encoder never waits for listeners: a slow one falls behind and is cut
//! off (see `server.rs` on "lag"), it does not slow anyone else down.

mod server;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::broadcast;

use crate::audio::ffi::Encoder;
use server::{ListenerServer, PageSource};

/// How many page batches the channel keeps for listeners that are briefly
/// behind. Anyone further back than this has lagged and is dropped.
const PAGE_CAPACITY: usize = 256;

/// How long the encoder thread naps when the encoder has nothing ready. Opus
/// pages arrive every few tens of milliseconds, so 10 ms adds no audible
/// delay and keeps `pump` (which allocates a 64 KiB scratch per call) from
/// spinning.
const IDLE_SLEEP: Duration = Duration::from_millis(10);

/// A running broadcast: the encoder thread plus the listener server.
/// Stop it with [`Broadcaster::stop`] or simply drop it; either way the
/// encoder thread is joined (which drops the [`Encoder`] and disables the
/// engine's tap) before the listener server shuts down.
///
/// Dropping always joins the encoder thread because that thread holds a raw
/// pointer to the audio engine: `AudioEngine` owns the `Broadcaster` and
/// declares it before its `engine` field, so this join finishes before the
/// engine is destroyed. Without it, dropping the engine while the thread was
/// still pumping would be a use-after-free.
pub struct Broadcaster {
    stop_flag: Arc<AtomicBool>,
    /// `Option` only so `Drop` can take the handle out and join it.
    encoder_thread: Option<JoinHandle<()>>,
    /// `Option` only so `Drop` can take the server out and stop it.
    server: Option<ListenerServer>,
}

impl Broadcaster {
    /// Start broadcasting `encoder`'s output on `0.0.0.0:port`, reachable at
    /// `/listen/{key}`.
    ///
    /// The server starts first. If the port is busy we return its error
    /// before any encoder thread exists; `encoder` is dropped right here,
    /// which disables the tap, so a failed start leaves nothing running.
    ///
    /// The encoder runs on a plain OS thread rather than a tokio task
    /// because `pump` is blocking, CPU-bound work (Opus encoding through C)
    /// on a loop that sleeps between calls; on a task it would hold up the
    /// server's network work sharing that runtime.
    pub fn start(encoder: Encoder, port: u16, key: String) -> Result<Broadcaster, String> {
        let header = Arc::new(encoder.header());
        let (pages, _) = broadcast::channel::<Bytes>(PAGE_CAPACITY);
        let server = ListenerServer::start(port, key, PageSource { header, pages: pages.clone() })?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let thread_stop = stop_flag.clone();
        let spawned = std::thread::Builder::new()
            .name("broadcast-encoder".into())
            .spawn(move || encoder_loop(encoder, pages, thread_stop));
        let encoder_thread = match spawned {
            Ok(handle) => handle,
            Err(e) => {
                // The closure (and the encoder inside it) was dropped by the
                // failed spawn; shut the server down too.
                server.stop();
                return Err(format!("cannot start broadcast encoder thread: {e}"));
            }
        };

        Ok(Broadcaster { stop_flag, encoder_thread: Some(encoder_thread), server: Some(server) })
    }

    /// How many listeners are connected right now.
    pub fn listener_count(&self) -> usize {
        self.server.as_ref().map_or(0, |s| s.listener_count())
    }

    /// Stop broadcasting. Same as dropping it (see the `Drop` impl); this
    /// just names the intent at the call site.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Broadcaster {
    /// Stop the encoder thread and wait for it (it drops the encoder, which
    /// disables the tap), then shut down the listener server. The encoder
    /// goes first so nothing is still reading the engine once this returns.
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(thread) = self.encoder_thread.take() {
            let _ = thread.join();
        }
        if let Some(server) = self.server.take() {
            server.stop();
        }
    }
}

/// The encoder thread's body: pump finished pages, publish them, nap when
/// there is nothing ready, until `stop` is set. One reusable buffer avoids a
/// fresh allocation per batch. Publishing with no listeners connected fails
/// harmlessly (nobody to receive), so the error is ignored.
fn encoder_loop(mut encoder: Encoder, pages: broadcast::Sender<Bytes>, stop: Arc<AtomicBool>) {
    let mut out = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        out.clear();
        if encoder.pump(&mut out) > 0 {
            let _ = pages.send(Bytes::copy_from_slice(&out));
        } else {
            std::thread::sleep(IDLE_SLEEP);
        }
    }
    // Dropping the encoder here, on its own thread, disables the tap.
    drop(encoder);
}

#[cfg(test)]
mod tests {
    // Needs a real engine (an audio device), like the ffi tests.
    #[test]
    fn enable_fails_cleanly_when_port_busy() {
        let holder = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let mut audio = crate::audio::AudioEngine::new().unwrap();
        let err = audio.start_broadcast(port, "k".into()).err().expect("busy port must fail");
        assert!(err.contains(&port.to_string()));
        assert!(audio.broadcast_listeners().is_none(), "nothing left running after a failed start");
        drop(holder);
        audio.start_broadcast(port, "k".into()).expect("works once the port is free");
        assert_eq!(audio.broadcast_listeners(), Some(0));
        audio.stop_broadcast();
        assert!(audio.broadcast_listeners().is_none());
    }
}
