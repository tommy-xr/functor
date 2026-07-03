//! End-to-end test: spawn the real `mle-lsp` binary and speak framed LSP to
//! it over stdin/stdout — initialize, open a broken document, assert the
//! diagnostic, fix it, assert the clear, and check unknown requests get
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_mle-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn mle-lsp");
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

const URI: &str = "file:///tmp/e2e.mle";

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

    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));

    // didOpen with a broken document → one Error diagnostic at the `=`
    // (bytes 4..5, i.e. 0-based line 0 chars 4..5).
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": URI, "languageId": "mle", "version": 1, "text": "let = 3",
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
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
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

    // Clean shutdown.
    server.send(json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }));
    assert_eq!(server.recv()["id"], 3);
    server.send(json!({ "jsonrpc": "2.0", "method": "exit" }));
    let status = server.child.wait().expect("wait for exit");
    assert!(status.success(), "server exited with {status}");
}

/// The VSCode extension's grammar and manifests must at least be valid JSON
/// (visual verification happens in the editor — see tools/mle-vscode/README).
/// The highlighting sample must also be a valid MLE module, so it stays in
/// step with the language.
#[test]
fn vscode_extension_assets_are_well_formed() {
    let extension_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../mle-vscode");
    for file in [
        "syntaxes/mle.tmLanguage.json",
        "language-configuration.json",
        "package.json",
    ] {
        let path = format!("{extension_dir}/{file}");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        serde_json::from_str::<Value>(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"));
    }

    let sample = std::fs::read_to_string(format!("{extension_dir}/test/sample.mle")).unwrap();
    let program = mle::parse(&sample).expect("sample.mle parses");
    mle::lower(program).expect("sample.mle lowers");
}
