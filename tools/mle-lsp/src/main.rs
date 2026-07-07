//! `mle-lsp` — a minimal Language Server for `.mle` files (docs/mle.md
//! Track D).
//!
//! Hand-rolled LSP over stdio: a blocking read loop with `Content-Length`
//! framing and `serde_json` dispatch — no async runtime, no LSP framework.
//! On `didOpen`/`didChange` the buffer is fed through [`mle::parse`],
//! [`mle::lower`], and [`mle::check`]; the first parse/lower error, or ALL
//! type diagnostics, publish via `textDocument/publishDiagnostics`.
//!
//! Document sync is **full** (`textDocumentSync: 1`): every change carries
//! the whole buffer. The server keeps one piece of state — a uri→text map —
//! to answer `textDocument/hover` (quick info: `name : Type` from the
//! gradual checker, via `mle::hover`), `textDocument/definition`
//! (go-to-definition via `mle::goto`; MLE is single-file, so the answer is
//! always a `Location` in the same document), `textDocument/inlayHint`
//! (inferred `: Type` ghost text on unannotated lambda params, via
//! `mle::inlay`), and `textDocument/codeLens` (each top-level def's inferred
//! signature above it, via `mle::codelens`). Diagnostics cover parse,
//! lowering, and every `mle::check` type diagnostic.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

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
    let mut documents: HashMap<String, String> = HashMap::new();
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
                    "capabilities": {
                        "textDocumentSync": 1,
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "inlayHintProvider": true,
                        "codeLensProvider": { "resolveProvider": false },
                    },
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
            ("textDocument/hover", Some(id)) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                let result = documents
                    .contains_key(uri)
                    .then(|| hover(uri, &documents, &params["position"]))
                    .flatten()
                    .unwrap_or(Value::Null);
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                );
            }
            ("textDocument/definition", Some(id)) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                let result = documents
                    .contains_key(uri)
                    .then(|| definition(uri, &documents, &params["position"]))
                    .flatten()
                    .unwrap_or(Value::Null);
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                );
            }
            ("textDocument/inlayHint", Some(id)) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                let result = documents
                    .contains_key(uri)
                    .then(|| inlay_hints(uri, &documents, &params["range"]))
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                );
            }
            ("textDocument/codeLens", Some(id)) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                let result = documents
                    .contains_key(uri)
                    .then(|| code_lenses(uri, &documents))
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
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
                documents.insert(uri.to_string(), text.to_string());
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
                    documents.insert(uri.to_string(), text.to_string());
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
    0
}

/// Run the MLE front-end over `text` and publish the outcome: one diagnostic
/// for the first parse/lower failure, every `mle::check` type diagnostic for
/// a lowered module, or an empty list (clearing squiggles) when clean.
fn publish_diagnostics(writer: &mut impl Write, uri: &str, text: &str) {
    let diagnostic = |message: &str, span: mle::Span| {
        json!({
            "range": span_to_range(text, span),
            "severity": 1, // Error
            "source": "mle",
            "message": message,
        })
    };
    // Parse and lowering stop at the first error; a clean module then gets
    // ALL of the gradual checker's type diagnostics.
    let diagnostics = match mle::parse(text) {
        Err(err) => vec![diagnostic(&err.message, err.span)],
        Ok(program) => match mle::lower(program) {
            Err(err) => vec![diagnostic(&err.message, err.span)],
            Ok(module) => mle::check(&module)
                .into_iter()
                .map(|err| diagnostic(&err.message, err.span))
                .collect(),
        },
    };
    write_diagnostics(writer, uri, diagnostics);
}

/// Load the MLE program an open document belongs to. With a `functor.json`
/// above it, that's the whole multi-file project — every open buffer stands
/// in for its on-disk file (unsaved edits count), siblings load from disk.
/// With no `functor.json`, it's just this buffer as a single-file project
/// (the directory is NOT scanned — unrelated `.mle` files nearby must not
/// leak in). `None` on a non-`file:` URI or a load failure.
fn load_project(uri: &str, documents: &HashMap<String, String>) -> Option<mle::project::Project> {
    let path = uri_to_path(uri)?;
    let single_file = || mle::project::load_single_file(&path, documents.get(uri)?).ok();
    match discover_entry(&path) {
        Some(entry) => {
            let overrides: HashMap<PathBuf, String> = documents
                .iter()
                .filter_map(|(u, text)| Some((uri_to_path(u)?, text.clone())))
                .collect();
            // Use the project only if it loaded AND actually contains this
            // file. A nearest `functor.json` whose entry lives in another dir,
            // or a broken sibling that fails the load, must not strip the open
            // buffer of all features — fall back to a single-file view.
            match mle::project::load_with_overrides(&entry, &overrides) {
                Ok(project) if project.sources.file_by_path(&path).is_some() => Some(project),
                _ => single_file(),
            }
        }
        None => single_file(),
    }
}

