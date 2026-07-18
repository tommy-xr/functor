//! End-to-end test: spawn the real `functor-lang-lsp` binary and speak framed LSP to
//! it over stdin/stdout — initialize, open a broken document, assert the
//! diagnostic, fix it, assert the clear, round-trip a hover and a
//! definition (hit and null), and check unknown requests get
//! MethodNotFound without killing the server.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Server {
    fn spawn() -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_functor-lang-lsp"))
            // These tests assert diagnostic content, not typing cadence: disable
            // the didChange debounce so publishes are immediate and deterministic.
            .env("FUNCTOR_LSP_DIAGNOSTICS_DEBOUNCE_MS", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn functor-lang-lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Server {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Value {
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("read header");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                content_length = value.trim().parse().expect("content length");
            }
        }
        let mut body = vec![0; content_length];
        self.stdout.read_exact(&mut body).expect("read body");
        serde_json::from_slice(&body).expect("parse body")
    }
}

const URI: &str = "file:///tmp/e2e.fun";

#[test]
fn diagnostics_over_real_stdio() {
    let mut server = Server::spawn();

    // initialize → full-sync capability advertised.
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} },
    }));
    let response = server.recv();
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["capabilities"]["textDocumentSync"], 1);
    assert_eq!(
        response["result"]["capabilities"]["definitionProvider"],
        true
    );
    assert_eq!(
        response["result"]["capabilities"]["inlayHintProvider"],
        true
    );
    assert_eq!(
        response["result"]["capabilities"]["codeLensProvider"]["resolveProvider"],
        false
    );
    assert_eq!(
        response["result"]["capabilities"]["completionProvider"]["triggerCharacters"],
        json!(["."])
    );

    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));

    // didOpen with a broken document → one Error diagnostic at the `=`
    // (bytes 4..5, i.e. 0-based line 0 chars 4..5).
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": URI, "languageId": "functor-lang", "version": 1, "text": "let = 3",
        } },
    }));
    let publish = server.recv();
    assert_eq!(publish["method"], "textDocument/publishDiagnostics");
    assert_eq!(publish["params"]["uri"], URI);
    let diagnostics = publish["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "expected one diagnostic: {publish}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["severity"], 1);
    assert_eq!(
        diagnostic["message"],
        "expected a name after `let`, found `=`"
    );
    assert_eq!(
        diagnostic["range"],
        json!({
            "start": { "line": 0, "character": 4 },
            "end": { "line": 0, "character": 5 },
        })
    );

    // An unknown request gets MethodNotFound and the server keeps serving.
    server.send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/implementation",
        "params": {},
    }));
    let response = server.recv();
    assert_eq!(response["id"], 2);
    assert_eq!(response["error"]["code"], -32601);

    // didChange to valid source → diagnostics clear (empty array).
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": URI, "version": 2 },
            "contentChanges": [ { "text": "let x = 3" } ],
        },
    }));
    let publish = server.recv();
    assert_eq!(publish["method"], "textDocument/publishDiagnostics");
    assert_eq!(publish["params"]["uri"], URI);
    assert_eq!(publish["params"]["diagnostics"], json!([]));

    // Hover over the now-valid document: quick info for `x` at line 0 col 4.
    server.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": URI },
            "position": { "line": 0, "character": 4 },
        },
    }));
    let response = server.recv();
    assert_eq!(response["id"], 3);
    assert_eq!(
        response["result"]["contents"]["value"], "```functor\nx : float\n```",
        "hover response: {response}"
    );

    // didChange to a document with a global reference, for definition.
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": URI, "version": 3 },
            "contentChanges": [ { "text":
                "let double = (x: float): float => x * 2.0\nlet main = () => double(2.0)" } ],
        },
    }));
    assert_eq!(server.recv()["params"]["diagnostics"], json!([]));

    // Definition on `double(2.0)` (line 1 char 17) → the `let double = `
    // region of line 0, as a Location in the same document.
    server.send(json!({
        "jsonrpc": "2.0", "id": 4, "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": URI },
            "position": { "line": 1, "character": 17 },
        },
    }));
    let response = server.recv();
    assert_eq!(response["id"], 4);
    assert_eq!(
        response["result"],
        json!({
            "uri": URI,
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 13 },
            },
        }),
        "definition response: {response}"
    );

    // Definition on the `=` of line 0 (no reference there) → null.
    server.send(json!({
        "jsonrpc": "2.0", "id": 5, "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": URI },
            "position": { "line": 0, "character": 11 },
        },
    }));
    let response = server.recv();
    assert_eq!(response["id"], 5);
    assert_eq!(response["result"], Value::Null, "expected null: {response}");

    // didChange to a document with an unannotated lambda param, for inlay.
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": URI, "version": 4 },
            "contentChanges": [ { "text": "let f = (x) => x + 1.0" } ],
        },
    }));
    assert_eq!(server.recv()["params"]["diagnostics"], json!([]));

    // Inlay hints over line 0 → one `: float` type hint right after the `x`
    // param (byte 10 = line 0 char 10).
    server.send(json!({
        "jsonrpc": "2.0", "id": 6, "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": URI },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 22 },
            },
        },
    }));
    let response = server.recv();
    assert_eq!(response["id"], 6);
    assert_eq!(
        response["result"],
        json!([{
            "position": { "line": 0, "character": 10 },
            "label": ": float",
            "kind": 1,
            "paddingLeft": false,
            "paddingRight": false,
        }]),
        "inlay response: {response}"
    );

    // Code lens over the same document → one inferred-signature lens for `f`,
    // anchored on its `let` line (bytes 0..22 of the single line).
    server.send(json!({
        "jsonrpc": "2.0", "id": 7, "method": "textDocument/codeLens",
        "params": { "textDocument": { "uri": URI } },
    }));
    let response = server.recv();
    assert_eq!(response["id"], 7);
    assert_eq!(
        response["result"],
        json!([{
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 0, "character": 22 },
            },
            "command": { "title": "f : (float) => float", "command": "" },
        }]),
        "codeLens response: {response}"
    );

    // Clean shutdown.
    server.send(json!({ "jsonrpc": "2.0", "id": 8, "method": "shutdown" }));
    assert_eq!(server.recv()["id"], 8);
    server.send(json!({ "jsonrpc": "2.0", "method": "exit" }));
    let status = server.child.wait().expect("wait for exit");
    assert!(status.success(), "server exited with {status}");
}

