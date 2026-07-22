//! Small native HTTP transport for the shared debug protocol.
//!
//! This intentionally uses only `std::net`: desktop and Android share every
//! route and status rule without either shell pulling in an HTTP framework.
//! Requests are handed to the runtime loop through [`DebugRequest`]'s response
//! channels, keeping game state and graphics access on that loop's thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::debug_protocol::{
    self, CaptureError, DebugRequest, InputCommand, ProjectSources, RewindCommand, RuntimeState,
    TimeCommand,
};

const MAX_HEADER_LINE_BYTES: u64 = 8 * 1024;
const MAX_HEADER_LINES: usize = 64;
const MAX_COMMAND_BYTES: usize = 64 * 1024;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Bind `address`, start the transport thread, and return the frame loop's
/// request receiver. Runtime shells choose their own bind/error policy.
pub fn spawn(address: impl ToSocketAddrs) -> std::io::Result<mpsc::Receiver<DebugRequest>> {
    spawn_listener(TcpListener::bind(address)?)
}

fn spawn_listener(listener: TcpListener) -> std::io::Result<mpsc::Receiver<DebugRequest>> {
    let (tx, rx) = mpsc::channel::<DebugRequest>();
    std::thread::Builder::new()
        .name("functor-debug-http".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if handle(stream, &tx).is_none() {
                    break;
                }
            }
        })?;
    Ok(rx)
}

fn read_line_capped(reader: &mut BufReader<TcpStream>, cap: u64) -> Option<String> {
    let mut limited = Read::by_ref(reader).take(cap);
    let mut line = String::new();
    match limited.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) if line.ends_with('\n') => Some(line),
        Ok(_) | Err(_) => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BodyError {
    MissingLength,
    TooLarge,
    Invalid,
}

fn read_body(
    reader: &mut BufReader<TcpStream>,
    content_length: Option<usize>,
    max: usize,
) -> Result<Vec<u8>, BodyError> {
    let len = match content_length {
        Some(len) if len <= max => len,
        Some(_) => return Err(BodyError::TooLarge),
        None => return Err(BodyError::MissingLength),
    };
    let mut body = vec![0; len];
    reader
        .read_exact(&mut body)
        .map_err(|_| BodyError::Invalid)?;
    Ok(body)
}

fn parse_json<T: DeserializeOwned>(
    reader: &mut BufReader<TcpStream>,
    content_length: Option<usize>,
) -> Result<T, String> {
    let body =
        read_body(reader, content_length, MAX_COMMAND_BYTES).map_err(|error| match error {
            BodyError::MissingLength => "missing Content-Length".to_string(),
            BodyError::TooLarge => "body too large".to_string(),
            BodyError::Invalid => "bad body".to_string(),
        })?;
    serde_json::from_slice(&body).map_err(|error| error.to_string())
}

fn recv<T>(rx: mpsc::Receiver<T>) -> Result<T, mpsc::RecvTimeoutError> {
    rx.recv_timeout(RUNTIME_REPLY_TIMEOUT)
}

