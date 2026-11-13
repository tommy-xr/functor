//! `--headless` serving: the WebSocket protocol, token and origin checks and
//! UI files, against a temp tree of two projects.

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
    /// `/ws` with this launch's token.
    ws: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(dir: &Path, extra: &[&str]) -> Server {
    serve_env(dir, extra, &[])
}

fn serve_env(dir: &Path, extra: &[&str], env: &[(&str, &str)]) -> Server {
    let mut child = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--no-device-link", "--port", "0", "-d"])
        .arg(dir)
        .args(extra)
        .envs(env.iter().copied())
        // Never the user's saved tasks; these tests start none.
        .env("FUNCTOR_APP_HOME", dir.join(".app-home"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    stderr.read_line(&mut line).unwrap();
    let (addr, token) = line
        .trim()
        .strip_prefix("[functor-app] listening on http://")
        .and_then(|url| url.split_once("/#token="))
        .unwrap_or_else(|| panic!("unexpected first line: {line:?}"));
    let ws = format!("ws://{addr}/ws?token={token}");
    let addr = addr.to_string();
    Server { child, addr, ws }
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

async fn connect(ws: &str) -> Socket {
    tokio_tungstenite::connect_async(ws).await.unwrap().0
}

/// The next message, within a timeout.
async fn next_msg(socket: &mut Socket) -> Value {
    let msg = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .expect("no message within 10s")
        .unwrap()
        .unwrap();
    serde_json::from_str(msg.to_text().unwrap()).unwrap()
}

/// The next message, which is the state.
async fn next_state(socket: &mut Socket) -> Value {
    let json = next_msg(socket).await;
    assert_eq!(json["type"], "state", "{json}");
    json["state"].clone()
}

/// Reads states until one matches `done`, skipping the session's clock
/// (`transport`, `marks`), which travels apart from them.
async fn state_where(socket: &mut Socket, done: impl Fn(&Value) -> bool) -> Value {
    loop {
        let json = next_msg(socket).await;
        if json["type"] == "state" && done(&json["state"]) {
            return json["state"].clone();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn clients_get_the_state_and_see_a_switch_broadcast() {
    let tmp = tree();
    // Opening a project starts its game: with no `functor`, it fails at once.
    let server = serve(tmp.path(), &["--functor", "/nonexistent/functor"]);
    let failed = |s: &Value, id: u64| {
        s["session"]["id"] == id && s["session"]["status"]["kind"] == "failed"
    };
    let mut one = connect(&server.ws).await;
    let mut two = connect(&server.ws).await;
    for socket in [&mut one, &mut two] {
        // The templates are listed as the app starts, then broadcast.
        let state = state_where(socket, |s| {
            (s["templates"].as_array().is_some_and(|t| !t.is_empty())
                || !s["templates_error"].is_null())
                && failed(s, 1)
        })
        .await;
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
        let state = state_where(socket, |s| {
            !s["files"].as_array().unwrap().is_empty() && failed(s, 2)
        })
        .await;
        assert_eq!(state["active"], "b", "{state}");
        assert_eq!(state["files"], serde_json::json!(["b.fun", "functor.json"]));
    }

    // A message that changes nothing answers only its sender.
    two.send(Message::text(switch)).await.unwrap();
    assert_eq!(next_state(&mut two).await["active"], "b");
    let quiet = tokio::time::timeout(Duration::from_millis(300), one.next()).await;
    assert!(quiet.is_err(), "the other client got {quiet:?}");
}

/// Whether `/ws` at `url` upgrades with `origin`; else the refusal.
async fn upgrade(url: &str, origin: Option<&str>) -> Result<Socket, String> {
    let mut request = url.into_client_request().unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(e) => Err(e.to_string()),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn the_socket_needs_the_token_and_our_own_origin() {
    let tmp = tree();
    let server = serve(tmp.path(), &[]);
    let own = format!("http://{}", server.addr);
    let no_token = format!("ws://{}/ws", server.addr);
    let wrong = format!("ws://{}/ws?token=0123", server.addr);
    for (url, origin) in [
        // No token or the wrong one: with our origin, or none (any process).
        (&no_token, Some(own.as_str())),
        (&no_token, None),
        (&wrong, Some(own.as_str())),
        (&wrong, None),
        // The token from another page.
        (&server.ws, Some("http://evil.example")),
        (&server.ws, Some("http://127.0.0.1:1")),
    ] {
        let refused = upgrade(url, origin).await.err();
        let refused = refused.unwrap_or_else(|| panic!("{url} from {origin:?} was accepted"));
        assert!(refused.contains("403"), "{refused}");
    }
    for origin in [Some(own.as_str()), None] {
        let mut socket = upgrade(&server.ws, origin).await.unwrap();
        assert_eq!(next_state(&mut socket).await["active"], "a");
    }
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

/// A fake debug server: answers `GET /state` and sends every `POST /input`
/// body on `inputs`.
fn fake_debug_server(inputs: std::sync::mpsc::Sender<String>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut reader = BufReader::new(stream.unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().unwrap();
                }
                head.push_str(&line);
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let reply = if head.starts_with("POST /input ") {
                let _ = inputs.send(String::from_utf8(body).unwrap());
                "ok".to_string()
            } else {
                r#"{"frame":1,"input":{"mouse":{"surface_width":800,"surface_height":600}}}"#
                    .to_string()
            };
            let mut stream = reader.into_inner();
            let _ = write!(stream, "HTTP/1.0 200 OK\r\n\r\n{reply}");
        }
    });
    port
}

/// Input from the page reaches the running session's `POST /input`, in order.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn input_is_posted_to_the_session() {
    use std::os::unix::fs::PermissionsExt;

    let (inputs_tx, inputs) = std::sync::mpsc::channel();
    let port = fake_debug_server(inputs_tx);
    let tmp = tree();
    // A `functor` that only announces the fake server, then waits to be killed.
    let fake = tmp.path().join("functor");
    let script = format!(
        "#!/bin/sh\necho '[debug-server] listening on http://127.0.0.1:{port}' >&2\nexec sleep 30\n"
    );
    std::fs::write(&fake, script).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let server = serve(tmp.path(), &["--functor", fake.to_str().unwrap()]);
    let mut socket = connect(&server.ws).await;
    next_state(&mut socket).await;

    socket
        .send(Message::text(r#"{"type":"start_session"}"#))
        .await
        .unwrap();
    let state = state_where(&mut socket, |s| {
        s["session"]["status"]["surface"] == serde_json::json!([800, 600])
    })
    .await;
    assert_eq!(state["session"]["status"]["kind"], "running", "{state}");
    // Its clock follows, apart from the state (this fake reports no `clock`).
    let json = next_msg(&mut socket).await;
    assert_eq!(json["type"], "transport", "{json}");
    assert_eq!(json["transport"]["frame"], 1, "{json}");
    assert_eq!(json["transport"]["clock"], false, "{json}");

    let sent = [
        r#"{"type":"key","key":"w","down":true}"#,
        r#"{"type":"mouse_move","x":10,"y":20}"#,
        r#"{"type":"mouse_button","button":"left","down":true}"#,
        r#"{"type":"key","key":"w","down":false}"#,
        r#"{"type":"mouse_button","button":"left","down":false}"#,
    ];
    for input in sent {
        let msg = format!(r#"{{"type":"input","input":{input}}}"#);
        socket.send(Message::text(msg)).await.unwrap();
    }
    for input in sent {
        let posted = inputs.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&posted).unwrap(),
            serde_json::from_str::<Value>(input).unwrap()
        );
    }
    // Input is not answered with the state. (The fake's polls never change,
    // so they broadcast nothing either.)
    let quiet = tokio::time::timeout(Duration::from_millis(300), socket.next()).await;
    assert!(quiet.is_err(), "input was answered: {quiet:?}");

    // A client that goes away holding a key or button has them released for it.
    socket
        .send(Message::text(
            r#"{"type":"input","input":{"type":"key","key":"a","down":true}}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        inputs.recv_timeout(Duration::from_secs(10)).unwrap(),
        r#"{"type":"key","key":"a","down":true}"#
    );
    // The left button was let go above; only `a` is still held.
    drop(socket);
    assert_eq!(
        inputs.recv_timeout(Duration::from_secs(10)).unwrap(),
        r#"{"type":"key","key":"a","down":false}"#
    );
    let quiet = inputs.recv_timeout(Duration::from_millis(300));
    assert!(quiet.is_err(), "more input posted: {quiet:?}");

    // A new page gets the state, then the clock.
    let mut socket = connect(&server.ws).await;
    next_state(&mut socket).await;
    assert_eq!(next_msg(&mut socket).await["type"], "transport");
    socket
        .send(Message::text(r#"{"type":"stop_session"}"#))
        .await
        .unwrap();
    state_where(&mut socket, |s| s["session"]["status"]["kind"] == "stopped").await;
}

/// The token reaches no child: the game session's environment and arguments
/// do not carry it, even when it came in through `FUNCTOR_APP_TOKEN`.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn the_game_child_never_sees_the_token() {
    use std::os::unix::fs::PermissionsExt;

    let (inputs_tx, _inputs) = std::sync::mpsc::channel();
    let port = fake_debug_server(inputs_tx);
    let tmp = tree();
    let seen = tmp.path().join("seen.txt");
    let fake = tmp.path().join("functor");
    let script = format!(
        "#!/bin/sh\n{{ env; echo \"$@\"; }} > '{}'\n\
         echo '[debug-server] listening on http://127.0.0.1:{port}' >&2\nexec sleep 30\n",
        seen.display()
    );
    std::fs::write(&fake, script).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let token = "e2e0token0that0must0not0leak";
    let server = serve_env(
        tmp.path(),
        &["--functor", fake.to_str().unwrap()],
        &[("FUNCTOR_APP_TOKEN", token)],
    );
    assert!(server.ws.ends_with(token), "{}", server.ws);
    let mut socket = connect(&server.ws).await;
    // The game starts by itself on open. A second start here would run the
    // script again, emptying the file while the first run still reads as running.
    state_where(&mut socket, |s| s["session"]["status"]["kind"] == "running").await;
    let seen = std::fs::read_to_string(&seen).unwrap();
    assert!(seen.contains("--debug-port"), "{seen}");
    assert!(!seen.contains(token), "{seen}");
    assert!(!seen.contains("FUNCTOR_APP_TOKEN"), "{seen}");
}