/// Answer a hover request: load the project, find the innermost node at the
/// (UTF-16) position in the open file, and render `name : Type` as markdown.
/// Cross-file inference means the type can come from a sibling module.
fn hover(uri: &str, documents: &HashMap<String, String>, position: &Value) -> Option<Value> {
    let project = load_project(uri, documents)?;
    let file = project.sources.file_by_path(&uri_to_path(uri)?)?;
    let offset = file.base + position_to_offset(&file.src, position)?;
    let (_, types) = project.check_with_types();
    let (span, hover_text) = mle::hover::hover_text(&project.module, &types, offset)?;
    let (_, range) = localize(&project, span)?;
    Some(json!({
        "contents": { "kind": "markdown", "value": format!("```mle\n{hover_text}\n```") },
        "range": range,
    }))
}

/// Answer a definition request: load the project, resolve the reference at
/// the position via `mle::goto`, and return the definition site as a
/// `Location`. The target may live in a **sibling file** (cross-file goto),
/// so the location's URI is whichever file owns the resolved span.
fn definition(uri: &str, documents: &HashMap<String, String>, position: &Value) -> Option<Value> {
    let project = load_project(uri, documents)?;
    let file = project.sources.file_by_path(&uri_to_path(uri)?)?;
    let offset = file.base + position_to_offset(&file.src, position)?;
    let span = mle::goto::definition_span(&project.module, offset)?;
    let (target_uri, range) = localize(&project, span)?;
    Some(json!({ "uri": target_uri, "range": range }))
}

/// Answer a code-lens request: load the project and return one lens per
/// top-level def **in the open file** with a known inferred signature
/// (`name : Type`), anchored on the line above the def. The command is inert
/// (empty `command`), so the lens is informational, not clickable.
fn code_lenses(uri: &str, documents: &HashMap<String, String>) -> Option<Value> {
    let project = load_project(uri, documents)?;
    let file = project.sources.file_by_path(&uri_to_path(uri)?)?;
    let (_, types) = project.check_with_types();
    let lenses: Vec<Value> = mle::codelens::signatures(&project.module, &types)
        .into_iter()
        // A merged project holds every module's defs; keep only this file's.
        .filter(|lens| owns(file, lens.span.start))
        .map(|lens| {
            json!({
                "range": local_range(file, lens.span),
                "command": { "title": lens.title, "command": "" },
            })
        })
        .collect();
    Some(Value::Array(lenses))
}

/// Answer an inlay-hint request: load the project and return an inferred-type
/// hint (`: Type`) after each unannotated lambda parameter **in the open
/// file**, clipped to the requested range.
fn inlay_hints(uri: &str, documents: &HashMap<String, String>, range: &Value) -> Option<Value> {
    let project = load_project(uri, documents)?;
    let file = project.sources.file_by_path(&uri_to_path(uri)?)?;
    let (_, types) = project.check_with_types();
    // The client's range is local to the open file; lift it into the
    // project-wide span space. A missing/unparsable end means "no filter".
    let start = position_to_offset(&file.src, &range["start"]).map(|o| file.base + o);
    let end = position_to_offset(&file.src, &range["end"]).map(|o| file.base + o);
    let in_range = |offset: usize| match (start, end) {
        // LSP ranges are half-open — `end` is exclusive.
        (Some(s), Some(e)) => s <= offset && offset < e,
        _ => true,
    };
    let hints: Vec<Value> = mle::inlay::inlay_hints(&project.module, &types)
        .into_iter()
        .filter(|h| owns(file, h.offset) && in_range(h.offset))
        .map(|h| {
            json!({
                "position": lsp_position(&file.src, h.offset - file.base),
                "label": h.label,
                "kind": 1, // InlayHintKind.Type
                "paddingLeft": false,
                "paddingRight": false,
            })
        })
        .collect();
    Some(Value::Array(hints))
}

