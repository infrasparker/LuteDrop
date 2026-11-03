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
/// Stop it with [`Broadcaster::stop`]; that also drops the [`Encoder`],
/// which disables the engine's tap.
// Task 6 wires this into the app; it removes this allow.
#[allow(dead_code)]
pub struct Broadcaster {
    stop_flag: Arc<AtomicBool>,
    encoder_thread: JoinHandle<()>,
    server: ListenerServer,
}

// Task 6 wires this into the app; it removes this allow.
#[allow(dead_code)]
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

        Ok(Broadcaster { stop_flag, encoder_thread, server })
    }

    /// How many listeners are connected right now.
    pub fn listener_count(&self) -> usize {
        self.server.listener_count()
    }

    /// Stop the encoder thread (which drops the encoder and disables the
    /// tap), then shut down the listener server.
    pub fn stop(self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let _ = self.encoder_thread.join();
        self.server.stop();
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
