//! New projects over the WebSocket with a real `functor` (`FUNCTOR_BIN`,
//! e.g. from `npm run build:cli:debug`; skipped without it): created from
//! two templates, opened, started, `git init` run or skipped, a described
//! game given its first task (the fake agent), and refusals. Temp dirs, a
//! temp home and git config isolated. Paths are spelled as on Unix.
#![cfg(unix)]

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

fn serve(dir: &Path, home: &Path, functor: &Path) -> Server {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/e2e/fixtures/chat.jsonl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--no-gpu", "--port", "0", "-d"])
        .arg(dir)
        .arg("--functor")
        .arg(functor)
        .env("FUNCTOR_APP_HOME", home)
        .env("FUNCTOR_APP_FAKE_AGENT", fixture)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
    let line = stderr
        .by_ref()
        .map(Result::unwrap)
        .find(|l| l.starts_with("[functor-app] listening on"))
        .expect("a listening line");
    // Keep reading, so the app never blocks on a full pipe.
    std::thread::spawn(move || stderr.for_each(drop));
    let (addr, token) = line
        .strip_prefix("[functor-app] listening on http://")
        .and_then(|url| url.split_once("/#token="))
        .unwrap();
    let ws = format!("ws://{addr}/ws?token={token}");
    Server { child, ws }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(socket: &mut Socket, msg: Value) {
    socket.send(Message::text(msg.to_string())).await.unwrap();
}

/// Messages until one matches `done`, within 60 s.
async fn until(socket: &mut Socket, done: impl Fn(&Value) -> bool) -> Value {
    let wait = async {
        loop {
            let msg = socket.next().await.unwrap().unwrap();
            let json: Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
            if done(&json) {
                return json;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), wait)
        .await
        .expect("no matching message within 60s")
}

async fn state_where(socket: &mut Socket, done: impl Fn(&Value) -> bool) -> Value {
    until(socket, |m| m["type"] == "state" && done(&m["state"])).await["state"].clone()
}

fn request(name: &str, parent: &Path, template: &str, git: bool, describe: &str) -> Value {
    json!({
        "name": name,
        "parent": parent.to_string_lossy(),
        "template": template,
        "git": git,
        "describe": describe,
    })
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

/// `functor -d dir build` succeeds.
fn builds(functor: &Path, dir: &Path) -> bool {
    let out = Command::new(functor)
        .arg("-d")
        .arg(dir)
        .arg("build")
        .output()
        .unwrap();
    out.status.success()
}

#[tokio::test(flavor = "current_thread")]
async fn projects_are_created_opened_started_and_given_their_first_task() {
    let Some(functor) = std::env::var_os("FUNCTOR_BIN").map(PathBuf::from) else {
        eprintln!("skipped: set FUNCTOR_BIN to a built functor");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let (home, start, repo) = (root.join("home"), root.join("start"), root.join("repo"));
    for dir in [&home, &start, &repo] {
        std::fs::create_dir(dir).unwrap();
    }
    assert!(git(&repo, &["init", "-q"]).status.success());
    let server = serve(&start, &home, &functor);
    let mut ws = tokio_tungstenite::connect_async(&server.ws)
        .await
        .unwrap()
        .0;

    // The modal lists the CLI's templates, starters first.
    send(&mut ws, json!({"type": "show_new_project", "here": false})).await;
    let state = state_where(&mut ws, |s| !s["templates"].as_array().unwrap().is_empty()).await;
    let names: Vec<&str> = state["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"3d") && names.contains(&"2d"), "{names:?}");
    assert_eq!(state["templates"][0]["kind"], "starter");

    // Outside git, into a parent that does not exist: created, git
    // initialised, opened, started.
    let games = root.join("games");
    let alpha = request("Alpha", &games, "3d", true, "");
    send(
        &mut ws,
        json!({"type": "check_new_project", "request": alpha}),
    )
    .await;
    let plan = until(&mut ws, |m| m["type"] == "new_project_plan").await;
    assert_eq!(plan["plan"]["git_init"], true, "{plan}");
    send(
        &mut ws,
        json!({"type": "create_project", "request": alpha, "harness": "claude"}),
    )
    .await;
    let dir = games.join("Alpha");
    let state = state_where(&mut ws, |s| s["folder"] == dir.to_string_lossy().as_ref()).await;
    assert_eq!(state["new_project"], Value::Null);
    assert_eq!(state["active"], ".");
    assert_eq!(state["tracks_changes"], true);
    let kind = state["session"]["status"]["kind"].as_str().unwrap();
    assert!(matches!(kind, "starting" | "running"), "{kind}");
    assert!(dir.join("functor.json").is_file() && dir.join("game.fun").is_file());
    assert!(dir.join(".git").is_dir());
    assert!(builds(&functor, &dir), "Alpha does not build");

    // Inside a work tree, described: no `git init`; its task starts.
    send(&mut ws, json!({"type": "show_new_project", "here": false})).await;
    let beta = request("Beta", &repo, "2d", true, "Make the player blue");
    send(
        &mut ws,
        json!({"type": "check_new_project", "request": beta}),
    )
    .await;
    let plan = until(&mut ws, |m| m["type"] == "new_project_plan").await;
    assert_eq!(plan["plan"]["in_git"], true, "{plan}");
    send(
        &mut ws,
        json!({"type": "create_project", "request": beta, "harness": "claude"}),
    )
    .await;
    let dir = repo.join("Beta");
    let state = state_where(&mut ws, |s| {
        s["folder"] == dir.to_string_lossy().as_ref() && !s["tasks"].as_array().unwrap().is_empty()
    })
    .await;
    assert_eq!(state["repo_root"], repo.to_string_lossy().as_ref());
    assert_eq!(state["active"], "Beta");
    assert_eq!(state["tasks"][0]["title"], "Make the player blue");
    assert_eq!(state["tasks"][0]["project"], "Beta");
    assert!(!dir.join(".git").exists());
    assert!(builds(&functor, &dir), "Beta does not build");
    // The fake agent's run ends.
    state_where(&mut ws, |s| s["tasks"][0]["run"].is_null()).await;

    // Not empty: the plan says what is in the way, and creating stops there.
    send(&mut ws, json!({"type": "show_new_project", "here": false})).await;
    let again = request("Alpha", &games, "3d", true, "");
    send(
        &mut ws,
        json!({"type": "check_new_project", "request": again}),
    )
    .await;
    let plan = until(&mut ws, |m| m["type"] == "new_project_plan").await;
    let alpha = games.join("Alpha");
    let error = plan["error"].as_str().unwrap();
    assert!(
        error.starts_with(&format!("{} is not empty: ", alpha.display())),
        "{error}"
    );
    send(
        &mut ws,
        json!({"type": "create_project", "request": again, "harness": "claude"}),
    )
    .await;
    let state = state_where(&mut ws, |s| s["new_project"]["step"]["kind"] == "failed").await;
    let step = &state["new_project"]["step"];
    assert!(
        step["reason"]
            .as_str()
            .unwrap()
            .contains("would be in the way"),
        "{step}"
    );
    assert_eq!(step["done"], json!([]));
    assert_eq!(state["folder"], dir.to_string_lossy().as_ref());
}
