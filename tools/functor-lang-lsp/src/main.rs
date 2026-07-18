//! `functor-lang-lsp` — a minimal Language Server for `.fun` source and `.funi`
//! interface files (docs/functor-lang.md Track D).
//!
//! Hand-rolled LSP over stdio: a blocking read loop with `Content-Length`
//! framing and `serde_json` dispatch — no async runtime, no LSP framework.
//! On `didOpen`/`didChange` the buffer is fed through [`functor_lang::parse`] or
//! [`functor_lang::parse_interface`],
//! [`functor_lang::lower`], and [`functor_lang::check`]; the first parse/lower error, or ALL
//! type diagnostics, publish via `textDocument/publishDiagnostics`.
//!
//! Document sync is **full** (`textDocumentSync: 1`): every change carries
//! the whole buffer. The server keeps one piece of state — a uri→text map —
//! to answer `textDocument/hover` (quick info: `name : Type` from the
//! gradual checker, via `functor_lang::hover`), `textDocument/definition`
//! (go-to-definition via `functor_lang::goto`; Functor Lang is single-file, so the answer is
//! always a `Location` in the same document), `textDocument/inlayHint`
//! (inferred `: Type` ghost text on unannotated lambda params, via
//! `functor_lang::inlay`), and `textDocument/codeLens` (each top-level def's inferred
//! signature above it, via `functor_lang::codelens`). Diagnostics cover parse,
//! lowering, and every `functor_lang::check` type diagnostic.
//!
//! On top of that it hosts the **paused-scene inspector** (see [`inspector`]):
//! it ingests a runtime trace document — pushed via the custom notification
//! `functor/inspector/trace`, or pulled by polling `GET /trace` after
//! `functor/inspector/attach {port}` — and overlays live binding values as
//! inlay hints plus a click-to-cycle execution-picker code lens (command
//! `functor.inspector.cycleExecution`), gated on a per-file source-hash match so
//! stale buffers never show values on the wrong lines. Any trace or selection
//! change triggers server-initiated `workspace/inlayHint/refresh` and
//! `workspace/codeLens/refresh` requests.

mod inspector;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;

use serde_json::{json, Value};

use inspector::TraceDoc;

/// JSON-RPC "method not found" (LSP inherits JSON-RPC 2.0 error codes).
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC "invalid request" — requests after `shutdown`.
const INVALID_REQUEST: i64 = -32600;

/// A message arriving on the multiplexed channel: a framed message (from stdin
/// or an attach-poll thread), or the stdin reader hitting EOF. `Eof` ends the
/// loop even though attach-poll senders (which never close on their own) keep
/// the channel otherwise alive.
enum Incoming {
    Msg(Value),
    Eof,
}

fn main() {
    // Multiplex two message sources into one queue: stdin (the LSP client) and
    // background attach-poll threads (which inject `functor/inspector/trace`
    // notifications). A dedicated thread reads framed stdin messages so the main
    // loop can block on the shared channel.
    let (tx, rx) = std::sync::mpsc::channel::<Incoming>();
    let stdin_tx = tx.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());
        while let Some(message) = read_message(&mut reader) {
            if stdin_tx.send(Incoming::Msg(message)).is_err() {
                return;
            }
        }
        let _ = stdin_tx.send(Incoming::Eof); // client vanished — stop the loop
    });
    // Debounced diagnostics: `didChange` signals this thread instead of
    // publishing inline; after a quiet window it injects a `$/functorFlush`
    // notification back onto the channel and the loop publishes the settled
    // buffer. Tunable via FUNCTOR_LSP_DIAGNOSTICS_DEBOUNCE_MS (default 1000ms;
    // 0 disables and publishes inline, per-keystroke).
    let debounce_ms = std::env::var("FUNCTOR_LSP_DIAGNOSTICS_DEBOUNCE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1000);
    let debounce = if debounce_ms > 0 {
        let (change_tx, change_rx) = std::sync::mpsc::channel::<String>();
        let flush_tx = tx.clone();
        std::thread::spawn(move || {
            debounce_publisher(change_rx, flush_tx, std::time::Duration::from_millis(debounce_ms));
        });
        Some(change_tx)
    } else {
        None
    };

    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let code = run(
        &mut || match rx.recv() {
            Ok(Incoming::Msg(message)) => Some(message),
            Ok(Incoming::Eof) | Err(_) => None,
        },
        &mut writer,
        tx,
        debounce,
    );
    std::process::exit(code);
}