/// `None` means the frame-loop receiver is gone and the accept loop should end.
fn handle(mut stream: TcpStream, tx: &mpsc::Sender<DebugRequest>) -> Option<()> {
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));
    let mut reader = match stream.try_clone() {
        Ok(stream) => BufReader::new(stream),
        Err(_) => return Some(()),
    };

    let Some(request_line) = read_line_capped(&mut reader, MAX_HEADER_LINE_BYTES) else {
        respond_text(&mut stream, None, 400, "Bad Request", "bad request line");
        return Some(());
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path_with_query = parts.next().unwrap_or("");
    let path = path_with_query.split('?').next().unwrap_or("");

    let mut content_length = None;
    let mut origin = None;
    let mut duplicate_origin = false;
    let mut header_ok = false;
    for _ in 0..MAX_HEADER_LINES {
        let Some(line) = read_line_capped(&mut reader, MAX_HEADER_LINE_BYTES) else {
            break;
        };
        let line = line.trim_end();
        if line.is_empty() {
            header_ok = true;
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().ok();
            } else if name.eq_ignore_ascii_case("origin") {
                if origin.is_some() {
                    duplicate_origin = true;
                } else {
                    origin = Some(value.trim().to_string());
                }
            }
        }
    }
    if !header_ok {
        respond_text(&mut stream, None, 400, "Bad Request", "bad headers");
        return Some(());
    }
    if duplicate_origin
        || origin
            .as_deref()
            .is_some_and(|value| !is_loopback_origin(value))
    {
        respond_text(&mut stream, None, 403, "Forbidden", "origin is not allowed");
        return Some(());
    }
    let cors_origin = origin.as_deref();

    match (method, path) {
        ("OPTIONS", path) if is_protocol_path(path) => respond_bytes(
            &mut stream,
            cors_origin,
            204,
            "No Content",
            "text/plain",
            &[],
        ),
        ("GET", "/") => respond_bytes(
            &mut stream,
            cors_origin,
            200,
            "OK",
            "application/json",
            debug_protocol::discovery_json().as_bytes(),
        ),
        ("POST", "/capture") => {
            let (resp_tx, resp_rx) = mpsc::channel();
            if tx.send(DebugRequest::Capture(resp_tx)).is_err() {
                return runtime_gone(&mut stream, cors_origin);
            }
            match recv(resp_rx) {
                Ok(Ok(png)) => {
                    respond_bytes(&mut stream, cors_origin, 200, "OK", "image/png", &png)
                }
                Ok(Err(CaptureError::Unavailable(message))) => respond_text(
                    &mut stream,
                    cors_origin,
                    503,
                    "Service Unavailable",
                    &message,
                ),
                Ok(Err(CaptureError::Failed(message))) => respond_text(
                    &mut stream,
                    cors_origin,
                    500,
                    "Internal Server Error",
                    &message,
                ),
                Err(mpsc::RecvTimeoutError::Timeout) => respond_text(
                    &mut stream,
                    cors_origin,
                    503,
                    "Service Unavailable",
                    "runtime did not capture a frame in time",
                ),
                Err(mpsc::RecvTimeoutError::Disconnected) => respond_text(
                    &mut stream,
                    cors_origin,
                    500,
                    "Internal Server Error",
                    "capture failed",
                ),
            }
        }
        ("GET", "/state") => {
            let (resp_tx, resp_rx) = mpsc::channel::<RuntimeState>();
            if tx.send(DebugRequest::State(resp_tx)).is_err() {
                return runtime_gone(&mut stream, cors_origin);
            }
            match recv(resp_rx) {
                Ok(state) => respond_bytes(
                    &mut stream,
                    cors_origin,
                    200,
                    "OK",
                    "application/json",
                    state.to_json().as_bytes(),
                ),
                Err(_) => respond_text(
                    &mut stream,
                    cors_origin,
                    500,
                    "Internal Server Error",
                    "state failed",
                ),
            }
        }
        ("GET", "/scene") | ("GET", "/trace") => {
            let (resp_tx, resp_rx) = mpsc::channel();
            let request = if path == "/scene" {
                DebugRequest::Scene(resp_tx)
            } else {
                DebugRequest::Trace(resp_tx)
            };
            if tx.send(request).is_err() {
                return runtime_gone(&mut stream, cors_origin);
            }
            match recv(resp_rx) {
                Ok(json) => respond_bytes(
                    &mut stream,
                    cors_origin,
                    200,
                    "OK",
                    "application/json",
                    json.as_bytes(),
                ),
                Err(_) => respond_text(
                    &mut stream,
                    cors_origin,
                    500,
                    "Internal Server Error",
                    if path == "/scene" {
                        "scene failed"
                    } else {
                        "trace failed"
                    },
                ),
            }
        }
        ("POST", "/input") => {
            let command = match parse_json::<InputCommand>(&mut reader, content_length) {
                Ok(command) => command,
                Err(error) => {
                    respond_text(
                        &mut stream,
                        cors_origin,
                        400,
                        "Bad Request",
                        &format!("bad input json: {error}"),
                    );
                    return Some(());
                }
            };
            let (resp_tx, resp_rx) = mpsc::channel();
            if tx.send(DebugRequest::Input(command, resp_tx)).is_err() {
                return runtime_gone(&mut stream, cors_origin);
            }
            match recv(resp_rx) {
                Ok(Ok(())) => respond_text(&mut stream, cors_origin, 200, "OK", "ok"),
                Ok(Err(message)) => {
                    respond_text(&mut stream, cors_origin, 400, "Bad Request", &message)
                }
                Err(_) => respond_text(
                    &mut stream,
                    cors_origin,
                    500,
                    "Internal Server Error",
                    "input failed",
                ),
            }
        }
        ("POST", "/time") => {
            let command = match parse_json::<TimeCommand>(&mut reader, content_length) {
                Ok(command) => command,
                Err(error) => {
                    respond_text(
                        &mut stream,
                        cors_origin,
                        400,
                        "Bad Request",
                        &format!("bad time json: {error}"),
                    );
                    return Some(());
                }
            };
            let (resp_tx, resp_rx) = mpsc::channel();
            if tx.send(DebugRequest::Time(command, resp_tx)).is_err() {
                return runtime_gone(&mut stream, cors_origin);
            }
            match recv(resp_rx) {
                Ok(()) => respond_text(&mut stream, cors_origin, 200, "OK", "ok"),
                Err(_) => respond_text(
                    &mut stream,
                    cors_origin,
                    500,
                    "Internal Server Error",
                    "time failed",
                ),
            }
        }
        ("POST", "/rewind") => {
            let command = match parse_json::<RewindCommand>(&mut reader, content_length) {
                Ok(command) => command,
                Err(error) => {
                    respond_text(
                        &mut stream,
                        cors_origin,
                        400,
                        "Bad Request",
                        &format!("bad rewind json: {error}"),
                    );
                    return Some(());
                }
            };
            let (resp_tx, resp_rx) = mpsc::channel();
            if tx
                .send(DebugRequest::Rewind(command.frame, resp_tx))
                .is_err()
            {
                return runtime_gone(&mut stream, cors_origin);
            }
            respond_result(&mut stream, cors_origin, recv(resp_rx), "rewind")
        }
        ("POST", "/reload-source") | ("POST", "/reload-project") => {
            let body = match read_body(
                &mut reader,
                content_length,
                debug_protocol::MAX_RELOAD_BYTES,
            ) {
                Ok(body) => body,
                Err(BodyError::MissingLength | BodyError::TooLarge) => {
                    respond_text(
                        &mut stream,
                        cors_origin,
                        413,
                        "Payload Too Large",
                        "source too large (or missing Content-Length); limit is 4MB",
                    );
                    return Some(());
                }
                Err(BodyError::Invalid) => {
                    respond_text(&mut stream, cors_origin, 400, "Bad Request", "bad body");
                    return Some(());
                }
            };
            let (resp_tx, resp_rx) = mpsc::channel();
            let request = if path == "/reload-project" {
                let files = match serde_json::from_slice::<ProjectSources>(&body) {
                    Ok(files) => files,
                    Err(error) => {
                        respond_text(
                            &mut stream,
                            cors_origin,
                            400,
                            "Bad Request",
                            &format!("body must be a JSON array of [path, source] pairs: {error}"),
                        );
                        return Some(());
                    }
                };
                DebugRequest::ReloadProject(files, resp_tx)
            } else {
                let source = match String::from_utf8(body) {
                    Ok(source) => source,
                    Err(_) => {
                        respond_text(&mut stream, cors_origin, 400, "Bad Request", "bad body");
                        return Some(());
                    }
                };
                DebugRequest::ReloadSource(source, resp_tx)
            };
            if tx.send(request).is_err() {
                return runtime_gone(&mut stream, cors_origin);
            }
            respond_result(&mut stream, cors_origin, recv(resp_rx), "reload")
        }
        _ => respond_text(&mut stream, cors_origin, 404, "Not Found", "not found"),
    }
    Some(())
}

