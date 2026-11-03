//! The listener HTTP server: serves the live Ogg/Opus broadcast to anyone who
//! knows the key, plus a one-line HTML page that plays it.
//!
//! ## How a listener receives audio
//! `GET /listen/{key}` answers with a **chunked response**: instead of
//! announcing a total length up front, the server keeps the connection open
//! and writes the body in pieces as they are produced (HTTP/1.1
//! `Transfer-Encoding: chunked`). A browser `<audio>` element plays such a
//! response as an endless stream. The body is the encoder's two header pages
//! first (a decoder cannot start without them), then every Ogg page the
//! encoder produces from the moment the listener connected.
//!
//! ## Fan-out and "lag"
//! Pages reach listeners through a `tokio::sync::broadcast` channel holding
//! the last 256 batches (each batch is whatever one encoder pump produced,
//! about one page). Each listener has its own read position in that
//! channel. If a listener reads too slowly (a bad network, a stalled player)
//! and falls more than 256 batches behind, the channel has already overwritten
//! pages it never saw: that listener has **lagged**. We then end that
//! listener's response; the encoder never waits for anyone, and every other
//! listener keeps going. The dropped listener's player reconnects and starts
//! again from the live edge.

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use tokio::sync::{broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

/// Where the server gets its audio: the encoder's header pages (sent to every
/// new listener first) and the channel the encoder publishes each new batch
/// of Ogg pages on. Create the channel with capacity 256; the server only
/// ever subscribes to it.
pub struct PageSource {
    pub header: Arc<Vec<u8>>,
    pub pages: broadcast::Sender<Bytes>,
}

/// The listening page served at `/listen/{key}/page`. `{{KEY}}` is replaced
/// with the broadcast key; the page is only a native audio player pointed at
/// the stream.
const LISTEN_PAGE_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>LuteDrop</title></head><body><audio controls src=\"/listen/{{KEY}}\"></audio></body></html>";

/// Shared, read-only state every request handler sees.
#[derive(Clone)]
struct ServerState {
    key: Arc<str>,
    header: Arc<Vec<u8>>,
    pages: broadcast::Sender<Bytes>,
    count: Arc<AtomicUsize>,
}

/// Counts one connected listener for as long as it exists. It is moved into
/// the response body's stream, so it is dropped (and the count goes down)
/// exactly when that body is dropped: either the stream ended because the
/// listener lagged, or the client disconnected and the HTTP library
/// discarded the body.
struct ListenerGuard(Arc<AtomicUsize>);

impl ListenerGuard {
    fn new(count: Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        ListenerGuard(count)
    }
}

impl Drop for ListenerGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A running listener server: one OS thread running its own small tokio
/// runtime, so it starts and stops independently of the phone-remote server.
pub struct ListenerServer {
    count: Arc<AtomicUsize>,
    shutdown_tx: oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

impl ListenerServer {
    /// Bind `0.0.0.0:port` and start serving `key`'s stream from `source`.
    ///
    /// The bind happens right here, synchronously, before any thread is
    /// spawned: if the port is taken, the caller gets an `Err` naming the
    /// port and the OS error immediately, instead of a background thread
    /// failing quietly later. Only once the socket is ours does the server
    /// thread start.
    pub fn start(port: u16, key: String, source: PageSource) -> Result<ListenerServer, String> {
        let std_listener = std::net::TcpListener::bind(("0.0.0.0", port))
            .map_err(|e| format!("cannot listen on port {port}: {e}"))?;
        // tokio requires a non-blocking socket when adopting a std listener.
        std_listener
            .set_nonblocking(true)
            .map_err(|e| format!("cannot configure port {port}: {e}"))?;

        let count = Arc::new(AtomicUsize::new(0));
        let state = ServerState {
            key: Arc::from(key),
            header: source.header,
            pages: source.pages,
            count: count.clone(),
        };
        let router = Router::new()
            .route("/listen/{key}", get(stream))
            .route("/listen/{key}/page", get(page))
            .fallback(|| async { StatusCode::NOT_FOUND })
            .with_state(state);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("broadcast-server".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        log::error!("broadcast server: cannot build runtime: {e}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    // `from_std` needs a running runtime to register the
                    // socket with, hence here and not in `start` itself.
                    let listener = match tokio::net::TcpListener::from_std(std_listener) {
                        Ok(l) => l,
                        Err(e) => {
                            log::error!("broadcast server: cannot adopt socket: {e}");
                            return;
                        }
                    };
                    // Serve in a background task and wait here for the stop
                    // signal. Not `with_graceful_shutdown`: that waits for
                    // every open response to finish, and a listener's stream
                    // never finishes, so `stop` would hang while anyone is
                    // listening.
                    tokio::spawn(async move {
                        if let Err(e) = axum::serve(listener, router).await {
                            log::error!("broadcast server error: {e}");
                        }
                    });
                    let _ = shutdown_rx.await;
                });
                // Dropping the runtime cancels the serve task and every
                // connection task: sockets close, bodies (and their listener
                // guards) drop, and the port is released.
                drop(runtime);
            })
            .map_err(|e| format!("cannot start listener server thread: {e}"))?;

        Ok(ListenerServer { count, shutdown_tx, thread })
    }

    /// How many listeners are connected to the stream right now.
    pub fn listener_count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    /// Stop accepting connections, close the open streams, and wait for the
    /// server thread to finish. The port is free again once this returns.
    pub fn stop(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.thread.join();
    }
}