/// Coalesce rapid `didChange`s into one diagnostics publish. Each edit (re)arms
/// a per-URI deadline; once a URI has been quiet for `delay`, inject a
/// `$/functorFlush` notification back onto the server channel so the loop
/// publishes the settled buffer's diagnostics. One thread serves every URI, and
/// idles on a long recv when nothing is pending.
fn debounce_publisher(rx: Receiver<String>, flush: Sender<Incoming>, delay: std::time::Duration) {
    let mut deadlines: HashMap<String, std::time::Instant> = HashMap::new();
    loop {
        let now = std::time::Instant::now();
        let due: Vec<String> = deadlines
            .iter()
            .filter(|(_, &deadline)| deadline <= now)
            .map(|(uri, _)| uri.clone())
            .collect();
        for uri in due {
            deadlines.remove(&uri);
            let message = json!({
                "jsonrpc": "2.0",
                "method": "$/functorFlush",
                "params": { "uri": uri },
            });
            if flush.send(Incoming::Msg(message)).is_err() {
                return; // the server loop is gone
            }
        }
        let timeout = deadlines
            .values()
            .min()
            .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or(std::time::Duration::from_secs(3600));
        match rx.recv_timeout(timeout) {
            Ok(uri) => {
                deadlines.insert(uri, std::time::Instant::now() + delay);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The server loop: read framed messages until `exit` (or EOF), dispatching
/// requests (messages with an `id`) and notifications. Returns the process
/// exit code: per the LSP spec, `exit` after `shutdown` is 0, `exit` without
/// it is 1 (EOF — the client vanished — is a quiet 0).
#[cfg(test)]
fn serve(reader: &mut impl BufRead, writer: &mut impl Write) -> i32 {
    // The synchronous single-source loop used by the tests: stdin only. Attach
    // polling is a `main`-only concern (its trace notifications arrive on the
    // channel `main` drains), so the receiver is dropped here.
    let (trace_tx, _trace_rx) = std::sync::mpsc::channel::<Incoming>();
    run(&mut || read_message(reader), writer, trace_tx, None)
}

/// The server core: pull messages from `next` (stdin in tests; a stdin+attach
/// multiplexed channel in `main`) and dispatch them, writing replies and
/// server-initiated requests to `writer`. `trace_tx` is handed to attach-poll
/// threads so a fetched trace re-enters here as a `functor/inspector/trace`
/// notification.
fn run(
    next: &mut dyn FnMut() -> Option<Value>,
    writer: &mut impl Write,
    trace_tx: Sender<Incoming>,
    debounce: Option<Sender<String>>,
) -> i32 {
    let mut shutdown_seen = false;
    let mut documents: HashMap<String, String> = HashMap::new();
    // The last-good parsed project per URI, for completion. Completion fires on
    // code that does not parse (`let s = Scene.`), where `load_project` returns
    // `None`; keeping the previous good load lets us still answer. A failed
    // refresh retains the previous entry — that IS "last good".
    let mut projects: HashMap<String, functor_lang::project::Project> = HashMap::new();
    // Inspector overlay state: the latest trace (parsed + its raw params, for
    // change detection), the per-entry selected execution index (raw; reduced
    // mod count at display), a monotonic id for server→client requests, and
    // the live attach poller's cancel flag.
    let mut trace: Option<TraceDoc> = None;
    let mut last_trace_params: Option<Value> = None;
    let mut selected: HashMap<String, usize> = HashMap::new();
    let mut next_request_id: i64 = 1;
    let mut attach_stop: Option<Arc<AtomicBool>> = None;
    while let Some(message) = next() {
        // A message with an `id` but no `method` is the client's response to one
        // of our server-initiated refresh requests: fire-and-forget, so tolerate
        // the reply arriving whenever and never try to "answer" it.
        if message.get("method").is_none() {
            continue;
        }
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
                        "completionProvider": { "triggerCharacters": ["."] },
                        "executeCommandProvider": {
                            "commands": ["functor.inspector.cycleExecution"],
                        },
                    },
                    "serverInfo": { "name": "functor-lang-lsp" },
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
                // The type hover merged with the live value under the cursor
                // (trace v2's inline-vs-hover policy: previews render inline,
                // the FULL value lives here).
                let result = documents
                    .contains_key(uri)
                    .then(|| {
                        hover_with_live(
                            uri,
                            &documents,
                            trace.as_ref(),
                            &selected,
                            &params["position"],
                        )
                    })
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
                // Type hints (project-derived) merged with live-value hints
                // (trace-derived, hash-gated) — the two are independent so live
                // values still show even when the buffer fails to check.
                let mut hints = documents
                    .contains_key(uri)
                    .then(|| inlay_hints(uri, &documents, &params["range"]))
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                hints.extend(live_inlay_hints(
                    uri,
                    &documents,
                    trace.as_ref(),
                    &selected,
                    &params["range"],
                ));
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": Value::Array(hints) }),
                );
            }
            ("textDocument/codeLens", Some(id)) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                // Signature lenses merged with the execution-picker lens.
                let mut lenses = documents
                    .contains_key(uri)
                    .then(|| code_lenses(uri, &documents))
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                lenses.extend(picker_code_lenses(
                    uri,
                    &documents,
                    trace.as_ref(),
                    &selected,
                ));
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": Value::Array(lenses) }),
                );
            }
            ("workspace/executeCommand", Some(id)) => {
                if params["command"].as_str() == Some("functor.inspector.cycleExecution") {
                    cycle_execution(&params["arguments"], trace.as_ref(), &mut selected);
                    refresh_overlays(writer, &mut next_request_id);
                }
                write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null }),
                );
            }
            ("textDocument/completion", Some(id)) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                // No reload here: didOpen/didChange already refreshed the cache
                // from this same `documents` state, so it is as fresh as the
                // live buffer allows — a parseable buffer refreshed it, a
                // broken one kept the last good load. Completion is the hot
                // keystroke path; don't reparse the project a second time.
                let result = projects
                    .get(uri)
                    .and_then(|project| completion(project, uri, &documents, &params["position"]))
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
                if let Some(project) = load_project(uri, &documents) {
                    projects.insert(uri.to_string(), project);
                }
                // A just-opened matching document gets the current coverage.
                push_coverage(writer, trace.as_ref(), &documents);
            }
            ("textDocument/didChange", None) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                // Full sync: the last content change is the whole new buffer.
                if let Some(text) = params["contentChanges"]
                    .as_array()
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str())
                {
                    documents.insert(uri.to_string(), text.to_string());
                    // Diagnostics are debounced so typing doesn't flash squiggles:
                    // signal the debounce thread, which flushes via `$/functorFlush`
                    // once the buffer settles. With no debouncer (tests), publish now.
                    match &debounce {
                        Some(change_tx) => {
                            let _ = change_tx.send(uri.to_string());
                        }
                        None => publish_diagnostics(writer, uri, text),
                    }
                    if let Some(project) = load_project(uri, &documents) {
                        projects.insert(uri.to_string(), project);
                    }
                    // The edit changed the hash: the gate re-evaluates (and
                    // the client's gutter clears via the empty push).
                    push_coverage(writer, trace.as_ref(), &documents);
                }
            }
            ("textDocument/didClose", None) => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                documents.remove(uri);
                projects.remove(uri);
                // Clear stale squiggles for the closed file.
                write_diagnostics(writer, uri, vec![]);
            }
            // A debounce-thread flush: `uri`'s buffer has settled — publish its
            // diagnostics from the latest stored document.
            ("$/functorFlush", None) => {
                let uri = params["uri"].as_str().unwrap_or("");
                if let Some(text) = documents.get(uri) {
                    publish_diagnostics(writer, uri, text);
                }
            }
            // A pushed trace (extension relay, wasm postMessage, tests, or the
            // attach poller re-entering): replace the overlay and refresh.
            // Refresh is event-driven — a document identical to the last one
            // (e.g. an idle unpaused poll returning the same tiny `paused:false`
            // JSON) causes no rebuild and no refresh.
            ("functor/inspector/trace", None) => {
                if last_trace_params.as_ref() == Some(params) {
                    continue;
                }
                if let Some(doc) = TraceDoc::from_json(params) {
                    last_trace_params = Some(params.clone());
                    trace = Some(doc);
                    refresh_overlays(writer, &mut next_request_id);
                    push_coverage(writer, trace.as_ref(), &documents);
                }
            }
            // Native refresh: poll `GET /trace` on a background thread. Attach
            // persists until `{"port": null}` (or any non-number) detaches.
            ("functor/inspector/attach", None) => {
                if let Some(stop) = attach_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                if let Some(port) = params["port"].as_u64() {
                    let stop = Arc::new(AtomicBool::new(false));
                    attach_stop = Some(stop.clone());
                    let tx = trace_tx.clone();
                    std::thread::spawn(move || poll_attached(port as u16, tx, stop));
                }
            }
            // initialized, $/… and any other notification: deliberately ignored.
            (_, None) => {}
        }
    }
    0
}

