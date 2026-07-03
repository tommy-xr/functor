//! `mle-lsp` — a minimal Language Server for `.mle` files (docs/mle.md
//! Track D).
//!
//! Hand-rolled LSP over stdio: a blocking read loop with `Content-Length`
//! framing and `serde_json` dispatch — no async runtime, no LSP framework.
//! The one feature is diagnostics: on `didOpen`/`didChange` the buffer is fed
//! through [`mle::parse`] and [`mle::lower`], and the first error (or a clean
//! bill of health) is published via `textDocument/publishDiagnostics`.
//!
//! Document sync is **full** (`textDocumentSync: 1`): every change carries
//! the whole buffer, so the server keeps no state at all — diagnostics are
//! computed straight from the incoming text.

use std::io::{BufRead, BufReader, Write};

use serde_json::{json, Value};

/// JSON-RPC "method not found" (LSP inherits JSON-RPC 2.0 error codes).
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC "invalid request" — requests after `shutdown`.
const INVALID_REQUEST: i64 = -32600;

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let code = serve(&mut BufReader::new(stdin.lock()), &mut stdout.lock());
    std::process::exit(code);
}

/// The server loop: read framed messages until `exit` (or EOF), dispatching
/// requests (messages with an `id`) and notifications. Returns the process
/// exit code: per the LSP spec, `exit` after `shutdown` is 0, `exit` without
/// it is 1 (EOF — the client vanished — is a quiet 0).
fn serve(reader: &mut impl BufRead, writer: &mut impl Write) -> i32 {
    let mut shutdown_seen = false;
    while let Some(message) = read_message(reader) {
        let method = message["method"].as_str().unwrap_or("").to_string();
        let id = message.get("id").cloned();
        let params = &message["params"];
        // Post-shutdown, the only valid message is `exit`: requests get
        // InvalidRequest per the spec; notifications are dropped.
        if shutdown_seen && method != "exit" {
            if let Some(id) = id {
                let error = json!({
                    "code": INVALID_REQUEST,
                    "message": "server is shutting down",
                });
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "error": error }),
                );
            }
            continue;
        }
        match (method.as_str(), id) {
            // --- Requests (have an id; must be answered). ---
            ("initialize", Some(id)) => {
                let result = json!({
                    "capabilities": { "textDocumentSync": 1 },
                    "serverInfo": { "name": "mle-lsp" },
                });
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                );
            }
            ("shutdown", Some(id)) => {
                shutdown_seen = true;
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": null }),
                );
            }
            (_, Some(id)) => {
                let error = json!({
                    "code": METHOD_NOT_FOUND,
                    "message": format!("method not found: {method}"),
                });
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "error": error }),
                );
            }
            // --- Notifications (no id; never answered). ---
            ("exit", None) => return i32::from(!shutdown_seen),
            ("textDocument/didOpen", None) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                let text = params["textDocument"]["text"].as_str().unwrap_or("");
                publish_diagnostics(writer, uri, text);
            }
            ("textDocument/didChange", None) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                // Full sync: the last content change is the whole new buffer.
                if let Some(text) = params["contentChanges"]
                    .as_array()
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str())
                {
                    publish_diagnostics(writer, uri, text);
                }
            }
            ("textDocument/didClose", None) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                // Clear stale squiggles for the closed file.
                write_diagnostics(writer, uri, vec![]);
            }
            // initialized, $/… and any other notification: deliberately ignored.
            (_, None) => {}
        }
    }
    0
}

/// Run the MLE front-end over `text` and publish the outcome: one Error
/// diagnostic for the first parse/lower failure, or an empty list (which
/// clears previous diagnostics) when the module is clean.
fn publish_diagnostics(writer: &mut impl Write, uri: &str, text: &str) {
    let error = match mle::parse(text) {
        Err(err) => Some((err.message, err.span)),
        Ok(program) => match mle::lower(program) {
            Err(err) => Some((err.message, err.span)),
            Ok(_) => None,
        },
    };
    let diagnostics = match error {
        Some((message, span)) => vec![json!({
            "range": span_to_range(text, span),
            "severity": 1, // Error
            "source": "mle",
            "message": message,
        })],
        None => vec![],
    };
    write_diagnostics(writer, uri, diagnostics);
}

fn write_diagnostics(writer: &mut impl Write, uri: &str, diagnostics: Vec<Value>) {
    write_message(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": uri, "diagnostics": diagnostics },
        }),
    );
}

/// Convert a byte-offset [`mle::Span`] to an LSP range. The span is
/// half-open, matching LSP's exclusive `end`. LSP positions count characters
/// in **UTF-16 code units** (the default encoding; this server doesn't
/// negotiate another), not chars — an astral-plane character earlier on the
/// line counts as two.
fn span_to_range(text: &str, span: mle::Span) -> Value {
    json!({
        "start": lsp_position(text, span.start),
        "end": lsp_position(text, span.end),
    })
}