/// Whether project-wide `offset` falls in `file` (its half-open base range).
fn owns(file: &mle::project::SourceFile, offset: usize) -> bool {
    file.base <= offset && offset <= file.base + file.src.len()
}

/// A project-wide span → an LSP range local to the file it belongs to, plus
/// that file's URI. Spans never straddle files (each node is lowered from one
/// file), so both ends localize against the start file.
fn localize(project: &mle::project::Project, span: mle::Span) -> Option<(String, Value)> {
    let file = project.sources.file_at(span.start);
    Some((path_to_uri(&file.path), local_range(file, span)))
}

/// A project-wide span → an LSP range in `file`'s local coordinates.
fn local_range(file: &mle::project::SourceFile, span: mle::Span) -> Value {
    let start = span.start.saturating_sub(file.base);
    let end = span.end.saturating_sub(file.base).min(file.src.len());
    json!({
        "start": lsp_position(&file.src, start),
        "end": lsp_position(&file.src, end),
    })
}

/// The project entry for `path`: walk up for a `functor.json` and join its
/// `entry`. `None` when there's no `functor.json` above `path` — the caller
/// then treats the file as a standalone single-file program. Reads at most
/// one `functor.json` per ancestor — cheap, and the tree is shallow.
fn discover_entry(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if let Ok(text) = std::fs::read_to_string(d.join("functor.json")) {
            if let Ok(config) = serde_json::from_str::<Value>(&text) {
                if let Some(entry) = config["entry"].as_str() {
                    return Some(d.join(entry));
                }
            }
        }
        dir = d.parent();
    }
    None
}

/// A `file:` URI → filesystem path (percent-decoded). `None` for other
/// schemes (untitled buffers, `git:` diffs) — the server simply won't answer
/// those, which is correct.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    Some(PathBuf::from(percent_decode(rest)))
}

/// A filesystem path → `file:` URI. Every byte that isn't an unreserved URI
/// character (or a `/` path separator) is percent-encoded — including
/// non-ASCII bytes, which must be escaped per-byte rather than widened to a
/// `char` (that would double-encode). Round-trips with [`uri_to_path`] /
/// [`percent_decode`].
fn path_to_uri(path: &Path) -> String {
    let mut encoded = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Decode `%XX` escapes in a URI path. Non-escape bytes pass through. Works
/// on bytes (never slicing a `&str`), so a `%` before a multi-byte character
/// or at the string's end can't panic — the LSP must survive odd URIs, like
/// `read_message` survives garbage frames.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16);
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Invert [`lsp_position`]: an LSP `{line, character}` (UTF-16 code units)
/// to a byte offset. Clamps past-end-of-line characters to the line end.
fn position_to_offset(text: &str, position: &Value) -> Option<usize> {
    let line = position["line"].as_u64()? as usize;
    let character = position["character"].as_u64()? as usize;
    let line_start = if line == 0 {
        0
    } else {
        text.match_indices('\n').nth(line - 1)?.0 + 1
    };
    let line_text = &text[line_start..];
    let line_text = &line_text[..line_text.find('\n').unwrap_or(line_text.len())];
    let mut units = 0;
    for (byte, ch) in line_text.char_indices() {
        if units >= character {
            return Some(line_start + byte);
        }
        units += ch.len_utf16();
    }
    Some(line_start + line_text.len())
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

    // [xreview High] percent_decode must not panic on a `%` before a
    // multi-byte char or at the string's end — the old str-slice did.
    #[test]
    fn percent_decode_survives_malformed_escapes() {
        assert_eq!(percent_decode("caf%C3%A9"), "café"); // valid escape
        assert_eq!(percent_decode("%bé"), "%bé"); // % before a multi-byte char
        assert_eq!(percent_decode("trailing%"), "trailing%"); // % at end
        assert_eq!(percent_decode("bad%zz"), "bad%zz"); // non-hex digits
    }

    // [xreview Medium] path_to_uri must round-trip a non-ASCII path (each
    // byte escaped, not widened to a char), so cross-file goto URIs resolve.
    #[test]
    fn path_to_uri_round_trips_non_ascii() {
        let path = std::path::Path::new("/Users/café/game.mle");
        let uri = path_to_uri(path);
        assert!(!uri.contains('é'), "non-ascii must be percent-escaped: {uri}");
        assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
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
