//! The device-side push endpoint (M1 of the remote-develop loop): a minimal
//! HTTP listener accepting `POST /reload-source` with a `.fun` body — the
//! headset twin of the desktop debug server's endpoint (same route, same
//! body-size guard, same status semantics), sized to the one route it
//! serves rather than pulling in an HTTP crate.
//!
//! Threading: GL and the producer live on the `android_main` thread, so the
//! listener runs on a background thread and hands each request to the frame
//! loop over an `mpsc` channel (the desktop `debug_server` pattern); the
//! frame loop drains with `try_recv` and replies through the per-request
//! response channel.
//!
//! Binding: **loopback only.** The dev PC reaches it via
//! `adb forward tcp:PORT tcp:PORT` (adb connects to the device's loopback),
//! so nothing on the LAN can push code to the headset. Wi-Fi push is a
//! later, explicitly opt-in slice.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

/// A pushed source body and the channel the frame loop answers on.
pub type ReloadRequest = (String, mpsc::Sender<Result<String, String>>);

/// Game source is KBs; an unbounded read is an OOM invitation (the desktop
/// endpoint's guard, verbatim).
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;

/// A stalled client must not wedge the (single) listener thread forever.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

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

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return Some(());
    }
    let mut parts = request_line.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));

    // Headers: only Content-Length matters here.
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return Some(()), // client hung up mid-headers
            Ok(_) => {}
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
    }

    match (method, path) {
        ("POST", "/reload-source") => {
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
            let (resp_tx, resp_rx) = mpsc::channel();
            tx.send((body, resp_tx)).ok()?;
            match resp_rx.recv() {
                Ok(Ok(status)) => respond(&mut stream, 200, "OK", &status),
                // A load error in the pushed source — the pusher's mistake,
                // and the error names it (they're looking at the source): 400.
                Ok(Err(message)) => respond(&mut stream, 400, "Bad Request", &message),
                Err(_) => respond(&mut stream, 500, "Internal Server Error", "reload failed"),
            }
        }
        ("GET", "/") => {
            // Discoverability (a probing dev/LLM): who this is, what it takes.
            respond(
                &mut stream,
                200,
                "OK",
                "{\"service\":\"functor quest runtime\",\"endpoints\":[\"POST /reload-source\"]}",
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