/// Run the Functor Lang front-end over `text` and publish the outcome: one diagnostic
/// for the first parse/lower failure, every `functor_lang::check` type diagnostic for
/// a lowered module, or an empty list (clearing squiggles) when clean.
fn publish_diagnostics(writer: &mut impl Write, uri: &str, text: &str) {
    let diagnostic = |message: &str, span: functor_lang::Span| {
        json!({
            "range": span_to_range(text, span),
            "severity": 1, // Error
            "source": "functor-lang",
            "message": message,
        })
    };
    // Parse and lowering stop at the first error; a clean module then gets
    // ALL of the gradual checker's type diagnostics.
    let parsed = if uri_to_path(uri)
        .and_then(|path| path.extension().map(|extension| extension == "funi"))
        .unwrap_or(false)
    {
        functor_lang::parse_interface(text)
    } else {
        functor_lang::parse(text)
    };
    let diagnostics = match parsed {
        Err(err) => vec![diagnostic(&err.message, err.span)],
        Ok(program) => match functor_lang::lower(program) {
            Err(err) => vec![diagnostic(&err.message, err.span)],
            Ok(module) => functor_lang::check(&module)
                .into_iter()
                .map(|err| diagnostic(&err.message, err.span))
                .collect(),
        },
    };
    write_diagnostics(writer, uri, diagnostics);
}