/// Project-aware (Track D, slice 1b): a real `functor.json` project on disk
/// with an entry that references a sibling. Hover on the cross-module call
/// shows the sibling's inferred signature, and go-to-definition jumps to the
/// sibling file — the two headline multi-file wins, over the real binary.
#[test]
fn project_aware_hover_and_cross_file_definition() {
    // A scratch project: game.fun (entry) calls Utils.double from utils.fun.
    let dir = std::env::temp_dir().join(format!(
        "functor-lang-lsp-e2e-project-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(
        dir.join("functor.json"),
        r#"{"language": "functor-lang","entry":"game.fun"}"#,
    )
    .unwrap();
    let game = "let apply = (n) => Utils.double(n)\n";
    std::fs::write(dir.join("game.fun"), game).unwrap();
    std::fs::write(
        dir.join("utils.fun"),
        "let double = (x: float): float => x * 2.0\n",
    )
    .unwrap();
    let game_uri = format!("file://{}/game.fun", dir.display());
    let utils_uri = format!("file://{}/utils.fun", dir.display());

    let mut server = Server::spawn();
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} },
    }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": game_uri, "languageId": "functor-lang", "version": 1, "text": game,
        } },
    }));
    server.recv(); // publishDiagnostics (clean)

    // Hover on the `Utils.double` call (char 19 = the `U`): the sibling's
    // inferred signature, resolved across the file boundary.
    let hover_col = game.find("Utils").unwrap() as i64;
    server.send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": game_uri },
            "position": { "line": 0, "character": hover_col + 6 }, // inside `double`
        },
    }));
    let response = server.recv();
    assert_eq!(
        response["result"]["contents"]["value"], "```functor\nUtils.double : (float) => float\n```",
        "cross-file hover: {response}"
    );

    // Go-to-definition on the same reference → a Location in utils.fun, NOT
    // the entry: cross-file navigation.
    server.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": game_uri },
            "position": { "line": 0, "character": hover_col + 6 },
        },
    }));
    let response = server.recv();
    assert_eq!(
        response["result"]["uri"], utils_uri,
        "cross-file goto: {response}"
    );
    assert_eq!(
        response["result"]["range"]["start"]["line"], 0,
        "double is on line 0 of utils.fun: {response}"
    );

    // Row 23: cross-file completion. Edit game.fun to an (unparseable) buffer
    // ending in `Utils.` — completion still answers from the last-good project,
    // offering the sibling's `double` with its inferred signature.
    let broken = "let apply = (n) => Utils.";
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": game_uri, "version": 2 },
            "contentChanges": [ { "text": broken } ],
        },
    }));
    server.recv(); // publishDiagnostics (a parse error on the broken buffer)
    server.send(json!({
        "jsonrpc": "2.0", "id": 10, "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": game_uri },
            "position": { "line": 0, "character": broken.encode_utf16().count() as i64 },
        },
    }));
    let response = server.recv();
    let items = response["result"].as_array().expect("completion array");
    let double = items
        .iter()
        .find(|item| item["label"] == "double")
        .unwrap_or_else(|| panic!("no `double` in completion: {response}"));
    assert_eq!(double["detail"], "Utils.double : (float) => float");

    server.send(json!({ "jsonrpc": "2.0", "id": 4, "method": "shutdown" }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "exit" }));
    server.child.wait().expect("wait for exit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Completion over the real binary (matrix rows 7, 8, 9, 13, 22): a broken
/// live buffer still completes prelude members from the last-good project, with
/// LSP kind codes over the wire and UTF-16 positions. Single-file (no
/// functor.json) still gets the injected prelude.
#[test]
fn completion_over_real_stdio() {
    let mut server = Server::spawn();
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} },
    }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));

    // A valid buffer opens first, seeding the last-good project cache.
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": URI, "languageId": "functor-lang", "version": 1, "text": "let s = () => 1.0\n",
        } },
    }));
    server.recv(); // publishDiagnostics (clean)

    // A helper: replace the whole buffer, drain its diagnostics, then ask for
    // completion at (line, character) — character in UTF-16 code units.
    let complete_at = |server: &mut Server, version: i64, text: &str, line: i64, character: i64| {
        server.send(json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": URI, "version": version },
                "contentChanges": [ { "text": text } ],
            },
        }));
        server.recv(); // publishDiagnostics
        server.send(json!({
            "jsonrpc": "2.0", "id": 100 + version, "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": URI },
                "position": { "line": line, "character": character },
            },
        }));
        server.recv()["result"].clone()
    };

    // Row 7: didChange to a broken buffer ending `Scene.` → completion still
    // answers `cube` with the real detail and kind 3 (Function), from cache.
    let broken = "let s = Scene.";
    let result = complete_at(
        &mut server,
        2,
        broken,
        0,
        broken.encode_utf16().count() as i64,
    );
    let items = result.as_array().expect("completion array");
    let cube = items
        .iter()
        .find(|item| item["label"] == "cube")
        .unwrap_or_else(|| panic!("no `cube` in {result}"));
    assert_eq!(cube["detail"], "Scene.cube : () => Scene.t");
    assert_eq!(cube["kind"], 3);

    // Row 8: broken ELSEWHERE than the cursor line (line 0 never parses) — the
    // members are still offered on the completion line.
    let elsewhere = "let broken =\nlet s = Scene.";
    let line1 = "let s = Scene.";
    let result = complete_at(
        &mut server,
        3,
        elsewhere,
        1,
        line1.encode_utf16().count() as i64,
    );
    let items = result.as_array().expect("completion array");
    assert!(
        items.iter().any(|item| item["label"] == "cube"),
        "row 8 expected cube: {result}"
    );

    // Row 9: an unknown module `Nope.` → an empty array (not null).
    let nope = "let s = Nope.";
    let result = complete_at(&mut server, 4, nope, 0, nope.encode_utf16().count() as i64);
    assert_eq!(result, json!([]), "row 9 expected []: {result}");

    // Row 13: an emoji earlier on the completion line — the character must be
    // counted in UTF-16 code units (the emoji is 2), or the offset lands wrong.
    let utf16 = "let s = { a: \"\u{1F642}\", b: Scene. }";
    let cursor = utf16.find("Scene.").unwrap() + "Scene.".len();
    let character = utf16[..cursor].encode_utf16().count() as i64;
    let result = complete_at(&mut server, 5, utf16, 0, character);
    let items = result.as_array().expect("completion array");
    assert!(
        items.iter().any(|item| item["label"] == "cube"),
        "row 13 expected cube with UTF-16 offset: {result}"
    );

    // Row 22: a top-level request shows the JSON kind codes over the wire —
    // `let` is a Keyword (14) and `Scene` is a Module (9).
    let top = "let x = 1.0\nlet y = ";
    let result = complete_at(
        &mut server,
        6,
        top,
        1,
        "let y = ".encode_utf16().count() as i64,
    );
    let items = result.as_array().expect("completion array");
    let kw = items
        .iter()
        .find(|item| item["label"] == "let")
        .unwrap_or_else(|| panic!("no `let` at top level: {result}"));
    assert_eq!(kw["kind"], 14);
    let module = items
        .iter()
        .find(|item| item["label"] == "Scene")
        .unwrap_or_else(|| panic!("no `Scene` module at top level: {result}"));
    assert_eq!(module["kind"], 9);

    server.send(json!({ "jsonrpc": "2.0", "id": 9, "method": "shutdown" }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "exit" }));
    server.child.wait().expect("wait for exit");
}

