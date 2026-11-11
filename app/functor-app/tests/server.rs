//! `--headless` serving: the WebSocket protocol, origin check and UI files,
//! against a temp tree of two projects.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Kills the server when the test ends, however it ends.
struct Server {
    child: Child,
    addr: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(dir: &Path, extra: &[&str]) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--port", "0", "-d"])
        .arg(dir)
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let addr = line
        .trim()
        .strip_prefix("[functor-app] listening on http://")
        .unwrap_or_else(|| panic!("unexpected first line: {line:?}"))
        .to_string();
    Server { child, addr }
}

fn tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = r#"{"language": "functor-lang"}"#;
    for (file, text) in [
        ("a/functor.json", manifest),
        ("a/game.fun", ""),
        ("b/functor.json", manifest),
        ("b/b.fun", ""),
        ("b/notes.txt", ""),
    ] {
        let path = tmp.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    tmp
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: &str) -> Socket {
    tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap()
        .0
}

/// The next state message, within a timeout.
async fn next_state(socket: &mut Socket) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .expect("no message within 10s")
        .unwrap()
        .unwrap();
    let json: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
    assert_eq!(json["type"], "state", "{json}");
    json["state"].clone()
}

/// Reads states until one matches `done`.
async fn state_where(socket: &mut Socket, done: impl Fn(&Value) -> bool) -> Value {
    loop {
        let state = next_state(socket).await;
        if done(&state) {
            return state;
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn clients_get_the_state_and_see_a_switch_broadcast() {
    let tmp = tree();
    let server = serve(tmp.path(), &[]);
    let mut one = connect(&server.addr).await;
    let mut two = connect(&server.addr).await;
    for socket in [&mut one, &mut two] {
        let state = next_state(socket).await;
        // Outside any project, a child of the start directory opens first.
        assert_eq!(state["active"], "a", "{state}");
        assert_eq!(
            state["files"],
            serde_json::json!(["functor.json", "game.fun"])
        );
        assert_eq!(state["git"], Value::Null);
        assert_eq!(state["projects"].as_array().unwrap().len(), 2);
    }

    let switch = r#"{"type":"set_active_project","path":"b"}"#;
    one.send(Message::text(switch)).await.unwrap();
    for socket in [&mut one, &mut two] {
        let state = state_where(socket, |s| !s["files"].as_array().unwrap().is_empty()).await;
        assert_eq!(state["active"], "b", "{state}");
        assert_eq!(state["files"], serde_json::json!(["b.fun", "functor.json"]));
    }

    // A message that changes nothing answers only its sender.
    two.send(Message::text(switch)).await.unwrap();
    assert_eq!(next_state(&mut two).await["active"], "b");
    let quiet = tokio::time::timeout(Duration::from_millis(300), one.next()).await;
    assert!(quiet.is_err(), "the other client got {quiet:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn another_origin_is_refused() {
    let tmp = tree();
    let server = serve(tmp.path(), &[]);
    let mut request = format!("ws://{}/ws", server.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", "http://evil.example".parse().unwrap());
    let refused = tokio_tungstenite::connect_async(request).await.unwrap_err();
    assert!(refused.to_string().contains("403"), "{refused}");

    let mut request = format!("ws://{}/ws", server.addr)
        .into_client_request()
        .unwrap();
    let own = format!("http://{}", server.addr);
    request.headers_mut().insert("Origin", own.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(next_state(&mut socket).await["active"], "a");
}

/// `GET path` over HTTP/1.1: the status line, headers and body as text.
fn get(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn serves_the_ui_dir_and_nothing_outside_it() {
    let tmp = tree();
    let ui = tmp.path().join("ui");
    std::fs::create_dir(&ui).unwrap();
    std::fs::write(ui.join("index.html"), "<p>the ui</p>").unwrap();
    std::fs::write(ui.join("app.js"), "export {}").unwrap();
    std::fs::write(tmp.path().join("secret.txt"), "secret").unwrap();
    let server = serve(tmp.path(), &["--ui-dir", ui.to_str().unwrap()]);

    let index = get(&server.addr, "/");
    assert!(index.starts_with("HTTP/1.1 200"), "{index}");
    assert!(index.contains("text/html"), "{index}");
    assert!(index.ends_with("<p>the ui</p>"), "{index}");
    let js = get(&server.addr, "/app.js");
    assert!(js.contains("text/javascript"), "{js}");
    for path in ["/../secret.txt", "/missing.js"] {
        let response = get(&server.addr, path);
        assert!(response.starts_with("HTTP/1.1 404"), "{path}: {response}");
    }
}