/// Load the Functor Lang program an open document belongs to. With a `functor.json`
/// above it, that's the whole multi-file project — every open buffer stands
/// in for its on-disk file (unsaved edits count), siblings load from disk.
/// With no `functor.json`, it's just this buffer as a single-file project
/// (the directory is NOT scanned — unrelated `.fun` files nearby must not
/// leak in). `None` on a non-`file:` URI or a load failure.
fn load_project(uri: &str, documents: &HashMap<String, String>) -> Option<functor_lang::project::Project> {
    let path = uri_to_path(uri)?;
    // The engine prelude (`Scene.*`, `Camera.*`, …) as a check-time overlay, so
    // the editor shows real host types (`Scene.cube() : Scene.t`) instead of
    // `Unknown`. This is what makes the LSP host-aware; the runtime injects the
    // same set (funi 2e).
    let prelude = functor_prelude::modules();
    let single_file = || functor_lang::project::load_single_file(&path, documents.get(uri)?, &prelude).ok();
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
            match functor_lang::project::load_with_prelude(&entry, &overrides, &prelude) {
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
    let (span, hover_text) = functor_lang::hover::hover_text(&project.module, &types, offset)?;
    let (_, range) = localize(&project, span)?;
    Some(json!({
        "contents": { "kind": "markdown", "value": format!("```functor\n{hover_text}\n```") },
        "range": range,
    }))
}

/// The type hover merged with the live value under the cursor. Either half
/// can be absent: a live value with no type hover still answers (values show
/// even when the buffer doesn't check — the live-hint precedent), and vice
/// versa. The live value renders FIRST (it's the immediate question when
/// paused), the type after.
fn hover_with_live(
    uri: &str,
    documents: &HashMap<String, String>,
    trace: Option<&TraceDoc>,
    selected: &HashMap<String, usize>,
    position: &Value,
) -> Option<Value> {
    let type_hover = hover(uri, documents, position);
    let live = live_hover_text(uri, documents, trace, selected, position);
    match (type_hover, live) {
        (Some(mut h), Some(live)) => {
            let existing = h["contents"]["value"].as_str().unwrap_or("").to_string();
            h["contents"]["value"] =
                json!(format!("```functor\n{live}\n```\n\n{existing}"));
            Some(h)
        }
        (h @ Some(_), None) => h,
        (None, Some(live)) => Some(json!({
            "contents": { "kind": "markdown", "value": format!("```functor\n{live}\n```") },
        })),
        (None, None) => None,
    }
}

/// The live value under the cursor, from the trace (hash-gated, selected
/// executions) — mirrors `live_inlay_hints`' file/offset mapping.
fn live_hover_text(
    uri: &str,
    documents: &HashMap<String, String>,
    trace: Option<&TraceDoc>,
    selected: &HashMap<String, usize>,
    position: &Value,
) -> Option<String> {
    let trace = trace?;
    let path = uri_to_path(uri)?;
    let file_name = match_trace_file(trace, &path)?;
    let source = source_text(uri, documents, &path);
    let offset = position_to_offset(&source, position)?;
    let select = |entry: &str| selected.get(entry).copied().unwrap_or(0);
    inspector::live_hover(trace, &file_name, &source, offset, &select)
}

/// Answer a definition request: load the project, resolve the reference at
/// the position via `functor_lang::goto`, and return the definition site as a
/// `Location`. The target may live in a **sibling file** (cross-file goto),
/// so the location's URI is whichever file owns the resolved span.
fn definition(uri: &str, documents: &HashMap<String, String>, position: &Value) -> Option<Value> {
    let project = load_project(uri, documents)?;
    let file = project.sources.file_by_path(&uri_to_path(uri)?)?;
    let offset = file.base + position_to_offset(&file.src, position)?;
    let span = functor_lang::goto::definition_span(&project.module, offset)?;
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
    let lenses: Vec<Value> = functor_lang::codelens::signatures(&project.module, &types)
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

/// The live-value inlay hints for `uri` as LSP hints. Independent of the
/// project load — it needs only the open-buffer (or on-disk) source text and
/// the hash-gated trace — so live values still show when the buffer fails to
/// check. Merged with the type hints by the caller.
fn live_inlay_hints(
    uri: &str,
    documents: &HashMap<String, String>,
    trace: Option<&TraceDoc>,
    selected: &HashMap<String, usize>,
    range: &Value,
) -> Vec<Value> {
    let Some(trace) = trace else {
        return Vec::new();
    };
    let Some(path) = uri_to_path(uri) else {
        return Vec::new();
    };
    let Some(file_name) = match_trace_file(trace, &path) else {
        return Vec::new();
    };
    let source = source_text(uri, documents, &path);
    let select = |entry: &str| selected.get(entry).copied().unwrap_or(0);
    // The client's range is local to the file; live-hint offsets are already
    // file-local trace byte offsets, so both compare directly (no `base`).
    let start = position_to_offset(&source, &range["start"]);
    let end = position_to_offset(&source, &range["end"]);
    let in_range = |offset: usize| match (start, end) {
        (Some(s), Some(e)) => s <= offset && offset < e,
        _ => true,
    };
    inspector::live_hints(trace, &file_name, &source, &select)
        .into_iter()
        .filter(|hint| in_range(hint.offset))
        .map(|hint| {
            json!({
                "position": lsp_position(&source, hint.offset),
                "label": hint.label,
                "paddingLeft": true,
                "paddingRight": false,
            })
        })
        .collect()
}

/// The execution-picker lenses for `uri`: one clickable lens per entry-point
/// def whose name the trace recorded, cycling executions via
/// `functor.inspector.cycleExecution`. The source-hash gate lives in the pure
/// half; on a mismatch (or no trace) this is empty.
fn picker_code_lenses(
    uri: &str,
    documents: &HashMap<String, String>,
    trace: Option<&TraceDoc>,
    selected: &HashMap<String, usize>,
) -> Vec<Value> {
    let Some(trace) = trace else {
        return Vec::new();
    };
    let Some(project) = load_project(uri, documents) else {
        return Vec::new();
    };
    let Some(path) = uri_to_path(uri) else {
        return Vec::new();
    };
    let Some(file) = project.sources.file_by_path(&path) else {
        return Vec::new();
    };
    let Some(file_name) = match_trace_file(trace, &path) else {
        return Vec::new();
    };
    // Every top-level def in this file (project-wide span); the pure half keeps
    // only those the trace has an invocation for.
    let entry_defs: Vec<(String, functor_lang::Span)> = project
        .module
        .defs
        .iter()
        .filter(|def| owns(file, def.span.start))
        .map(|def| (def.name.clone(), def.span))
        .collect();
    let select = |entry: &str| selected.get(entry).copied().unwrap_or(0);
    inspector::picker_lenses(trace, &file_name, &file.src, &entry_defs, &select)
        .into_iter()
        .map(|lens| {
            json!({
                "range": local_range(file, lens.span),
                "command": {
                    "title": lens.title,
                    "command": "functor.inspector.cycleExecution",
                    "arguments": [lens.file, lens.entry, lens.current_index],
                },
            })
        })
        .collect()
}

/// Advance the selected execution for a `cycleExecution` command by one (mod
/// the entry's execution count). Args are `[file, entry, currentIndex]`; the
/// clicked lens's `currentIndex` is authoritative.
fn cycle_execution(
    arguments: &Value,
    trace: Option<&TraceDoc>,
    selected: &mut HashMap<String, usize>,
) {
    let Some(args) = arguments.as_array() else {
        return;
    };
    let Some(entry) = args.get(1).and_then(|v| v.as_str()) else {
        return;
    };
    let current = args.get(2).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let count = trace
        .map(|t| inspector::execution_count(t, entry))
        .unwrap_or(0);
    if count > 0 {
        selected.insert(entry.to_string(), (current + 1) % count);
    }
}

/// Ask the client to re-pull inlay hints and code lenses — server-initiated
/// requests with ids from the server counter. Fire-and-forget: the client's
/// responses are tolerated whenever (and ignored on arrival).
fn refresh_overlays(writer: &mut impl Write, next_request_id: &mut i64) {
    for method in ["workspace/inlayHint/refresh", "workspace/codeLens/refresh"] {
        let id = *next_request_id;
        *next_request_id += 1;
        write_message(
            writer,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method }),
        );
    }
}