/// Completion PR 2 (scope-aware): over the real binary, a FRESH valid buffer
/// offers an in-scope local (a lambda param, kind Value = 12) and a typed
/// record field (kind Field = 5), with the cursor mid-buffer. Single-file
/// (no functor.json) still gets the injected prelude.
#[test]
fn scope_aware_completion_over_real_stdio() {
    let mut server = Server::spawn();
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} },
    }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));

    // A helper: replace the whole buffer, drain its diagnostics, then ask for
    // completion at (line, character) — character in UTF-16 code units.
    let complete_at = |server: &mut Server, version: i64, text: &str, line: i64, character: i64| {
        server.send(json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": URI, "version": version },
                "contentChanges": [ { "text": text } ],
            },
        }));
        server.recv(); // publishDiagnostics
        server.send(json!({
            "jsonrpc": "2.0", "id": 200 + version, "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": URI },
                "position": { "line": line, "character": character },
            },
        }));
        server.recv()["result"].clone()
    };

    // Seed the last-good cache with a valid buffer.
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": URI, "languageId": "functor-lang", "version": 1, "text": "let s = () => 1.0\n",
        } },
    }));
    server.recv(); // publishDiagnostics (clean)

    // A FRESH, valid buffer: complete at the end of the body reference `v`
    // (line 1) → the lambda param `v` is an in-scope local (kind Value = 12).
    let local_line = "let mk = (v: Vec2) => v";
    let local_text = format!("type Vec2 = {{ x: float, y: float }}\n{local_line}");
    let result = complete_at(
        &mut server,
        2,
        &local_text,
        1,
        local_line.encode_utf16().count() as i64,
    );
    let items = result.as_array().expect("completion array");
    let local = items
        .iter()
        .find(|item| item["label"] == "v")
        .unwrap_or_else(|| panic!("no local `v` in {result}"));
    assert_eq!(local["kind"], 12, "local `v` should be a Value: {result}");
    assert_eq!(local["detail"], "v : Vec2");

    // Same fresh buffer with a completed `v.x`: complete just after the dot →
    // the record's fields, `x` carrying kind Field = 5.
    let field_line = "let mk = (v: Vec2) => v.x";
    let field_text = format!("type Vec2 = {{ x: float, y: float }}\n{field_line}");
    let after_dot = (field_line.find("v.").unwrap() + 2) as i64;
    let result = complete_at(&mut server, 3, &field_text, 1, after_dot);
    let items = result.as_array().expect("completion array");
    let field = items
        .iter()
        .find(|item| item["label"] == "x")
        .unwrap_or_else(|| panic!("no field `x` in {result}"));
    assert_eq!(
        field["kind"], 5,
        "record field `x` should be a Field: {result}"
    );
    assert_eq!(field["detail"], "x : float");

    // The trigger keystroke itself: retype the line WITHOUT `.x` (valid —
    // refreshes the cache), then add just the `.` (breaks the parse — the
    // cache is one edit behind). Fields must still appear: the member gate
    // accepts exactly that typed-tail shape.
    let result = complete_at(&mut server, 4, &local_text, 0, 0);
    assert!(result.is_array(), "cache re-seeded: {result}");
    let dot_line = "let mk = (v: Vec2) => v.";
    let dot_text = format!("type Vec2 = {{ x: float, y: float }}\n{dot_line}");
    let result = complete_at(
        &mut server,
        5,
        &dot_text,
        1,
        dot_line.encode_utf16().count() as i64,
    );
    let items = result.as_array().expect("completion array");
    assert!(
        items.iter().any(|item| item["label"] == "x"),
        "no field `x` at the dot keystroke: {result}"
    );

    server.send(json!({ "jsonrpc": "2.0", "id": 9, "method": "shutdown" }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "exit" }));
    server.child.wait().expect("wait for exit");
}