fn is_protocol_path(path: &str) -> bool {
    debug_protocol::DEBUG_ROUTES
        .iter()
        .any(|route| route.path == path)
}

/// Browser access is intentionally limited to IDEs served from this machine.
/// Origin syntax is much smaller than general URL syntax: a scheme and
/// authority only, with no credentials, path, query, or fragment.
fn is_loopback_origin(origin: &str) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
        || authority.contains('@')
        || authority.chars().any(char::is_whitespace)
    {
        return false;
    }

    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((address, suffix)) = rest.split_once(']') else {
            return false;
        };
        if address != "::1" {
            return false;
        }
        if suffix.is_empty() {
            ("[::1]", None)
        } else if let Some(port) = suffix.strip_prefix(':') {
            ("[::1]", Some(port))
        } else {
            return false;
        }
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        (host, Some(port))
    } else {
        (authority, None)
    };

    if !host.eq_ignore_ascii_case("localhost") && host != "127.0.0.1" && host != "[::1]" {
        return false;
    }
    match port {
        None => true,
        Some(port) => !port.is_empty() && port.parse::<u16>().is_ok(),
    }
}

fn runtime_gone(stream: &mut TcpStream, cors_origin: Option<&str>) -> Option<()> {
    respond_text(
        stream,
        cors_origin,
        503,
        "Service Unavailable",
        "runtime gone",
    );
    None
}