/// Push the recency-gutter coverage to the client — a custom NOTIFICATION
/// (`functor/inspector/coverage`, no id: fire-and-forget) with per-line
/// states for every OPEN document the trace covers. Hash-gated per file by
/// the pure half; a document that no longer matches gets an explicit empty
/// list so the client clears its gutter (never stale colors on wrong lines).
fn push_coverage(
    writer: &mut impl Write,
    trace: Option<&TraceDoc>,
    documents: &HashMap<String, String>,
) {
    // No trace yet → nothing to draw OR clear (the client starts empty), and
    // pushing would inject notifications into sessions that never touch the
    // inspector (the stdio e2e reads the stream in strict order).
    if trace.is_none() {
        return;
    }
    for (uri, text) in documents {
        let lines: Vec<Value> = trace
            .and_then(|t| {
                let path = uri_to_path(uri)?;
                let file_name = match_trace_file(t, &path)?;
                Some(
                    inspector::coverage_lines(t, &file_name, text)
                        .into_iter()
                        .map(|(line, state)| json!({ "line": line, "state": state }))
                        .collect(),
                )
            })
            .unwrap_or_default();
        write_message(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "method": "functor/inspector/coverage",
                "params": { "uri": uri, "lines": lines },
            }),
        );
    }
}

/// The server's current text for a file: the open buffer wins over disk, so
/// unsaved edits participate in the source-hash gate.
fn source_text(uri: &str, documents: &HashMap<String, String>, path: &Path) -> String {
    documents
        .get(uri)
        .cloned()
        .unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default())
}

/// The trace-relative file name (e.g. `game.fun`) whose path suffix matches
/// `path`. The wire contract keys sources/bindings by project-relative name;
/// the LSP holds absolute paths, so we match on a `/`-boundary suffix.
fn match_trace_file(trace: &TraceDoc, path: &Path) -> Option<String> {
    let path = path.to_string_lossy();
    trace
        .sources
        .iter()
        .find(|source| {
            let file = source.file.as_str();
            path == file || path.ends_with(&format!("/{file}"))
        })
        .map(|source| source.file.clone())
}