/// `GET /listen/{key}`: the audio stream itself (see the module docs).
async fn stream(Path(key): Path<String>, State(state): State<ServerState>) -> Response {
    if key != *state.key {
        return StatusCode::NOT_FOUND.into_response();
    }
    let guard = ListenerGuard::new(state.count.clone());
    let header = tokio_stream::once(Bytes::copy_from_slice(&state.header));
    // Subscribing now means this listener gets every page published from
    // this moment on. `map_while` ends the stream at the first `Lagged`
    // error, which ends this one response and nothing else.
    let pages = BroadcastStream::new(state.pages.subscribe()).map_while(|page| page.ok());
    let body = header.chain(pages).map(move |bytes| {
        // Naming the guard here moves it into this closure, so it lives
        // exactly as long as the body stream.
        let _alive = &guard;
        Ok::<Bytes, Infallible>(bytes)
    });
    (
        [(header::CONTENT_TYPE, "audio/ogg"), (header::CACHE_CONTROL, "no-store")],
        Body::from_stream(body),
    )
        .into_response()
}

/// `GET /listen/{key}/page`: a minimal HTML page that plays the stream.
async fn page(Path(key): Path<String>, State(state): State<ServerState>) -> Response {
    if key != *state.key {
        return StatusCode::NOT_FOUND.into_response();
    }
    Html(LISTEN_PAGE_HTML.replace("{{KEY}}", &key)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::time::Duration;

    fn fake_source() -> PageSource { PageSource { header: Arc::new(b"HDR!".to_vec()), pages: broadcast::channel(256).0 } }
    fn free_port() -> u16 { std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port() }

    /// Send a bare HTTP/1.1 GET request for `path` on an open connection.
    fn write_get(mut stream: &std::net::TcpStream, path: &str) {
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
    }

    /// Undo chunked transfer encoding on whatever has arrived so far.
    /// A chunked body is a series of `<hex size>\r\n<payload>\r\n` pieces; we
    /// read each size line and keep the payload. An incomplete trailing chunk
    /// contributes the bytes that did arrive, which is all the tests need.
    fn dechunk(mut raw: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let Some(eol) = raw.windows(2).position(|w| w == b"\r\n") else { break };
            let size_text = String::from_utf8_lossy(&raw[..eol]).to_string();
            // Chunk extensions (";name=value") are legal; ignore them.
            let size_hex = size_text.split(';').next().unwrap_or("").trim();
            let Ok(size) = usize::from_str_radix(size_hex, 16) else { break };
            if size == 0 {
                break; // the terminating chunk: body is complete
            }
            raw = &raw[eol + 2..];
            let take = size.min(raw.len());
            out.extend_from_slice(&raw[..take]);
            if raw.len() < size + 2 {
                break; // this chunk (or its trailing CRLF) is still in flight
            }
            raw = &raw[size + 2..];
        }
        out
    }

    /// Split a raw response into (status, header text, body-so-far), with the
    /// body de-chunked when the server used `Transfer-Encoding: chunked`.
    /// `None` until the blank line ending the header block has arrived.
    fn parse_response(raw: &[u8]) -> Option<(u16, String, Vec<u8>)> {
        let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
        let headers = String::from_utf8_lossy(&raw[..end]).to_string();
        let rest = &raw[end + 4..];
        let chunked = headers.to_ascii_lowercase().contains("transfer-encoding: chunked");
        let body = if chunked { dechunk(rest) } else { rest.to_vec() };
        let status = headers.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        Some((status, headers, body))
    }

    /// Connect, GET `path`, and read until the response headers are complete
    /// and at least `n` body bytes have arrived, or until the connection
    /// closes or the 2 s read timeout fires. Returns (status code, raw header
    /// text, de-chunked body bytes read so far); status 0 if no header block
    /// ever arrived.
    fn http_get(port: u16, path: &str, n: usize) -> (u16, String, Vec<u8>) {
        let mut conn = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write_get(&conn, path);
        conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            if let Some(parsed) = parse_response(&raw) {
                if parsed.2.len() >= n {
                    return parsed;
                }
            }
            match conn.read(&mut buf) {
                Ok(0) | Err(_) => break, // closed, or timed out
                Ok(k) => raw.extend_from_slice(&buf[..k]),
            }
        }
        parse_response(&raw).unwrap_or((0, String::new(), Vec::new()))
    }

    #[test]
    fn wrong_key_is_404_and_right_key_streams_header_then_pages() {
        let port = free_port(); let src = fake_source(); let tx = src.pages.clone();
        let srv = ListenerServer::start(port, "k1".into(), src).unwrap();
        assert_eq!(http_get(port, "/listen/nope", 0).0, 404);
        assert_eq!(http_get(port, "/", 0).0, 404);
        std::thread::spawn(move || { for _ in 0..10 { std::thread::sleep(Duration::from_millis(50)); let _ = tx.send(Bytes::from_static(b"PAGE")); } });
        let (st, headers, body) = http_get(port, "/listen/k1", 12);
        assert_eq!(st, 200);
        assert!(headers.to_ascii_lowercase().contains("content-type: audio/ogg"));
        assert!(body.starts_with(b"HDR!PAGEPAGE"));
        srv.stop();
    }

    #[test]
    fn listening_page_embeds_stream_url() {
        let port = free_port();
        let srv = ListenerServer::start(port, "k2".into(), fake_source()).unwrap();
        let (st, headers, body) = http_get(port, "/listen/k2/page", 64);
        assert_eq!(st, 200);
        assert!(headers.to_ascii_lowercase().contains("text/html"));
        assert!(String::from_utf8_lossy(&body).contains("/listen/k2"));
        srv.stop();
    }

    #[test]
    fn listener_count_tracks_open_connections() {
        let port = free_port();
        let srv = ListenerServer::start(port, "k3".into(), fake_source()).unwrap();
        assert_eq!(srv.listener_count(), 0);
        let conn = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write_get(&conn, "/listen/k3"); std::thread::sleep(Duration::from_millis(200));
        assert_eq!(srv.listener_count(), 1);
        drop(conn); std::thread::sleep(Duration::from_millis(500));
        assert_eq!(srv.listener_count(), 0);
        srv.stop();
    }

    #[test]
    fn lagging_listener_is_dropped_without_stalling_others() {
        let port = free_port(); let src = fake_source(); let tx = src.pages.clone();
        let srv = ListenerServer::start(port, "k4".into(), src).unwrap();
        let stalled = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write_get(&stalled, "/listen/k4"); // never read from it
        std::thread::sleep(Duration::from_millis(200));
        for _ in 0..2000 { let _ = tx.send(Bytes::from(vec![0u8; 8192])); } // » 256-slot capacity
        let (st, _, body) = http_get(port, "/listen/k4", 4); // a fresh listener still gets served
        assert_eq!(st, 200); assert!(body.starts_with(b"HDR!"));
        std::thread::sleep(Duration::from_millis(500));
        // The fresh listener has hung up, so the one left is the stalled one:
        // with its socket full, the server is parked on a write and has not
        // looked at the channel again, so it cannot have noticed the lag yet.
        assert_eq!(srv.listener_count(), 1, "only the stalled listener should remain");

        // Resume reading. Once its socket drains, the server polls the body
        // again, the receiver reports `Lagged`, and the body must END (the
        // chunked terminator `0\r\n\r\n`) rather than skip ahead.
        let mut stalled = stalled;
        stalled.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 64 * 1024];
        while !raw.ends_with(b"\r\n0\r\n\r\n") {
            match stalled.read(&mut buf) {
                Ok(0) | Err(_) => break, // closed, or timed out
                Ok(k) => raw.extend_from_slice(&buf[..k]),
            }
        }
        assert!(raw.ends_with(b"\r\n0\r\n\r\n"), "lagged body should have ended");
        let (_, _, body) = parse_response(&raw).unwrap();
        assert!(body.starts_with(b"HDR!"));
        assert!(body.len() < 4 + 2000 * 8192, "lagged listener must not receive every page");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(srv.listener_count(), 0, "lagged listener should have been dropped");
        drop(stalled); srv.stop();
    }

    #[test]
    fn restart_with_new_key_rejects_old_key() {
        let port = free_port();
        let srv = ListenerServer::start(port, "old".into(), fake_source()).unwrap();
        assert_eq!(http_get(port, "/listen/old", 0).0, 200);
        srv.stop();
        let srv = ListenerServer::start(port, "new".into(), fake_source()).unwrap();
        assert_eq!(http_get(port, "/listen/old", 0).0, 404);
        assert_eq!(http_get(port, "/listen/new", 0).0, 200);
        srv.stop();
    }

    #[test]
    fn stop_returns_while_a_listener_is_still_connected() {
        let port = free_port();
        let srv = ListenerServer::start(port, "k6".into(), fake_source()).unwrap();
        let conn = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write_get(&conn, "/listen/k6");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(srv.listener_count(), 1);
        // A stream never finishes on its own, so stop must not wait for it.
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || { srv.stop(); let _ = done_tx.send(()); });
        assert!(done_rx.recv_timeout(Duration::from_secs(3)).is_ok(), "stop hung on an open stream");
        // And the port is free again.
        std::net::TcpListener::bind(("0.0.0.0", port)).expect("port should be free after stop");
        drop(conn);
    }

    #[test]
    fn start_fails_when_port_is_busy() {
        let holder = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let port = holder.local_addr().unwrap().port();
        let err = ListenerServer::start(port, "k".into(), fake_source()).err().expect("must fail");
        assert!(err.contains(&port.to_string()));
    }
}