fn respond_result(
    stream: &mut TcpStream,
    cors_origin: Option<&str>,
    result: Result<Result<String, String>, mpsc::RecvTimeoutError>,
    operation: &str,
) {
    match result {
        Ok(Ok(status)) => respond_text(stream, cors_origin, 200, "OK", &status),
        Ok(Err(message)) => respond_text(stream, cors_origin, 400, "Bad Request", &message),
        Err(mpsc::RecvTimeoutError::Timeout) => respond_text(
            stream,
            cors_origin,
            503,
            "Service Unavailable",
            &format!("runtime did not apply the {operation} in time"),
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => respond_text(
            stream,
            cors_origin,
            500,
            "Internal Server Error",
            &format!("{operation} failed"),
        ),
    }
}

fn respond_text(
    stream: &mut TcpStream,
    cors_origin: Option<&str>,
    code: u16,
    reason: &str,
    body: &str,
) {
    respond_bytes(
        stream,
        cors_origin,
        code,
        reason,
        "text/plain; charset=utf-8",
        body.as_bytes(),
    )
}

fn respond_bytes(
    stream: &mut TcpStream,
    cors_origin: Option<&str>,
    code: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
) {
    let cors_headers = cors_origin.map_or_else(String::new, |origin| {
        format!(
            "Access-Control-Allow-Origin: {origin}\r\n\
             Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
             Access-Control-Allow-Headers: Content-Type\r\n\
             Access-Control-Allow-Private-Network: true\r\n\
             Vary: Origin\r\n"
        )
    });
    let header = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         {cors_headers}Connection: close\r\n\r\n",
        body.len(),
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use std::net::Shutdown;

    use super::*;

    fn connect(listener: &TcpListener, request: String) -> std::thread::JoinHandle<String> {
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        })
    }

    #[test]
    fn discovery_is_served_from_the_shared_contract() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = connect(
            &listener,
            "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n".into(),
        );
        let (tx, _rx) = mpsc::channel();
        handle(listener.accept().unwrap().0, &tx).unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("Content-Type: application/json\r\n"));
        assert!(!response.contains("Access-Control-Allow-Origin"));
        let body = response.split("\r\n\r\n").nth(1).unwrap();
        assert_eq!(body, debug_protocol::discovery_json());
    }

    #[test]
    fn reload_project_decodes_and_round_trips_runtime_status() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let body = r#"[["game.fun","let init = 1"]]"#;
        let request = format!(
            "POST /reload-project HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let client = connect(&listener, request);
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || handle(listener.accept().unwrap().0, &tx));

        match rx.recv().unwrap() {
            DebugRequest::ReloadProject(files, response) => {
                assert_eq!(files, vec![("game.fun".into(), "let init = 1".into())]);
                response.send(Ok("reloaded project".into())).unwrap();
            }
            _ => panic!("expected project reload request"),
        }

        assert_eq!(server.join().unwrap(), Some(()));
        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("reloaded project"));
    }

    #[test]
    fn loopback_origin_answers_browser_private_network_preflight() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let request = "OPTIONS /reload-project HTTP/1.1\r\n\
                       Host: localhost\r\n\
                       Origin: http://localhost:3000\r\n\
                       Access-Control-Request-Method: POST\r\n\
                       Access-Control-Request-Private-Network: true\r\n\r\n";
        let client = connect(&listener, request.into());
        let (tx, _rx) = mpsc::channel();
        handle(listener.accept().unwrap().0, &tx).unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 204 No Content\r\n"));
        assert!(response.contains("Access-Control-Allow-Origin: http://localhost:3000\r\n"));
        assert!(response.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n"));
        assert!(response.contains("Access-Control-Allow-Headers: Content-Type\r\n"));
        assert!(response.contains("Access-Control-Allow-Private-Network: true\r\n"));
        assert!(response.contains("Vary: Origin\r\n"));
        assert!(response.ends_with("\r\n\r\n"));
    }

    #[test]
    fn non_loopback_browser_origin_is_rejected_before_routing() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let request = "GET / HTTP/1.1\r\n\
                       Host: localhost\r\n\
                       Origin: https://evil.example\r\n\r\n";
        let client = connect(&listener, request.into());
        let (tx, _rx) = mpsc::channel();
        handle(listener.accept().unwrap().0, &tx).unwrap();

        let response = client.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 403 Forbidden\r\n"));
        assert!(!response.contains("Access-Control-Allow-Origin"));
        assert!(!response.contains("Access-Control-Allow-Private-Network"));
        assert!(response.ends_with("origin is not allowed"));
    }

    #[test]
    fn origin_parser_accepts_only_exact_loopback_hosts_and_numeric_ports() {
        for origin in [
            "http://localhost",
            "https://LOCALHOST:443",
            "http://127.0.0.1:8123",
            "https://[::1]",
            "http://[::1]:3000",
        ] {
            assert!(is_loopback_origin(origin), "should allow {origin}");
        }
        for origin in [
            "https://evil.example",
            "https://localhost.evil.example",
            "https://127.0.0.1.evil.example",
            "https://user@localhost",
            "https://localhost:not-a-port",
            "https://localhost:65536",
            "https://localhost/path",
            "file://localhost",
            "null",
        ] {
            assert!(!is_loopback_origin(origin), "should reject {origin}");
        }
    }
}
