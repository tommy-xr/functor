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
//! the whole buffer, so the server keeps no incremental state beyond a
//! uri→text map.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};

use serde_json::{json, Value};

/// JSON-RPC "method not found" (LSP inherits JSON-RPC 2.0 error codes).
const METHOD_NOT_FOUND: i64 = -32601;

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(&mut BufReader::new(stdin.lock()), &mut stdout.lock());
}

/// The server loop: read framed messages until `exit` (or EOF), dispatching
/// requests (messages with an `id`) and notifications.
fn serve(reader: &mut impl BufRead, writer: &mut impl Write) {
    let mut documents: HashMap<String, String> = HashMap::new();
    while let Some(message) = read_message(reader) {
        let method = message["method"].as_str().unwrap_or("").to_string();
        let id = message.get("id").cloned();
        let params = &message["params"];
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
            ("exit", None) => return,
            ("textDocument/didOpen", None) => {
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let text = params["textDocument"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                publish_diagnostics(writer, &uri, &text);
                documents.insert(uri, text);
            }
            ("textDocument/didChange", None) => {
                let uri = params["textDocument"]["uri"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                // Full sync: the last content change is the whole new buffer.
                if let Some(text) = params["contentChanges"]
                    .as_array()
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str())
                {
                    publish_diagnostics(writer, &uri, text);
                    documents.insert(uri, text.to_string());
                }
            }
            ("textDocument/didClose", None) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                documents.remove(uri);
                // Clear stale squiggles for the closed file.
                write_diagnostics(writer, uri, vec![]);
            }
            // initialized, $/… and any other notification: deliberately ignored.
            (_, None) => {}
        }
    }
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

/// Convert a byte-offset [`mle::Span`] to an LSP range. `mle::line_col` is
/// 1-based; LSP positions are 0-based, so both line and character shift down
/// by one. The span is half-open, matching LSP's exclusive `end`.
fn span_to_range(text: &str, span: mle::Span) -> Value {
    let (start_line, start_col) = mle::line_col(text, span.start);
    let (end_line, end_col) = mle::line_col(text, span.end);
    json!({
        "start": { "line": start_line - 1, "character": start_col - 1 },
        "end": { "line": end_line - 1, "character": end_col - 1 },
    })
}

/// Read one `Content-Length`-framed message. Returns `None` on EOF or a
/// malformed frame (either way the server has nothing left to do).
fn read_message(reader: &mut impl BufRead) -> Option<Value> {
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
        if let Some(value) = line.strip_prefix("Content-Length:") {
            content_length = value.trim().parse().ok();
        }
        // Other headers (Content-Type) are ignored.
    }
    let mut body = vec![0; content_length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
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