/// Fetch and parse `GET http://<addr>/trace`. Hand-rolled HTTP/1.1 over TCP
/// (`Connection: close`, so read-to-EOF yields the whole body) to avoid an
/// HTTP-client dependency — the endpoint is always the localhost debug server,
/// whose tiny_http responses always carry a Content-Length body (never
/// chunked), so the bytes after the header block are the JSON. `None` on any
/// connection/read/parse failure.
fn fetch_trace(addr: &str) -> Option<Value> {
    let mut stream = std::net::TcpStream::connect(addr).ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .ok()?;
    let request = format!("GET /trace HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let split = find_subslice(&response, b"\r\n\r\n")?;
    serde_json::from_slice(&response[split + 4..]).ok()
}

/// The index of the first occurrence of `needle` in `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Poll `GET /trace` on `127.0.0.1:<port>`, re-injecting each *changed* trace
/// as a `functor/inspector/trace` notification on `tx`. Attach **persists
/// until explicit detach** (`stop`, or a dropped receiver): it fetches once on
/// attach, polls ~2Hz while the last response said `paused: true`, and idles
/// ~1Hz when unpaused (or unreachable) — never self-stopping, so a fresh pause
/// is discovered without any extension-side signal. Event-driven-ness governs
/// *refresh*, not discovery: a response identical to the last one is not
/// re-sent (and the trace handler dedupes again for the push path), so an idle
/// unpaused poll of the tiny `paused:false` JSON causes no downstream work.
fn poll_attached(port: u16, tx: Sender<Incoming>, stop: Arc<AtomicBool>) {
    let addr = format!("127.0.0.1:{port}");
    let mut paused = false;
    let mut last_sent: Option<Value> = None;
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if let Some(doc) = fetch_trace(&addr) {
            paused = doc["paused"].as_bool().unwrap_or(false);
            if last_sent.as_ref() != Some(&doc) {
                let message = json!({
                    "jsonrpc": "2.0",
                    "method": "functor/inspector/trace",
                    "params": doc.clone(),
                });
                if tx.send(Incoming::Msg(message)).is_err() {
                    return;
                }
                last_sent = Some(doc);
            }
        }
        // ~2Hz paused, ~1Hz idle; wake promptly on detach.
        let ticks = if paused { 5 } else { 10 };
        for _ in 0..ticks {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
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
    let hints: Vec<Value> = functor_lang::inlay::inlay_hints(&project.module, &types)
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

/// Answer a completion request. Unlike hover/definition, the offset stays
/// **local** to the live buffer (no `file.base +`): `functor_lang::complete`
/// derives context textually from that buffer, while candidates come from the
/// (possibly stale) last-good `project`. `current_module` is the open file's
/// module (a sibling's own defs are referenced bare), falling back to the
/// project entry.
fn completion(
    project: &functor_lang::project::Project,
    uri: &str,
    documents: &HashMap<String, String>,
    position: &Value,
) -> Option<Value> {
    let text = documents.get(uri)?;
    let offset = position_to_offset(text, position)?;
    let current_module = uri_to_path(uri)
        .and_then(|path| {
            project
                .sources
                .file_by_path(&path)
                .map(|file| file.module.clone())
        })
        .unwrap_or_else(|| project.entry.clone());
    let items = functor_lang::complete::complete(project, &current_module, text, offset);
    if items.is_empty() {
        return None;
    }
    let items: Vec<Value> = items
        .into_iter()
        .map(|item| {
            json!({
                "label": item.label,
                "kind": kind_code(item.kind),
                "detail": item.detail,
            })
        })
        .collect();
    Some(Value::Array(items))
}

/// The LSP `CompletionItemKind` code for a completion kind.
fn kind_code(kind: functor_lang::complete::CompletionKind) -> i64 {
    use functor_lang::complete::CompletionKind;
    match kind {
        CompletionKind::Function => 3,
        CompletionKind::Constructor => 4,
        CompletionKind::Field => 5,
        CompletionKind::Module => 9,
        CompletionKind::Value => 12,
        CompletionKind::Keyword => 14,
    }
}

/// Whether project-wide `offset` falls in `file` (its half-open base range).
fn owns(file: &functor_lang::project::SourceFile, offset: usize) -> bool {
    file.base <= offset && offset <= file.base + file.src.len()
}

/// A project-wide span → an LSP range local to the file it belongs to, plus
/// that file's URI. Spans never straddle files (each node is lowered from one
/// file), so both ends localize against the start file.
fn localize(project: &functor_lang::project::Project, span: functor_lang::Span) -> Option<(String, Value)> {
    let file = project.sources.file_at(span.start);
    Some((path_to_uri(&file.path), local_range(file, span)))
}

/// A project-wide span → an LSP range in `file`'s local coordinates.
fn local_range(file: &functor_lang::project::SourceFile, span: functor_lang::Span) -> Value {
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

/// Convert a byte-offset [`functor_lang::Span`] to an LSP range. The span is
/// half-open, matching LSP's exclusive `end`. LSP positions count characters
/// in **UTF-16 code units** (the default encoding; this server doesn't
/// negotiate another), not chars — an astral-plane character earlier on the
/// line counts as two.
fn span_to_range(text: &str, span: functor_lang::Span) -> Value {
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
        let range = span_to_range(text, functor_lang::Span::new(4, 5));
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
        let range = span_to_range(text, functor_lang::Span::new(14, 19));
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

    #[test]
    fn interface_diagnostics_use_interface_parser() {
        let mut output = Vec::new();
        publish_diagnostics(
            &mut output,
            "file:///project/widget.funi",
            "type Handle\nlet size : (Handle) => float",
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(r#""diagnostics":[]"#), "{output}");
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
        let path = std::path::Path::new("/Users/café/game.fun");
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
        let open = r#"{"method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///x.fun","text":"let = 3"}}}"#;
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

#[cfg(test)]
mod inspector_server_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::net::TcpListener;
    use std::time::Duration;

    // The entry-point fixture: a `let` binder inside `update` whose recorded
    // value the inspector renders inline.
    const FIXTURE: &str = "let update = (model, msg) =>\n  let velocity = model in\n  velocity\n";
    const URI: &str = "file:///tmp/functor_lsp_inspector_test_xyz/game.fun";

    // Drive `run` to completion over a fixed message queue, returning every
    // message the server wrote (responses, notifications, and its own
    // server→client requests), in order.
    fn drive(messages: Vec<Value>) -> Vec<Value> {
        let mut queue: VecDeque<Value> = messages.into();
        let mut out: Vec<u8> = Vec::new();
        let (trace_tx, _rx) = std::sync::mpsc::channel::<Incoming>();
        run(&mut || queue.pop_front(), &mut out, trace_tx, None);
        let mut reader: &[u8] = &out;
        let mut parsed = Vec::new();
        while let Some(message) = read_message(&mut reader) {
            parsed.push(message);
        }
        parsed
    }

    // The `velocity` let-binder as a REGION span (start of the inner `let`
    // through the value expr's start) — the PR1 convention.
    fn velocity_region() -> (usize, usize, usize) {
        let region_start = FIXTURE.match_indices("let").nth(1).unwrap().0;
        let name_pos = region_start + FIXTURE[region_start..].find("velocity").unwrap();
        let value_pos = name_pos + FIXTURE[name_pos..].find("model").unwrap();
        (region_start, value_pos, name_pos + "velocity".len())
    }

    // A 5-execution `update` trace whose hash matches `text`; execution 0 binds
    // `velocity`, the rest carry only distinct provenance for the picker.
    fn trace_message(text: &str) -> Value {
        let (start, end, _) = velocity_region();
        let mut invocations = vec![json!({
            "entry": "update", "index": 0, "count": 5, "provenance": "subscription: Tick",
            "ghost": false, "result": "0",
            "bindings": [{
                "name": "velocity", "file": "game.fun",
                "start": start, "end": end,
                "value": "{ x = 0.0, y = -9.8 }", "count": 1
            }]
        })];
        for (i, prov) in ["effect result: Pong", "input: Space down", "mouseMove", "collision: ground"]
            .iter()
            .enumerate()
        {
            invocations.push(json!({
                "entry": "update", "index": i + 1, "count": 5, "provenance": prov,
                "ghost": false, "result": "0", "bindings": []
            }));
        }
        json!({
            "jsonrpc": "2.0", "method": "functor/inspector/trace",
            "params": {
                "paused": true,
                "sources": [ { "file": "game.fun", "hash": inspector::sha256_hex(text.as_bytes()) } ],
                "invocations": invocations,
            }
        })
    }

    fn open(uri: &str, text: &str) -> Value {
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "languageId": "functor-lang", "version": 1, "text": text } }
        })
    }

    fn change(uri: &str, version: i64, text: &str) -> Value {
        json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [ { "text": text } ]
            }
        })
    }

    fn inlay_req(id: i64, uri: &str) -> Value {
        json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/inlayHint",
            "params": {
                "textDocument": { "uri": uri },
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 100, "character": 0 } }
            }
        })
    }

    fn codelens_req(id: i64, uri: &str) -> Value {
        json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/codeLens",
            "params": { "textDocument": { "uri": uri } }
        })
    }

    fn response(messages: &[Value], id: i64) -> Value {
        messages
            .iter()
            .find(|m| m["id"] == json!(id) && m.get("result").is_some())
            .unwrap_or_else(|| panic!("no response for id {id} in {messages:#?}"))
            .clone()
    }

    // Does a code-lens result carry a picker lens with this execution string?
    fn has_picker(result: &Value, needle: &str) -> bool {
        result.as_array().is_some_and(|lenses| {
            lenses.iter().any(|lens| {
                lens["command"]["command"] == json!("functor.inspector.cycleExecution")
                    && lens["command"]["title"].as_str().is_some_and(|t| t.contains(needle))
            })
        })
    }

    fn has_live_value(result: &Value) -> bool {
        result.as_array().is_some_and(|hints| {
            hints
                .iter()
                .any(|h| h["label"].as_str().is_some_and(|l| l.starts_with("= ")))
        })
    }

    #[test]
    fn initialize_advertises_the_cycle_command() {
        let out = drive(vec![json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} }
        })]);
        assert_eq!(
            response(&out, 1)["result"]["capabilities"]["executeCommandProvider"]["commands"],
            json!(["functor.inspector.cycleExecution"])
        );
    }

    #[test]
    fn trace_yields_live_hints_and_a_picker_that_cycles_and_gates_on_edit() {
        let mutated = format!("{FIXTURE}\n"); // hash-breaking edit
        let out = drive(vec![
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            open(URI, FIXTURE),
            trace_message(FIXTURE),
            inlay_req(2, URI),
            codelens_req(3, URI),
            // Cycle update's execution 0 → 1.
            json!({
                "jsonrpc": "2.0", "id": 4, "method": "workspace/executeCommand",
                "params": { "command": "functor.inspector.cycleExecution", "arguments": ["game.fun", "update", 0] }
            }),
            codelens_req(5, URI),
            change(URI, 2, &mutated),
            inlay_req(6, URI),
            codelens_req(7, URI),
        ]);

        // Live value hint appears at the binder name, hash-gated open.
        let inlay2 = response(&out, 2)["result"].clone();
        assert!(has_live_value(&inlay2), "expected a live value hint: {inlay2}");
        let (_, _, name_end) = velocity_region();
        let hit = inlay2
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["label"] == json!("= { x = 0.0, y = -9.8 }"))
            .expect("velocity hint");
        assert_eq!(hit["position"], lsp_position(FIXTURE, name_end));

        // Picker lens starts at execution 1/5 with execution 0's provenance.
        assert!(
            has_picker(&response(&out, 3)["result"], "update — execution 1/5 ▸ [subscription: Tick]"),
            "codeLens id 3: {:?}", response(&out, 3)["result"]
        );

        // After the cycle, the lens advances to 2/5 with execution 1's provenance.
        assert!(
            has_picker(&response(&out, 5)["result"], "update — execution 2/5 ▸ [effect result: Pong]"),
            "codeLens id 5: {:?}", response(&out, 5)["result"]
        );

        // A trace change AND the executeCommand each pushed a codeLens refresh.
        let refreshes = out
            .iter()
            .filter(|m| m["method"] == json!("workspace/codeLens/refresh"))
            .count();
        assert!(refreshes >= 2, "expected >=2 codeLens refreshes, got {refreshes}");
        assert!(
            out.iter().any(|m| m["method"] == json!("workspace/inlayHint/refresh")),
            "expected an inlayHint refresh"
        );

        // The hash-breaking edit closes the gate: no live hints, no picker.
        assert!(!has_live_value(&response(&out, 6)["result"]), "live hints must vanish on edit");
        assert!(
            !has_picker(&response(&out, 7)["result"], "execution"),
            "picker must vanish on edit"
        );
    }

    #[test]
    fn client_response_to_a_refresh_is_tolerated() {
        // A message with an `id` but no `method` is the client's reply to our
        // refresh request; it must be ignored, not answered with an error.
        let out = drive(vec![
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 42, "result": null }),
        ]);
        assert!(
            !out.iter().any(|m| m["id"] == json!(42)),
            "a client response must not be answered: {out:#?}"
        );
    }

    // A minimal one-shot HTTP server that replies with `body` and closes.
    fn stub_server(body: String) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut scratch = [0u8; 1024];
            let _ = sock.read(&mut scratch);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(response.as_bytes()).unwrap();
        });
        (addr, handle)
    }

    #[test]
    fn fetch_trace_reads_and_parses_a_stub_server() {
        let (addr, handle) = stub_server(
            r#"{"paused":true,"sources":[],"invocations":[]}"#.to_string(),
        );
        let trace = fetch_trace(&addr).expect("fetch");
        assert_eq!(trace["paused"], json!(true));
        handle.join().unwrap();
    }

    #[test]
    fn poll_attached_idles_slowly_while_unpaused_and_discovers_the_next_pause() {
        // A server that answers unpaused, unpaused (identical — must be
        // deduplicated, not re-delivered), then paused; then it goes away
        // (the poller must survive that too — attach persists until detach).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for paused in ["false", "false", "true"] {
                let (mut sock, _) = listener.accept().unwrap();
                let mut scratch = [0u8; 512];
                let _ = sock.read(&mut scratch);
                let body = format!(r#"{{"paused":{paused},"sources":[],"invocations":[]}}"#);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                sock.write_all(response.as_bytes()).unwrap();
            }
        });

        let (tx, rx) = std::sync::mpsc::channel::<Incoming>();
        let stop = Arc::new(AtomicBool::new(false));
        let poller_stop = stop.clone();
        let poller = std::thread::spawn(move || poll_attached(port, tx, poller_stop));
        let recv_msg = |rx: &std::sync::mpsc::Receiver<Incoming>, secs: u64| match rx
            .recv_timeout(Duration::from_secs(secs))
        {
            Ok(Incoming::Msg(v)) => Some(v),
            _ => None,
        };

        // Fetch-once-on-attach delivers the unpaused doc.
        let started = std::time::Instant::now();
        let first = recv_msg(&rx, 3).expect("first trace");
        assert_eq!(first["method"], json!("functor/inspector/trace"));
        assert_eq!(first["params"]["paused"], json!(false));
        // The identical second response is deduplicated; the NEXT delivery is
        // the discovered pause — after two ~1s idle waits, never self-stopped.
        let second = recv_msg(&rx, 6).expect("the discovered pause");
        assert_eq!(second["params"]["paused"], json!(true));
        assert!(
            started.elapsed() >= Duration::from_millis(1500),
            "unpaused polling must idle ~1Hz, got {:?}",
            started.elapsed()
        );

        // Only explicit detach ends the poller (the server is already gone).
        stop.store(true, Ordering::SeqCst);
        poller.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn unchanged_trace_emits_no_refresh() {
        // Event-driven refresh: a repeated identical trace document (the idle
        // unpaused poll case) must cause no refresh; a changed one must.
        let unpaused = json!({
            "jsonrpc": "2.0", "method": "functor/inspector/trace",
            "params": { "paused": false, "sources": [], "invocations": [] }
        });
        let out = drive(vec![
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            unpaused.clone(),
            unpaused.clone(), // identical — no refresh, no work
            trace_message(FIXTURE), // changed — refresh again
        ]);
        let count = |method: &str| {
            out.iter()
                .filter(|m| m["method"] == json!(method))
                .count()
        };
        assert_eq!(
            count("workspace/inlayHint/refresh"),
            2,
            "one refresh per CHANGED trace: {out:#?}"
        );
        assert_eq!(count("workspace/codeLens/refresh"), 2);
    }
}