fn lsp_position(text: &str, offset: usize) -> Value {
    let prefix = &text[..offset.min(text.len())];
    let line = prefix.matches('\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    let character = prefix[line_start..].encode_utf16().count();
    json!({ "line": line, "character": character })
}

/// Read one `Content-Length`-framed message. Returns `None` only when the
/// stream is done (EOF, short read, missing `Content-Length`); a frame whose
/// body is valid length but invalid JSON is skipped — the stream is still
/// framed-in-sync, so one garbage message must not kill diagnostics for the
/// whole session.
fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    loop {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).ok()? == 0 {
                return None; // EOF
            }
            let line = line.trim_end();
            if line.is_empty() {
                break; // blank line ends the header block
            }
            // Header names are case-insensitive (RFC 9110, and clients vary).
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    content_length = value.trim().parse().ok();
                }
            }
            // Other headers (Content-Type) are ignored.
        }
        let mut body = vec![0; content_length?];
        reader.read_exact(&mut body).ok()?;
        match serde_json::from_slice(&body) {
            Ok(message) => return Some(message),
            Err(_) => continue, // skip the garbage frame, keep serving
        }
    }
}

fn write_message(writer: &mut impl Write, message: &Value) {
    let body = message.to_string();
    // A write failure means the client is gone; exiting quietly beats a panic.
    let _ = write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = writer.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_to_range_is_zero_based_with_end_position() {
        // "let = 3": the parse error spans the `=` at bytes 4..5.
        let text = "let = 3";
        let range = span_to_range(text, mle::Span::new(4, 5));
        assert_eq!(
            range,
            json!({
                "start": { "line": 0, "character": 4 },
                "end": { "line": 0, "character": 5 },
            })
        );
    }

    #[test]
    fn span_to_range_crosses_lines() {
        let text = "let a = 1\nlet b = 2\n";
        // Span of `b = 2` on line 2 (bytes 14..19).
        let range = span_to_range(text, mle::Span::new(14, 19));
        assert_eq!(
            range,
            json!({
                "start": { "line": 1, "character": 4 },
                "end": { "line": 1, "character": 9 },
            })
        );
    }

    #[test]
    fn read_message_parses_a_framed_body() {
        let framed = b"Content-Length: 16\r\n\r\n{\"method\":\"foo\"}";
        let message = read_message(&mut &framed[..]).unwrap();
        assert_eq!(message["method"], "foo");
    }

    #[test]
    fn read_message_returns_none_on_eof() {
        assert!(read_message(&mut &b""[..]).is_none());
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;

    // A garbage frame must not kill the session — the next valid frame is
    // served. [review High]
    #[test]
    fn garbage_frame_is_skipped() {
        let framed =
            b"Content-Length: 5\r\n\r\n{oopsContent-Length: 16\r\n\r\n{\"method\":\"foo\"}";
        let message = read_message(&mut &framed[..]).unwrap();
        assert_eq!(message["method"], "foo");
    }

    // LSP characters are UTF-16 code units: the emoji is 2 units, so an
    // error after it lands 2 further than the char count. [review Medium]
    #[test]
    fn positions_count_utf16_code_units() {
        let text = "let s = \"\u{1F642}\" @";
        let at = text.find('@').unwrap();
        let pos = lsp_position(text, at);
        assert_eq!(pos["character"], 13); // 12 chars, but the emoji counts twice
    }

    // exit without shutdown is code 1; after shutdown, 0. [review Low]
    #[test]
    fn exit_code_reflects_shutdown() {
        let exit_only = b"Content-Length: 17\r\n\r\n{\"method\":\"exit\"}";
        let mut out = Vec::new();
        assert_eq!(serve(&mut &exit_only[..], &mut out), 1);

        let shutdown_then_exit = b"Content-Length: 36\r\n\r\n{\"id\":1,\"method\":\"shutdown\"}\
Content-Length: 17\r\n\r\n{\"method\":\"exit\"}";
        let mut out = Vec::new();
        assert_eq!(serve(&mut &shutdown_then_exit[..], &mut out), 0);
    }
}

#[cfg(test)]
mod codex_review_tests {
    use super::*;

    // Header names are case-insensitive. [review Low, probe-verified]
    #[test]
    fn lowercase_content_length_is_accepted() {
        let framed = b"content-length: 16\r\n\r\n{\"method\":\"foo\"}";
        let message = read_message(&mut &framed[..]).unwrap();
        assert_eq!(message["method"], "foo");
    }

    // After shutdown, only exit is honored: notifications are dropped
    // (no diagnostics published) and requests get InvalidRequest.
    #[test]
    fn post_shutdown_traffic_is_refused() {
        let open = r#"{"method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///x.mle","text":"let = 3"}}}"#;
        let req = r#"{"id":2,"method":"initialize","params":{}}"#;
        let shutdown = r#"{"id":1,"method":"shutdown"}"#;
        let mut input = Vec::new();
        for body in [shutdown, open, req] {
            input.extend_from_slice(
                format!("Content-Length: {}\r\n\r\n{body}", body.len()).as_bytes(),
            );
        }
        let mut out = Vec::new();
        assert_eq!(serve(&mut &input[..], &mut out), 0); // EOF after refusals
        let out = String::from_utf8(out).unwrap();
        assert!(!out.contains("publishDiagnostics"), "out: {out}");
        assert!(out.contains("-32600"), "out: {out}");
    }
}
