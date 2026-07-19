//! The device-side push endpoint (M1 of the remote-develop loop): a minimal
//! HTTP listener accepting `POST /reload-source` with a `.fun` body — the
//! headset twin of the desktop debug server's endpoint (same route, same
//! 4MB body bound — enforced here as an exact `Content-Length` match — and
//! same status semantics), sized to the one route it serves rather than
//! pulling in an HTTP crate.
//!
//! Threading: GL and the producer live on the `android_main` thread, so the
//! listener runs on a background thread and hands each request to the frame
//! loop over an `mpsc` channel (the desktop `debug_server` pattern); the
//! frame loop drains with `try_recv` and replies through the per-request
//! response channel. When `android_main` returns, the receiver drops and the
//! accept loop exits on the next connection; until then the thread and its
//! bound port outlive the loop — Android tears the process down with the
//! activity, which is what actually reclaims them.
//!
//! Binding: **loopback only.** The dev PC reaches it via
//! `adb forward tcp:PORT tcp:PORT` (adb connects to the device's loopback),
//! so nothing on the LAN can reach it. Note loopback is NOT app-isolated on
//! Android: another app on the device holding INTERNET could POST here too —
//! an accepted dev-tool tradeoff. Wi-Fi push is a later, explicitly opt-in
//! slice.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

/// What a push carries.
pub enum ReloadBody {
    /// A single pushed entry (`POST /reload-source`) — siblings keep their
    /// last-pushed text.
    Source(String),
    /// A whole-project push (`POST /reload-project`): every file as
    /// `(path, source)`, entry FIRST — the producer's `reload_project`
    /// contract. The wire body is a JSON array of pairs.
    Project(Vec<(String, String)>),
}

/// A pushed body and the channel the frame loop answers on.
pub type ReloadRequest = (ReloadBody, mpsc::Sender<Result<String, String>>);

/// Game source is KBs; an unbounded read is an OOM invitation (the desktop
/// endpoint's bound).
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

/// Request-line / header lines get the same treatment: a client streaming
/// bytes with no newline must not grow a String unbounded.
const MAX_HEADER_LINE_BYTES: u64 = 8 * 1024;
const MAX_HEADER_LINES: usize = 64;

/// A stalled client must not wedge the (single) listener thread forever.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the listener waits for the frame loop to apply a push. Normal
/// reloads answer within a frame (~10ms); this only fires if the loop is
/// wedged (e.g. blocked in the compositor during a system interruption).
const RELOAD_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Bind the loopback listener and spawn the accept loop. Returns the frame
/// loop's end of the request channel.
pub fn spawn(port: u16) -> std::io::Result<mpsc::Receiver<ReloadRequest>> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let (tx, rx) = mpsc::channel::<ReloadRequest>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if handle(stream, &tx).is_none() {
                // The frame loop dropped its receiver — the app is exiting.
                break;
            }
        }
    });
    Ok(rx)
}

/// Read one `\n`-terminated line through a byte cap. `None` = EOF, a line
/// exceeding the cap, a read error, or a timeout — all cases where the
/// request is unusable and the connection should just be answered/closed.
fn read_line_capped(reader: &mut BufReader<TcpStream>, cap: u64) -> Option<String> {
    let mut limited = Read::by_ref(reader).take(cap);
    let mut line = String::new();
    match limited.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) if line.ends_with('\n') => Some(line),
        // Hit the cap (or EOF) mid-line — reject rather than resynchronize.
        Ok(_) | Err(_) => None,
    }
}

/// Serve one connection: parse the request, hand a valid push to the frame
/// loop, write the reply. `None` = the frame-loop channel is gone (stop
/// accepting); any client/parse problem answers the client and returns
/// `Some(())` so the accept loop continues.
fn handle(mut stream: TcpStream, tx: &mpsc::Sender<ReloadRequest>) -> Option<()> {
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));

    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return Some(()),
    });

    let Some(request_line) = read_line_capped(&mut reader, MAX_HEADER_LINE_BYTES) else {
        respond(&mut stream, 400, "Bad Request", "bad request line");
        return Some(());
    };
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));

    // Headers: only Content-Length matters here. Both the per-line and the
    // line-count reads are bounded (see the caps above).
    let mut content_length: Option<usize> = None;
    let mut header_ok = false;
    for _ in 0..MAX_HEADER_LINES {
        let Some(line) = read_line_capped(&mut reader, MAX_HEADER_LINE_BYTES) else {
            break; // client hung up / oversized line mid-headers
        };
        let line = line.trim_end();
        if line.is_empty() {
            header_ok = true;
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
    }
    if !header_ok {
        respond(&mut stream, 400, "Bad Request", "bad headers");
        return Some(());
    }

    match (method, path) {
        ("POST", "/reload-source") | ("POST", "/reload-project") => {
            // Bound the body; chunked bodies (no declared length) are
            // rejected the same way (the desktop endpoint's rule).
            let len = match content_length {
                Some(len) if len <= MAX_SOURCE_BYTES => len,
                _ => {
                    respond(
                        &mut stream,
                        413,
                        "Payload Too Large",
                        "source too large (or missing Content-Length); limit is 4MB",
                    );
                    return Some(());
                }
            };
            let mut body = String::new();
            if reader
                .take(len as u64)
                .read_to_string(&mut body)
                .is_err()
                || body.len() != len
            {
                respond(&mut stream, 400, "Bad Request", "bad body");
                return Some(());
            }
            let reload = if path == "/reload-project" {
                // The whole file set as a JSON array of (path, source)
                // pairs, entry first. A malformed body is the pusher's
                // mistake — reject before bothering the frame loop.
                match serde_json::from_str::<Vec<(String, String)>>(&body) {
                    Ok(files) => ReloadBody::Project(files),
                    Err(error) => {
                        respond(
                            &mut stream,
                            400,
                            "Bad Request",
                            &format!("body must be a JSON array of [path, source] pairs: {error}"),
                        );
                        return Some(());
                    }
                }
            } else {
                ReloadBody::Source(body)
            };
            let (resp_tx, resp_rx) = mpsc::channel();
            tx.send((reload, resp_tx)).ok()?;
            match resp_rx.recv_timeout(RELOAD_REPLY_TIMEOUT) {
                Ok(Ok(status)) => respond(&mut stream, 200, "OK", &status),
                // A load error in the pushed source — the pusher's mistake,
                // and the error names it (they're looking at the source): 400.
                Ok(Err(message)) => respond(&mut stream, 400, "Bad Request", &message),
                Err(mpsc::RecvTimeoutError::Timeout) => respond(
                    &mut stream,
                    503,
                    "Service Unavailable",
                    "runtime did not apply the reload in time",
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    respond(&mut stream, 500, "Internal Server Error", "reload failed")
                }
            }
        }
        ("GET", "/") => {
            // Discoverability (a probing dev/LLM): who this is, what it takes.
            respond(
                &mut stream,
                200,
                "OK",
                "{\"service\":\"functor quest runtime\",\"endpoints\":[\"POST /reload-source\",\"POST /reload-project\"]}",
            );
        }
        _ => respond(&mut stream, 404, "Not Found", "unknown endpoint"),
    }
    Some(())
}

fn respond(stream: &mut TcpStream, code: u16, reason: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}