/// The VSCode extension's grammar and manifests must at least be valid JSON
/// (visual verification happens in the editor — see tools/functor-lang-vscode/README).
/// The highlighting sample must also be a valid Functor Lang module, so it stays in
/// step with the language.
#[test]
fn vscode_extension_assets_are_well_formed() {
    let extension_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../functor-lang-vscode");
    for file in [
        "syntaxes/functor-lang.tmLanguage.json",
        "language-configuration.json",
        "package.json",
    ] {
        let path = format!("{extension_dir}/{file}");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        serde_json::from_str::<Value>(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    }

    let sample = std::fs::read_to_string(format!("{extension_dir}/test/sample.fun")).unwrap();
    let program = functor_lang::parse(&sample).expect("sample.fun parses");
    functor_lang::lower(program).expect("sample.fun lowers");
}

/// The engine prelude is injected as a check-time overlay (funi 2e-iii), so a
/// host call hovers with its real type — `Scene.cube : () => Scene.t` — not
/// `Unknown`. Single-file (no functor.json) still gets the prelude.
#[test]
fn prelude_gives_host_calls_real_types() {
    let mut server = Server::spawn();
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} },
    }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    let text = "let s = () => Scene.cube()\nlet r = Random.seed(1.0)\n";
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": URI, "languageId": "functor-lang", "version": 1, "text": text,
        } },
    }));
    server.recv(); // publishDiagnostics (clean — Scene.cube is known)

    // Hover on `Scene.cube` (char 13 = the `c` of cube).
    let col = text.find("Scene.cube").unwrap() as i64 + 6;
    server.send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": URI },
            "position": { "line": 0, "character": col },
        },
    }));
    let response = server.recv();
    assert_eq!(
        response["result"]["contents"]["value"],
        "```functor\nScene.cube : () => Scene.t\n```\n\nPrimitive geometry.",
        "prelude hover carries the .funi doc block: {response}"
    );

    // Go-to-definition on the same external jumps INTO the interface: a
    // real, readable scene.funi materialized from the embedded prelude.
    server.send(json!({
        "jsonrpc": "2.0", "id": 5, "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": URI },
            "position": { "line": 0, "character": col },
        },
    }));
    let definition = server.recv();
    let uri = definition["result"]["uri"].as_str().unwrap_or_default().to_string();
    assert!(
        uri.ends_with("Scene.funi"),
        "definition lands in the materialized interface: {definition}"
    );
    // Percent-decode the path portion (temp dirs may contain encoded bytes).
    let raw = uri.strip_prefix("file://").expect("file uri");
    let mut decoded = Vec::new();
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                decoded.push(b);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let target = std::path::PathBuf::from(String::from_utf8(decoded).expect("utf8 path"));
    let contents = std::fs::read_to_string(&target).expect("materialized file readable");
    let line = definition["result"]["range"]["start"]["line"].as_u64().unwrap() as usize;
    assert!(
        contents.lines().nth(line).unwrap_or_default().contains("let cube"),
        "the range points at the `let cube` signature: {definition}"
    );

    // Built-in interface modules (injected by the language, not the host
    // prelude) materialize too: Random.seed jumps into Random.funi.
    let col = "let r = Random.s".len() as i64;
    server.send(json!({
        "jsonrpc": "2.0", "id": 6, "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": URI },
            "position": { "line": 1, "character": col },
        },
    }));
    let builtin = server.recv();
    assert!(
        builtin["result"]["uri"].as_str().unwrap_or_default().ends_with("Random.funi"),
        "Random.seed jumps into the materialized builtin interface: {builtin}"
    );

    server.send(json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }));
    server.recv();
    server.send(json!({ "jsonrpc": "2.0", "method": "exit" }));
    server.child.wait().expect("wait for exit");
}
