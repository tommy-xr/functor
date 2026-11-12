//! Opening folders over the WebSocket, directory suggestions, and the
//! recent projects across restarts, in temp trees with a temp home.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

struct Server {
    child: Child,
    ws: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `functor-app --headless`, with `-d dir` if given, started in `cwd`.
fn serve(dir: Option<&Path>, cwd: &Path, home: &Path) -> Server {
    let mut command = Command::new(env!("CARGO_BIN_EXE_functor-app"));
    command.args(["--headless", "--port", "0"]);
    if let Some(dir) = dir {
        command.arg("-d").arg(dir);
    }
    let mut child = command
        .current_dir(cwd)
        .env("FUNCTOR_APP_HOME", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = BufReader::new(child.stderr.take().unwrap());
    // Lines before it say why a startup folder could not be opened.
    let line = stderr
        .lines()
        .map(Result::unwrap)
        .find(|l| l.starts_with("[functor-app] listening on"))
        .expect("a listening line");
    let (addr, token) = line
        .strip_prefix("[functor-app] listening on http://")
        .and_then(|url| url.split_once("/#token="))
        .unwrap();
    let ws = format!("ws://{addr}/ws?token={token}");
    Server { child, ws }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(server: &Server) -> Socket {
    tokio_tungstenite::connect_async(&server.ws)
        .await
        .unwrap()
        .0
}

async fn send(socket: &mut Socket, msg: Value) {
    socket.send(Message::text(msg.to_string())).await.unwrap();
}

/// Messages until one matches `done`.
async fn until(socket: &mut Socket, done: impl Fn(&Value) -> bool) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no message within 10s")
            .unwrap()
            .unwrap();
        let json: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
        if done(&json) {
            return json;
        }
    }
}

async fn state_where(socket: &mut Socket, done: impl Fn(&Value) -> bool) -> Value {
    let msg = until(socket, |m| m["type"] == "state" && done(&m["state"])).await;
    msg["state"].clone()
}

async fn open(socket: &mut Socket, path: &Path) -> Value {
    let path = path.to_string_lossy().into_owned();
    send(socket, json!({"type": "open_folder", "path": path})).await;
    // The folder resolves to its canonical self.
    let canonical = std::fs::canonicalize(path.trim_end_matches('/')).unwrap();
    let canonical = canonical.to_string_lossy().into_owned();
    state_where(socket, |s| s["folder"] == canonical).await
}

/// Writes `files` (relative paths; a trailing `/` makes a directory) under `root`.
fn write(root: &Path, files: &[&str]) {
    let manifest = r#"{"language": "functor-lang"}"#;
    for file in files {
        let path = root.join(file);
        if file.ends_with('/') {
            std::fs::create_dir_all(&path).unwrap();
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = if file.ends_with("functor.json") {
            manifest
        } else {
            ""
        };
        std::fs::write(path, text).unwrap();
    }
}

/// A repo of two projects, a loose project folder, an empty folder, and the
/// app's home, under one canonical temp dir.
fn folders() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    write(
        &root,
        &[
            "repo/.git/",
            "repo/games/a/functor.json",
            "repo/games/a/game.fun",
            "repo/games/b/functor.json",
            "loose/functor.json",
            "loose/game.fun",
            "empty/",
            "home/",
            ".hidden/",
            "notes.txt",
        ],
    );
    (tmp, root)
}

/// The `path` of each of `state[key]`: projects or recents.
fn paths(state: &Value, key: &str) -> Vec<String> {
    state[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["path"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn folders_open_as_workspaces_and_become_recent_projects() {
    let (_tmp, root) = folders();
    let home = root.join("home");
    let s = |p: &str| root.join(p).to_string_lossy().into_owned();
    let server = serve(Some(&root.join("repo/games/b")), &root, &home);
    let mut ws = connect(&server).await;

    let state = state_where(&mut ws, |_| true).await;
    assert_eq!(state["repo_root"], s("repo"));
    assert_eq!(state["active"], "games/b");
    assert_eq!(paths(&state, "projects"), ["games/a", "games/b"]);
    assert_eq!(paths(&state, "recents"), [s("repo/games/b")]);

    // A loose project folder, outside git: usable, changes untracked.
    let state = open(&mut ws, &root.join("loose")).await;
    assert_eq!(state["repo_root"], s("loose"));
    assert_eq!(state["active"], ".");
    assert_eq!(state["files"], json!(["functor.json", "game.fun"]));
    assert_eq!(state["tracks_changes"], false);
    assert_eq!(paths(&state, "recents"), [s("loose"), s("repo/games/b")]);

    // An empty folder opens with no project, and is no recent project.
    let state = open(&mut ws, &root.join("empty")).await;
    assert_eq!(state["repo_root"], s("empty"));
    assert_eq!(state["projects"], json!([]));
    assert_eq!(state["active"], Value::Null);
    assert_eq!(paths(&state, "recents"), [s("loose"), s("repo/games/b")]);

    // The repo again, spelled with a trailing slash: the same entries.
    let state = open(&mut ws, Path::new(&format!("{}/", s("repo")))).await;
    assert_eq!(state["active"], "games/a");
    assert_eq!(
        paths(&state, "recents"),
        [s("repo/games/a"), s("loose"), s("repo/games/b")]
    );

    // What cannot be opened is refused, saying why.
    for (path, why) in [
        ("relative/dir", "give the folder's full path"),
        (&s("nope"), "it does not exist"),
        (&s("notes.txt"), "it is not a folder"),
    ] {
        send(&mut ws, json!({"type": "open_folder", "path": path})).await;
        let msg = until(&mut ws, |m| m["type"] == "rejected").await;
        assert_eq!(msg["reason"], format!("cannot open {path}: {why}"));
    }

    // Suggestions: directory names under the typed prefix, never files or
    // hidden ones, and whether the typed path opens.
    let prefix = format!("{}/", root.display());
    send(&mut ws, json!({"type": "list_dirs", "path": prefix})).await;
    let msg = until(&mut ws, |m| m["type"] == "dirs").await;
    assert_eq!(
        msg["dirs"],
        json!([s("empty"), s("home"), s("loose"), s("repo")])
    );
    assert_eq!(msg["error"], Value::Null);
    let partial = format!("{}/lo", root.display());
    send(&mut ws, json!({"type": "list_dirs", "path": partial})).await;
    let msg = until(&mut ws, |m| m["type"] == "dirs").await;
    assert_eq!(msg["dirs"], json!([s("loose")]));
    assert_eq!(msg["error"], "it does not exist");

    // A recent project can be removed.
    send(
        &mut ws,
        json!({"type": "remove_recent", "path": s("loose")}),
    )
    .await;
    let state = state_where(&mut ws, |s| s["recents"].as_array().unwrap().len() == 2).await;
    assert_eq!(
        paths(&state, "recents"),
        [s("repo/games/a"), s("repo/games/b")]
    );
    assert!(!root.join("repo/.functor").exists() && !root.join("loose/.functor").exists());
}

#[tokio::test(flavor = "current_thread")]
async fn without_a_dir_the_app_opens_the_most_recent_project_that_exists() {
    let (_tmp, root) = folders();
    let home = root.join("home");
    let s = |p: &str| root.join(p).to_string_lossy().into_owned();
    {
        let server = serve(Some(&root.join("repo/games/a")), &root.join("empty"), &home);
        let mut ws = connect(&server).await;
        state_where(&mut ws, |_| true).await;
        open(&mut ws, &root.join("loose")).await;
    }
    // Started where there is no project: the most recent one opens.
    let server = serve(None, &root.join("empty"), &home);
    let mut ws = connect(&server).await;
    let state = state_where(&mut ws, |_| true).await;
    assert_eq!(state["folder"], s("loose"));
    assert_eq!(paths(&state, "recents"), [s("loose"), s("repo/games/a")]);
    drop((ws, server));

    // Gone: shown missing, and the next one opens.
    std::fs::remove_dir_all(root.join("loose")).unwrap();
    let server = serve(None, &root.join("empty"), &home);
    let mut ws = connect(&server).await;
    let state = state_where(&mut ws, |_| true).await;
    assert_eq!(state["folder"], s("repo/games/a"));
    assert_eq!(state["recents"][1]["missing"], true);
    drop((ws, server));

    // Started in a project, it wins over the recents.
    let server = serve(None, &root.join("repo/games/b"), &home);
    let mut ws = connect(&server).await;
    let state = state_where(&mut ws, |_| true).await;
    assert_eq!(state["active"], "games/b");
    drop((ws, server));

    // Nothing to open: no folder.
    let other = tempfile::tempdir().unwrap();
    let server = serve(None, &root.join("empty"), other.path());
    let mut ws = connect(&server).await;
    let state = state_where(&mut ws, |_| true).await;
    assert_eq!(
        (&state["folder"], &state["repo_root"]),
        (&json!(""), &json!(""))
    );
    assert_eq!(state["projects"], json!([]));
}
