//! Agent tasks end to end, under the fake agent (`FUNCTOR_APP_FAKE_AGENT`):
//! over the WebSocket and as `functor-app task`, with every child process
//! checked gone afterwards. The tool server is a stub `functor` (a shell
//! script, so Unix only) or, when `FUNCTOR_BIN` is set, the real one. One
//! live test runs claude itself; it is skipped unless `FUNCTOR_APP_LIVE=1`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

#[cfg(unix)]
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../functor-app-core/fixtures/agent"
);

#[cfg(unix)]
fn fixture(name: &str) -> PathBuf {
    Path::new(FIXTURES).join(name)
}

/// A run whose tool call waits `gap_ms` for its result, so a test can look
/// at (or stop) it mid-run.
fn slow_fixture(dir: &Path, gap_ms: u64) -> PathBuf {
    let path = dir.join("slow.jsonl");
    let lines = [
        r#"{"ms":0,"event":{"type":"started","session":"fake-session-1","model":"claude-model"}}"#
            .to_string(),
        r#"{"ms":0,"event":{"type":"tool_use","name":"mcp__mis__get_state","input":{"session":"s1"}}}"#
            .to_string(),
        format!(r#"{{"ms":{gap_ms},"event":{{"type":"tool_result","content":"{{\"frame\":1}}"}}}}"#),
        format!(r#"{{"ms":{gap_ms},"event":{{"type":"text_delta","text":"Frame "}}}}"#),
        format!(r#"{{"ms":{gap_ms},"event":{{"type":"text_delta","text":"1."}}}}"#),
        format!(r#"{{"ms":{gap_ms},"event":{{"type":"text","text":"Frame 1."}}}}"#),
        format!(r#"{{"ms":{gap_ms},"event":{{"type":"usage","input":10,"output":2}}}}"#),
        format!(
            r#"{{"ms":{gap_ms},"event":{{"type":"finished","ok":true,"summary":"Frame 1."}}}}"#
        ),
    ];
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

/// A repo with one project, `a`.
fn tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("a")).unwrap();
    std::fs::write(
        tmp.path().join("a/functor.json"),
        r#"{"language": "functor-lang"}"#,
    )
    .unwrap();
    std::fs::write(tmp.path().join("a/game.fun"), "").unwrap();
    tmp
}

/// `tree()` as a git repo with one commit, its git reading no user config.
#[cfg(unix)]
fn git_tree() -> tempfile::TempDir {
    let tmp = tree();
    std::fs::write(tmp.path().join("a/game.fun"), "let x = 1\n").unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    git(tmp.path(), &["add", "-A"]);
    git(tmp.path(), &["commit", "-q", "-m", "first"]);
    tmp
}

#[cfg(unix)]
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // No `git/ignore` or `git/attributes` of the user's.
        .env("XDG_CONFIG_HOME", dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// A run that edits the project's `game.fun` (`x` → `xx`) as its agent would.
#[cfg(unix)]
fn edit_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("edit.jsonl");
    let lines = [
        r#"{"ms":0,"event":{"type":"started","session":"fake-session-1","model":"claude-model"}}"#,
        r#"{"ms":0,"event":{"type":"tool_use","name":"Edit","input":{"file_path":"game.fun"}}}"#,
        r#"{"ms":0,"edit":{"path":"game.fun","old":"x","new":"xx"}}"#,
        r#"{"ms":0,"event":{"type":"tool_result","content":"The file has been updated."}}"#,
        r#"{"ms":0,"event":{"type":"finished","ok":true,"summary":"Renamed x."}}"#,
    ];
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

/// Every file under `dir`, relative.
fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(dir).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

/// One process, as the tests look at it.
#[derive(Debug, Clone)]
struct Proc {
    pid: u32,
    argv: Vec<String>,
    #[cfg_attr(not(unix), allow(dead_code))]
    cwd: Option<PathBuf>,
}

/// `pid`'s live descendants (zombies waiting to be reaped do not count).
fn descendants(pid: u32) -> Vec<Proc> {
    let mut sys = System::new();
    let kind = ProcessRefreshKind::nothing()
        .with_cmd(UpdateKind::Always)
        .with_cwd(UpdateKind::Always);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let mut found = vec![Pid::from_u32(pid)];
    let mut i = 0;
    while i < found.len() {
        let parent = found[i];
        found.extend(
            sys.processes()
                .iter()
                .filter(|(_, p)| p.parent() == Some(parent) && p.status() != ProcessStatus::Zombie)
                .map(|(pid, _)| *pid),
        );
        i += 1;
    }
    found[1..]
        .iter()
        .filter_map(|pid| sys.process(*pid))
        .map(|p| Proc {
            pid: p.pid().as_u32(),
            argv: p
                .cmd()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
            cwd: p.cwd().map(Path::to_path_buf),
        })
        .collect()
}

fn alive(pid: u32) -> bool {
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(pid)
        .is_some_and(|p| p.status() != ProcessStatus::Zombie)
}

/// Polls `check` for up to 10 s.
fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `pid`'s two run children, once both are up: the fake agent
/// (`--replay-events`) and the tool server (the stub `exec`s `cat`).
fn run_children(pid: u32) -> (Proc, Proc) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let kids = descendants(pid);
        let is_agent = |p: &&Proc| p.argv.iter().any(|a| a == "--replay-events");
        let agent = kids.iter().find(is_agent);
        let server = kids.iter().find(|p| !is_agent(p));
        if let (Some(server), Some(agent)) = (server, agent) {
            return (server.clone(), agent.clone());
        }
        assert!(
            Instant::now() < deadline,
            "run children never appeared: {kids:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_gone(procs: &[&Proc]) {
    for p in procs {
        eventually(&format!("{p:?} to exit"), || !alive(p.pid));
    }
}

#[cfg(unix)]
mod stub {
    //! The stub `functor`: `mcp --http 0` prints the listening line for a
    //! fake HTTP server in the test, then holds until its stdin closes.
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{Receiver, Sender};

    /// One request the fake tool server received.
    pub struct Request {
        pub head: String,
        pub body: String,
    }

    impl Request {
        pub fn header(&self, name: &str) -> Option<String> {
            self.head.lines().find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
            })
        }
    }

    /// Answers every request 204, sending it on `requests`; its port.
    fn serve(requests: Sender<Request>) -> u16 {
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
                let body = String::from_utf8(body).unwrap();
                let _ = requests.send(Request { head, body });
                let _ = write!(reader.into_inner(), "HTTP/1.0 204 No Content\r\n\r\n");
            }
        });
        port
    }

    /// The stub in `dir` and the requests its server gets.
    pub fn functor(dir: &Path) -> (PathBuf, Receiver<Request>) {
        with_body(dir, "exec cat >/dev/null")
    }

    /// A stub running `body` after its listening line.
    pub fn with_body(dir: &Path, body: &str) -> (PathBuf, Receiver<Request>) {
        let (tx, requests) = std::sync::mpsc::channel();
        let port = serve(tx);
        let path = dir.join("functor");
        let script = format!(
            "#!/bin/sh\necho '[mcp] listening on http://127.0.0.1:{port}/mcp' >&2\n{body}\n"
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (path, requests)
    }
}

/// `functor-app --headless task …` under the fake agent replaying `fixture`.
fn task_command(repo: &Path, home: &Path, functor: &Path, fixture: &Path, prompt: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_functor-app"));
    command
        .args(["--headless", "task", prompt, "--follow", "-d"])
        .arg(repo)
        .arg("--functor")
        .arg(functor)
        .env("FUNCTOR_APP_HOME", home)
        .env("FUNCTOR_APP_FAKE_AGENT", fixture)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

#[cfg(unix)]
#[test]
fn cli_task_prints_the_transcript_and_exits_0() {
    let repo = tree();
    let home = tempfile::tempdir().unwrap();
    let (functor, requests) = stub::functor(home.path());
    let fixture = fixture("claude-game-tool.jsonl");
    let out = task_command(
        repo.path(),
        home.path(),
        &functor,
        &fixture,
        "What is the count?",
    )
    .output()
    .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(out.status.success(), "{stdout}{stderr}");
    assert_eq!(
        stdout,
        "\
you: What is the count?
tool: mcp__mis__get_state {\"session\":\"s1\"} -> {\"frame\":180,\"model\":{\"count\":3}}
agent: The game is on frame 180 and the count is 3.
usage: 2400 in, 75 out
finished: ok
"
    );

    // The run's context went to the tool server with its token, which
    // appears in no output.
    let put = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(put.head.starts_with("PUT /app-context "), "{}", put.head);
    let context: Value = serde_json::from_str(&put.body).unwrap();
    let game = serde_json::json!({"kind": "game"});
    assert_eq!(
        context,
        serde_json::json!({
            "project": "a", "active_project": "a", "session": null, "session_mode": "hidden",
            "layout": {
                "panes": [{"tabs": [game], "shown": game, "focused": true}],
                "game": "pane", "agent_tabs": [], "agent_arranges": true,
            },
        })
    );
    let auth = put.header("authorization").unwrap();
    let token = auth.strip_prefix("Bearer ").unwrap();
    assert_eq!(token.len(), 64, "{auth}");
    assert!(!stdout.contains(token) && !stderr.contains(token));

    // Saved in the app's home, nothing written in the repo.
    assert!(
        files(home.path())
            .iter()
            .any(|f| f.starts_with("projects/") && f.ends_with("/tasks/1.json")),
        "{:?}",
        files(home.path())
    );
    assert_eq!(files(repo.path()), ["a/functor.json", "a/game.fun"]);
}

#[cfg(unix)]
#[test]
fn cli_task_with_a_failed_run_exits_1() {
    let repo = tree();
    let home = tempfile::tempdir().unwrap();
    let (functor, _requests) = stub::functor(home.path());
    let fixture = fixture("claude-missing.jsonl");
    let out = task_command(repo.path(), home.path(), &functor, &fixture, "hi")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "you: hi\nfailed: spawning claude\n"
    );
}

/// `--max-input` stops a run past its budget, and `--metrics` says so.
#[cfg(unix)]
#[test]
fn cli_task_over_its_input_budget_stops_and_writes_metrics() {
    let repo = tree();
    let home = tempfile::tempdir().unwrap();
    let (functor, _requests) = stub::functor(home.path());
    // Usage 10 lands, then the run would wait 30 s.
    let fixture = home.path().join("budget.jsonl");
    let lines = [
        r#"{"ms":0,"event":{"type":"started","session":"s","model":"claude-model"}}"#,
        r#"{"ms":0,"event":{"type":"tool_use","name":"mcp__mis__get_state","input":{}}}"#,
        r#"{"ms":0,"event":{"type":"usage","input":10,"output":2}}"#,
        r#"{"ms":30000,"event":{"type":"finished","ok":true,"summary":null}}"#,
    ];
    std::fs::write(&fixture, lines.join("\n")).unwrap();
    let metrics = home.path().join("metrics.json");
    let started = Instant::now();
    let out = task_command(repo.path(), home.path(), &functor, &fixture, "hi")
        .arg("--max-input=5")
        .arg("--metrics")
        .arg(&metrics)
        .output()
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(20), "not stopped");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let m: Value = serde_json::from_str(&std::fs::read_to_string(&metrics).unwrap()).unwrap();
    assert_eq!(m["cut"], "over 5 input tokens");
    assert_eq!(m["ended"], "stopped");
    assert_eq!(m["input_tokens"], 10);
    assert_eq!(m["tools"], serde_json::json!({"mcp__mis__get_state": 1}));
    assert_eq!(m["calls"], serde_json::json!({"game": 1}));
    assert_eq!(m["model"], "claude-model");
    assert!(m["first_event_ms"].as_u64().unwrap() <= m["wall_ms"].as_u64().unwrap());

    // Over budget on its final usage, a run that finishes is not cut.
    let lines = [
        lines[0],
        lines[2],
        r#"{"ms":200,"event":{"type":"finished","ok":true,"summary":null}}"#,
    ];
    std::fs::write(&fixture, lines.join("\n")).unwrap();
    let out = task_command(repo.path(), home.path(), &functor, &fixture, "hi")
        .arg("--max-input=5")
        .arg("--metrics")
        .arg(&metrics)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let m: Value = serde_json::from_str(&std::fs::read_to_string(&metrics).unwrap()).unwrap();
    assert_eq!((&m["ended"], &m["cut"]), (&Value::from("ok"), &Value::Null));
}

/// Runs a task through the CLI with `functor` as the tool server; checks
/// its children exist mid-run and are gone after it exits 0.
fn cli_task_leaves_no_children(functor: &Path) {
    let repo = tree();
    let home = tempfile::tempdir().unwrap();
    let fixture = slow_fixture(home.path(), 2000);
    let mut app = task_command(
        repo.path(),
        home.path(),
        functor,
        &fixture,
        "What frame is it?",
    )
    .stdout(Stdio::piped())
    .spawn()
    .unwrap();
    let (server, agent) = run_children(app.id());
    // The agent runs in the project. (Windows does not show another
    // process's working directory.)
    #[cfg(unix)]
    assert_eq!(
        agent
            .cwd
            .as_ref()
            .map(|c| std::fs::canonicalize(c).unwrap()),
        Some(std::fs::canonicalize(repo.path().join("a")).unwrap())
    );
    let status = app.wait().unwrap();
    assert!(status.success());
    assert_gone(&[&server, &agent]);
}

#[cfg(unix)]
#[test]
fn cli_task_with_the_stub_leaves_no_children() {
    let home = tempfile::tempdir().unwrap();
    let (functor, _requests) = stub::functor(home.path());
    cli_task_leaves_no_children(&functor);
}

/// A tool server that ignores its stdin closing is killed after the grace
/// period, with what it started (a game, say).
#[cfg(unix)]
#[test]
fn a_hung_tool_server_is_killed_with_its_children() {
    let repo = tree();
    let home = tempfile::tempdir().unwrap();
    let (functor, _requests) = stub::with_body(home.path(), "sleep 60 &\nwait");
    let fixture = slow_fixture(home.path(), 500);
    let mut app = task_command(repo.path(), home.path(), &functor, &fixture, "hi")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    run_children(app.id());
    let all = descendants(app.id());
    assert!(
        all.iter()
            .any(|p| p.argv.first().is_some_and(|a| a == "sleep")),
        "{all:?}"
    );
    assert!(app.wait().unwrap().success());
    assert_gone(&all.iter().collect::<Vec<_>>());
}

/// The same against the real `functor mcp --http`, on every OS.
#[test]
fn cli_task_with_the_real_tool_server_leaves_no_children() {
    let Some(functor) = std::env::var_os("FUNCTOR_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    else {
        eprintln!("SKIPPED: set FUNCTOR_BIN to a built `functor` to use the real tool server");
        return;
    };
    cli_task_leaves_no_children(&functor);
}

#[cfg(unix)]
mod socket {
    //! The WebSocket flow, against a running `functor-app --headless`.
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::io::{BufRead, BufReader};
    use std::process::Child;
    use tokio_tungstenite::tungstenite::Message;

    pub struct App {
        pub child: Child,
        /// `/ws` with the launch's token.
        pub ws: String,
    }

    impl Drop for App {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub fn serve(repo: &Path, home: &Path, functor: &Path, fixture: &Path) -> App {
        let mut child = Command::new(env!("CARGO_BIN_EXE_functor-app"))
            .args(["--headless", "--no-device-link", "--port", "0", "-d"])
            .arg(repo)
            .arg("--functor")
            .arg(functor)
            .env("FUNCTOR_APP_HOME", home)
            .env("FUNCTOR_APP_FAKE_AGENT", fixture)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
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
        // Kept draining, so the app never blocks on its stderr.
        std::thread::spawn(move || for _ in stderr.lines() {});
        App { child, ws }
    }

    pub type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    pub async fn connect(ws: &str) -> Socket {
        tokio_tungstenite::connect_async(ws).await.unwrap().0
    }

    pub async fn send(socket: &mut Socket, msg: Value) {
        socket.send(Message::text(msg.to_string())).await.unwrap();
    }

    pub async fn next(socket: &mut Socket) -> Value {
        let msg = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no message within 10s")
            .unwrap()
            .unwrap();
        serde_json::from_str(msg.to_text().unwrap()).unwrap()
    }

    /// Reads until `done`; every `item` message on the way, as
    /// `(index, kind, partial or has-result)`.
    pub async fn until(
        socket: &mut Socket,
        done: impl Fn(&Value) -> bool,
    ) -> (Value, Vec<(u64, String, bool)>) {
        let mut items = Vec::new();
        loop {
            let msg = next(socket).await;
            if msg["type"] == "item" {
                let item = &msg["item"];
                let flag = item["partial"] == true || !item["result"].is_null();
                items.push((
                    msg["index"].as_u64().unwrap(),
                    item["kind"].as_str().unwrap().to_string(),
                    flag,
                ));
            }
            if done(&msg) {
                return (msg, items);
            }
        }
    }

    pub fn status_is(kind: &'static str) -> impl Fn(&Value) -> bool {
        move |msg| msg["type"] == "state" && msg["state"]["tasks"][0]["status"]["kind"] == kind
    }

    /// The `item` message adding the call at `index`, still unanswered.
    pub fn waiting_call(index: u64) -> impl Fn(&Value) -> bool {
        move |msg| msg["type"] == "item" && msg["index"] == index && msg["item"]["kind"] == "tool"
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn tasks_over_the_websocket() {
    use serde_json::json;
    use socket::*;

    let repo = tree();
    let home = tempfile::tempdir().unwrap();
    let (functor, requests) = stub::functor(home.path());
    let fixture = slow_fixture(home.path(), 2000);
    let app = serve(repo.path(), home.path(), &functor, &fixture);
    let pid = app.child.id();
    let mut ws = connect(&app.ws).await;
    assert_eq!(next(&mut ws).await["state"]["tasks"], json!([]));

    // A new task: its items stream in order and it ends done (no change).
    send(
        &mut ws,
        json!({"type": "new_task", "message": "What frame is it?", "harness": "claude"}),
    )
    .await;
    let (_, mut items) = until(&mut ws, waiting_call(1)).await;
    let (server, agent) = run_children(pid);
    let token = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    let token = token
        .header("authorization")
        .unwrap()
        .replace("Bearer ", "");
    for p in [&server, &agent] {
        assert!(
            !p.argv.iter().any(|a| a.contains(&token)),
            "token in argv: {p:?}"
        );
    }
    assert!(
        !agent.argv.iter().any(|a| a.starts_with("--replay-resume")),
        "{agent:?}"
    );
    let (state, rest) = until(&mut ws, status_is("done")).await;
    items.extend(rest);
    let k = |i: u64, kind: &str, flag: bool| (i, kind.to_string(), flag);
    // (index, kind, partial or answered). The streamed pieces of a text
    // may coalesce into its whole (the core's tests cover them), so they
    // are left out.
    items.retain(|(_, kind, partial)| !(kind == "agent" && *partial));
    assert_eq!(
        items,
        [
            k(0, "user", false),
            k(1, "tool", false),
            k(1, "tool", true),
            k(2, "agent", false),
            k(3, "usage", false),
            k(4, "finished", false),
        ]
    );
    let task = &state["state"]["tasks"][0];
    assert_eq!(task["title"], "What frame is it?");
    assert_eq!(task["session_id"], "fake-session-1");
    assert_gone(&[&server, &agent]);

    // A follow-up continues the stored session.
    send(
        &mut ws,
        json!({"type": "send_message", "task": 1, "message": "And now?"}),
    )
    .await;
    until(&mut ws, waiting_call(6)).await;
    let (server, agent) = run_children(pid);
    assert!(
        agent
            .argv
            .iter()
            .any(|a| a == "--replay-resume=fake-session-1"),
        "{agent:?}"
    );
    until(&mut ws, status_is("done")).await;
    assert_gone(&[&server, &agent]);

    // Stop mid-run ends it at once and leaves no children.
    send(
        &mut ws,
        json!({"type": "send_message", "task": 1, "message": "Once more"}),
    )
    .await;
    until(&mut ws, waiting_call(11)).await;
    let (server, agent) = run_children(pid);
    send(&mut ws, json!({"type": "stop_task", "task": 1})).await;
    let (_, items) = until(&mut ws, status_is("waiting_on_you")).await;
    assert_eq!(items, []);
    let (stopped, _) = until(&mut ws, |m| m["type"] == "item").await;
    assert_eq!(
        (stopped["index"].clone(), stopped["item"].clone()),
        (json!(12), json!({"kind": "stopped"}))
    );
    assert_gone(&[&server, &agent]);
    assert_eq!(descendants(pid).len(), 0);
    drop(ws);
    drop(app);

    // A restart with the same home has the task and its transcript back.
    let app = serve(repo.path(), home.path(), &functor, &fixture);
    let mut ws = connect(&app.ws).await;
    let state = next(&mut ws).await;
    let task = &state["state"]["tasks"][0];
    assert_eq!(
        (task["id"].clone(), task["status"]["kind"].clone()),
        (json!(1), json!("waiting_on_you"))
    );
    send(&mut ws, json!({"type": "get_transcript", "task": 1})).await;
    let (transcript, _) = until(&mut ws, |m| m["type"] == "transcript").await;
    let kinds: Vec<&str> = transcript["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "user", "tool", "agent", "usage", "finished", //
            "user", "tool", "agent", "usage", "finished", //
            "user", "tool", "stopped",
        ]
    );
}

/// In a git repo, a run that edits a file is ready for review; its diff
/// comes on request; Keep closes it; a follow-up measures from a fresh
/// baseline; Revert puts that one back. The user's git state is untouched.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn a_change_is_reviewed_kept_and_reverted() {
    use serde_json::json;
    use socket::*;

    let repo = git_tree();
    let home = tempfile::tempdir().unwrap();
    let (functor, _requests) = stub::functor(home.path());
    let fixture = edit_fixture(home.path());
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let app = serve(repo.path(), home.path(), &functor, &fixture);
    let mut ws = connect(&app.ws).await;
    assert_eq!(next(&mut ws).await["state"]["tracks_changes"], true);
    let game = repo.path().join("a/game.fun");
    let read = || std::fs::read_to_string(&game).unwrap();
    let diff = |msg: &Value| -> Vec<String> {
        let lines = msg["files"][0]["lines"].as_array().unwrap();
        lines
            .iter()
            .map(|l| {
                format!(
                    "{} {}",
                    l["kind"].as_str().unwrap(),
                    l["text"].as_str().unwrap()
                )
            })
            .collect()
    };

    send(
        &mut ws,
        json!({"type": "new_task", "message": "Rename x", "harness": "claude"}),
    )
    .await;
    let (state, _) = until(&mut ws, status_is("ready_for_review")).await;
    let task = &state["state"]["tasks"][0];
    let first = task["baseline"].as_str().unwrap().to_string();
    assert_eq!(
        (
            task["change"]["files"].clone(),
            task["change"]["more"].clone()
        ),
        (
            json!([{"path": "a/game.fun", "from": null, "added": 1, "removed": 1,
                "binary": false, "large": false}]),
            json!(0)
        )
    );
    assert_eq!(read(), "let xx = 1\n");
    send(&mut ws, json!({"type": "get_diff", "task": 1})).await;
    let (msg, _) = until(&mut ws, |m| m["type"] == "diff").await;
    assert_eq!(
        diff(&msg),
        ["hunk @@ -1 +1 @@", "removed let x = 1", "added let xx = 1"]
    );

    send(&mut ws, json!({"type": "keep_change", "task": 1})).await;
    let (state, _) = until(&mut ws, status_is("kept")).await;
    assert_eq!(state["state"]["tasks"][0]["baseline"], Value::Null);
    assert_eq!(read(), "let xx = 1\n");

    // The follow-up's change is its own: measured from the kept file.
    send(
        &mut ws,
        json!({"type": "send_message", "task": 1, "message": "Again"}),
    )
    .await;
    let (state, _) = until(&mut ws, status_is("ready_for_review")).await;
    let second = state["state"]["tasks"][0]["baseline"].as_str().unwrap();
    assert_ne!(second, first);
    send(&mut ws, json!({"type": "get_diff", "task": 1})).await;
    let (msg, _) = until(&mut ws, |m| m["type"] == "diff").await;
    assert_eq!(
        diff(&msg),
        [
            "hunk @@ -1 +1 @@",
            "removed let xx = 1",
            "added let xxx = 1"
        ]
    );

    let tree = msg["tree"].clone();
    send(
        &mut ws,
        json!({"type": "revert_change", "task": 1, "tree": tree}),
    )
    .await;
    until(&mut ws, status_is("reverted")).await;
    assert_eq!(read(), "let xx = 1\n");
    assert_eq!(git(repo.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(git(repo.path(), &["stash", "list"]), "");
    assert_eq!(
        git(repo.path(), &["status", "--porcelain"]),
        " M a/game.fun\n"
    );
    drop(ws);
    drop(app);

    // The CLI ends with the change it made.
    let out = task_command(repo.path(), home.path(), &functor, &fixture, "Once more")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.ends_with("finished: ok\nchange: 1 file, +1 -1\n  a/game.fun +1 -1\n"),
        "{stdout}"
    );
}

/// SIGTERM mid-run: the app stops the run, saves it as stopped, and its
/// children are gone.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn shutdown_stops_a_run() {
    use serde_json::json;
    use socket::*;

    let repo = tree();
    let home = tempfile::tempdir().unwrap();
    let (functor, _requests) = stub::functor(home.path());
    let fixture = slow_fixture(home.path(), 30_000);
    let mut app = serve(repo.path(), home.path(), &functor, &fixture);
    let mut ws = connect(&app.ws).await;
    next(&mut ws).await;
    send(
        &mut ws,
        json!({"type": "new_task", "message": "hi", "harness": "claude"}),
    )
    .await;
    until(&mut ws, waiting_call(1)).await;
    let (server, agent) = run_children(app.child.id());
    let term = Command::new("kill")
        .args(["-TERM", &app.child.id().to_string()])
        .status()
        .unwrap();
    assert!(term.success());
    assert_eq!(app.child.wait().unwrap().code(), Some(143));
    assert_gone(&[&server, &agent]);
    drop(app);

    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args(["--headless", "--print", "-d"])
        .arg(repo.path())
        .env("FUNCTOR_APP_HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains(
            "\ntasks: 1\n  #1 waiting_on_you a \"hi\" (claude claude-model): 3 items, 1 tool calls\n"
        ),
        "{text}"
    );
}

/// Live: claude answers from the game's own state. Spends real usage, so
/// it runs only with `FUNCTOR_APP_LIVE=1` and a `FUNCTOR_BIN` built from
/// this branch (`npm run build:cli:debug`).
#[test]
fn live_claude_reads_the_game_through_its_tools() {
    let functor = std::env::var_os("FUNCTOR_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file());
    let (Some(functor), Some("1")) = (functor, std::env::var("FUNCTOR_APP_LIVE").ok().as_deref())
    else {
        eprintln!("SKIPPED: set FUNCTOR_APP_LIVE=1 and FUNCTOR_BIN to run claude for real");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let counter = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter");
    let prompt = "Using your game tools, read the running game's current frame number and \
        the model's `count`. Answer in one short sentence giving both numbers.";
    let out = Command::new(env!("CARGO_BIN_EXE_functor-app"))
        .args([
            "--headless",
            "--no-gpu",
            "task",
            prompt,
            "--follow",
            "--start-session",
            "-d",
            counter,
        ])
        .arg("--functor")
        .arg(&functor)
        .env("FUNCTOR_APP_HOME", home.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    eprintln!("{stdout}");
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The saved transcript has the full tool results.
    let saved = files(home.path());
    let file = saved.iter().find(|f| f.ends_with("/tasks/1.json")).unwrap();
    let stored: Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join(file)).unwrap()).unwrap();
    let items = stored["transcript"].as_array().unwrap();
    let state = items
        .iter()
        .filter(|i| i["kind"] == "tool" && i["name"] == "mcp__mis__get_state")
        .filter_map(|i| serde_json::from_str::<Value>(i["result"].as_str()?).ok())
        .next_back()
        .expect("a get_state call with a JSON result");
    let frame = state["frame"].as_u64().expect("a frame in /state");
    let count = state["model"]["count"]
        .as_f64()
        .expect("a count in the model");
    let answer = items
        .iter()
        .filter(|i| i["kind"] == "agent")
        .filter_map(|i| i["text"].as_str())
        .next_back()
        .unwrap();
    assert!(
        answer.contains(&frame.to_string()),
        "{answer} (frame {frame})"
    );
    assert!(
        answer.contains(&format!("{count}")),
        "{answer} (count {count})"
    );
}

/// One raw request to the app's view route (`http://host:port/path`):
/// the status and body.
#[cfg(unix)]
fn post_view(url: &str, auth: Option<&str>, origin: bool, body: &str) -> (u16, String) {
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://").unwrap();
    let (addr, path) = rest.split_once('/').unwrap();
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    let auth = auth.map_or(String::new(), |a| format!("Authorization: Bearer {a}\r\n"));
    let origin = if origin {
        "Origin: http://evil.example\r\n"
    } else {
        ""
    };
    let request = format!(
        "POST /{path} HTTP/1.1\r\nHost: {addr}\r\n{auth}{origin}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // A refused oversize body may be cut off mid-write.
    let _ = stream.write_all(request.as_bytes());
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let status = response.split(' ').nth(1).unwrap_or("0").parse().unwrap();
    let body = response
        .split_once("\r\n\r\n")
        .map_or("", |(_, b)| b)
        .to_string();
    (status, body)
}

/// The view route takes the running task's own secret only, applies only
/// view changes to its project, and refuses the secret once the run ends.
/// The stub tool server records its argv and the secret it was given.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn the_view_route_takes_only_the_running_runs_secret() {
    use serde_json::json;
    use socket::*;

    let repo = tree();
    for (file, text) in [
        ("b/functor.json", r#"{"language": "functor-lang"}"#),
        ("b/b.fun", ""),
    ] {
        let path = repo.path().join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let home = tempfile::tempdir().unwrap();
    let (args, secret) = (home.path().join("args"), home.path().join("secret"));
    let body = format!(
        // Only as the tool server: the app also runs `functor init --list`.
        "[ \"$1\" = mcp ] && printf '%s\\n' \"$@\" > '{0}' && printf %s \"$FUNCTOR_MCP_APP_TOKEN\" > '{1}.tmp' && mv '{1}.tmp' '{1}'\nexec cat >/dev/null",
        args.display(),
        secret.display()
    );
    let (functor, requests) = stub::with_body(home.path(), &body);
    let fixture = slow_fixture(home.path(), 2500);
    let app = serve(repo.path(), home.path(), &functor, &fixture);
    let mut ws = connect(&app.ws).await;
    let first = next(&mut ws).await;
    assert_eq!(first["state"]["active"], "a");
    let b_layout = first["state"]["layouts"]["b"].clone();
    send(
        &mut ws,
        json!({"type": "new_task", "message": "Show me", "harness": "claude"}),
    )
    .await;
    until(&mut ws, waiting_call(1)).await;
    eventually("the stub to record its secret", || secret.is_file());
    let secret = std::fs::read_to_string(&secret).unwrap();
    let args = std::fs::read_to_string(&args).unwrap();
    let args: Vec<&str> = args.lines().collect();
    let at = args
        .iter()
        .position(|a| *a == "--app-url")
        .unwrap_or_else(|| panic!("no --app-url in {args:?}"));
    let url = args[at + 1].to_string();
    assert!(
        url.starts_with("http://127.0.0.1:") && url.ends_with("/agent/view"),
        "{url}"
    );
    assert_eq!(secret.len(), 64);
    // Never in an argv: the tool server's, or the agent's.
    let (server, agent) = run_children(app.child.id());
    for p in [&server, &agent] {
        assert!(
            !p.argv.iter().any(|a| a.contains(&secret)),
            "secret in argv: {p:?}"
        );
    }
    let _first_context = requests.recv_timeout(Duration::from_secs(5)).unwrap();

    let show = json!({"op": "show", "view": {"kind": "file", "path": "game.fun", "lines": [1, 1]}})
        .to_string();
    assert_eq!(post_view(&url, None, false, &show).0, 401);
    assert_eq!(post_view(&url, Some("wrong"), false, &show).0, 401);
    assert_eq!(post_view(&url, Some(&secret[..63]), false, &show).0, 401);
    assert_eq!(
        post_view(&url, Some(&secret), true, &show).0,
        403,
        "a page's request"
    );
    assert_eq!(post_view(&url, Some(&secret), false, "{not json").0, 400);
    let close = json!({"op": "close", "view": {"kind": "game"}}).to_string();
    assert_eq!(
        post_view(&url, Some(&secret), false, &close).0,
        400,
        "only the typed ops"
    );
    let huge =
        json!({"op": "show", "view": {"kind": "file", "path": "x".repeat(20 * 1024)}}).to_string();
    assert_eq!(post_view(&url, Some(&secret), false, &huge).0, 413);
    let (status, reply) = post_view(&url, Some(&secret), false, &show);
    assert_eq!(status, 200, "{reply}");
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["ok"], true, "{reply}");
    let traversal =
        json!({"op": "show", "view": {"kind": "file", "path": "../b/b.fun"}}).to_string();
    let (_, refused) = post_view(&url, Some(&secret), false, &traversal);
    assert!(
        refused.contains("\"ok\":false") && refused.contains("not a path inside"),
        "{refused}"
    );

    // Only project a's layout changed: game.fun opened beside, marked as the agent's.
    let (state, _) = until(&mut ws, |m| {
        m["type"] == "state"
            && m["state"]["layouts"]["a"]["agent"]["tabs"]
                .as_array()
                .is_some_and(|t| !t.is_empty())
    })
    .await;
    let layouts = &state["state"]["layouts"];
    assert_eq!(
        layouts["a"]["agent"]["tabs"],
        json!([{"kind": "file", "path": "game.fun"}])
    );
    assert_eq!(layouts["a"]["focus"], 1, "the user's pane keeps the focus");
    assert_eq!(layouts["b"], b_layout);
    // The tool server's app context followed the layout change.
    let put = requests.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(put.head.starts_with("PUT /app-context "), "{}", put.head);
    let context: Value = serde_json::from_str(&put.body).unwrap();
    assert_eq!(
        context["layout"]["panes"].as_array().unwrap().len(),
        2,
        "{context}"
    );
    assert_eq!(
        context["layout"]["panes"][1]["shown"],
        json!({"kind": "file", "path": "game.fun"})
    );

    // The secret dies with its run.
    until(&mut ws, status_is("done")).await;
    assert_eq!(post_view(&url, Some(&secret), false, &show).0, 401);
}

/// A run that shows a file mid-run, then presents a walkthrough, through
/// the run's real tool server (the fake agent's child calls it over MCP).
#[cfg(unix)]
fn view_fixture(dir: &Path) -> PathBuf {
    let path = dir.join("views.jsonl");
    let lines = [
        r#"{"ms":0,"event":{"type":"started","session":"fake-views-1","model":"claude-model"}}"#,
        r#"{"ms":0,"mcp":{"tool":"show_view","arguments":{"view":{"kind":"file","path":"game.fun","lines":[1,2]}}}}"#,
        r#"{"ms":1500,"mcp":{"tool":"present","arguments":{"steps":[{"title":"The count","note":"It starts at one.","view":{"kind":"file","path":"game.fun","lines":[2,2]}},{"title":"The game","view":{"kind":"game"}}]}}}"#,
        r#"{"ms":1500,"event":{"type":"text","text":"Shown."}}"#,
        r#"{"ms":3000,"event":{"type":"finished","ok":true,"summary":"Shown."}}"#,
    ];
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

/// The whole channel with the real `functor mcp` (`FUNCTOR_BIN`): a
/// mid-run `show_view` opens beside the user's pane, `present` adds the
/// card, the run's end applies its first step; with the setting off nothing
/// moves; "Restore my layout" restores.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn views_through_the_real_tool_server() {
    use serde_json::json;
    use socket::*;
    let Some(functor) = std::env::var_os("FUNCTOR_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    else {
        eprintln!("SKIPPED: set FUNCTOR_BIN to a built `functor` to use the real tool server");
        return;
    };
    let repo = tree();
    std::fs::write(
        repo.path().join("a/game.fun"),
        "let a = 1\nlet b = 2\nlet c = 3\n",
    )
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let fixture = view_fixture(home.path());
    let app = serve(repo.path(), home.path(), &functor, &fixture);
    let mut ws = connect(&app.ws).await;
    let user = next(&mut ws).await["state"]["layouts"]["a"].clone();
    assert_eq!(user["focus"], 1);
    let layout_is = |want: &'static str| {
        move |m: &Value| {
            m["type"] == "state"
                && functor_app_core::print_layout(
                    &serde_json::from_value(m["state"]["layouts"]["a"].clone()).unwrap(),
                ) == want
        }
    };
    send(
        &mut ws,
        json!({"type": "new_task", "message": "Show me", "harness": "claude"}),
    )
    .await;
    // Mid-run: beside the user's pane, the focus still theirs.
    let (state, _) = until(
        &mut ws,
        layout_is("right 50% (>#1[*game] | #3[*file:game.fun])"),
    )
    .await;
    assert_eq!(state["state"]["highlight"]["lines"], json!([1, 2]));
    assert!(
        state["state"]["tasks"][0]["run"].is_object(),
        "while the run goes on"
    );
    // The card arrives; the end of the run shows its first step.
    let (_, items) = until(&mut ws, |m| {
        m["type"] == "item" && m["item"]["kind"] == "walkthrough"
    })
    .await;
    assert!(
        items
            .iter()
            .any(|(_, kind, answered)| kind == "tool" && *answered),
        "{items:?}"
    );
    // The state goes before the items it changed.
    let (state, _) = until(&mut ws, |m| {
        m["type"] == "state" && m["state"]["highlight"]["lines"] == json!([2, 2])
    })
    .await;
    assert!(state["state"]["tasks"][0]["run"].is_null());
    let (msg, _) = until(&mut ws, |m| {
        m["type"] == "item" && m["item"]["kind"] == "walkthrough" && m["item"]["current"] == 0
    })
    .await;
    assert_eq!(msg["item"]["steps"][1]["view"], json!({"kind": "game"}));
    send(&mut ws, json!({"type": "get_transcript", "task": 1})).await;
    let (transcript, _) = until(&mut ws, |m| m["type"] == "transcript").await;
    let results: Vec<&str> = transcript["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["result"].as_str())
        .collect();
    assert_eq!(
        results,
        [
            "Opened game.fun, lines 1–2 in the pane beside the user's.",
            "Added a walkthrough of 2 steps to the chat; its first step opens beside the user's work when this run ends."
        ]
    );

    // Restore puts the user's layout back.
    send(
        &mut ws,
        json!({"type": "layout", "op": {"type": "restore"}}),
    )
    .await;
    until(&mut ws, layout_is(">#1[*game]")).await;

    // With the setting off, a second run moves nothing.
    send(
        &mut ws,
        json!({"type": "layout", "op": {"type": "agent_arranges", "on": false}}),
    )
    .await;
    send(
        &mut ws,
        json!({"type": "send_message", "task": 1, "message": "Again"}),
    )
    .await;
    let (shown, _) = until(&mut ws, |m| {
        m["type"] == "item" && m["index"] == 7 && !m["item"]["result"].is_null()
    })
    .await;
    assert!(
        shown["item"]["result"]
            .as_str()
            .unwrap()
            .contains("turned off"),
        "{shown}"
    );
    until(&mut ws, |m| {
        m["type"] == "item" && m["index"] == 11 && m["item"]["kind"] == "finished"
    })
    .await;
    let (state, _) = until(&mut ws, |m| {
        m["type"] == "state" && m["state"]["tasks"][0]["run"].is_null()
    })
    .await;
    let layout: functor_app_core::Layout =
        serde_json::from_value(state["state"]["layouts"]["a"].clone()).unwrap();
    assert_eq!(functor_app_core::print_layout(&layout), ">#1[*game]");
    assert!(!layout.agent.arranges);
}

/// A review's fake run: it reads the change and reports through the real
/// tool server, after `extra` lines (an edit, say).
#[cfg(unix)]
fn review_fixture(dir: &Path, extra: &[&str]) -> PathBuf {
    let path = dir.join("review.jsonl");
    let mut lines = vec![
        r#"{"ms":0,"event":{"type":"started","session":"fake-review-1","model":"claude-review-model"}}"#,
        r#"{"ms":0,"mcp":{"tool":"task_change","arguments":{}}}"#,
    ];
    lines.extend(extra);
    lines.extend([
        r#"{"ms":0,"mcp":{"tool":"report","arguments":{"summary":"The agent renamed `x` to `xx` in game.fun. Check the places that use `x`.","notes":["Nobody ran the game after the change."],"evidence":[]}}}"#,
        r#"{"ms":0,"event":{"type":"usage","input":21000,"output":300}}"#,
        r#"{"ms":0,"event":{"type":"finished","ok":true,"summary":"Reported."}}"#,
    ]);
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

/// `task --review` under the fake agent with the real tool server: a run
/// that changes a file is reviewed after it, and the review's report and
/// numbers go in the metrics. A review that edits anyway (the fake agent
/// can; a real one is denied by its permissions) has its report voided.
#[cfg(unix)]
#[test]
fn a_change_is_reviewed_through_the_real_tool_server() {
    let Some(functor) = std::env::var_os("FUNCTOR_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    else {
        eprintln!("SKIPPED: set FUNCTOR_BIN to a built `functor` to use the real tool server");
        return;
    };
    let edit = r#"{"ms":0,"edit":{"path":"game.fun","old":"xx","new":"xxx"}}"#;
    for (extra, ended) in [
        (&[][..], "prepared"),
        (&[edit][..], "failed: the files changed while the review ran"),
    ] {
        let repo = git_tree();
        let home = tempfile::tempdir().unwrap();
        let metrics = home.path().join("metrics.json");
        let run = edit_fixture(home.path());
        let out = task_command(repo.path(), home.path(), &functor, &run, "Rename x")
            .arg("--review")
            .arg("--metrics")
            .arg(&metrics)
            .env("FUNCTOR_APP_FAKE_REVIEW", review_fixture(home.path(), extra))
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let m: Value = serde_json::from_str(&std::fs::read_to_string(&metrics).unwrap()).unwrap();
        let review = &m["review"];
        assert_eq!(review["ended"], ended, "{m}");
        assert_eq!(review["model"], "claude-review-model");
        assert_eq!(review["input_tokens"], 21000);
        assert!(review["wall_ms"].as_u64().is_some(), "{m}");
        // The working run's numbers are its own.
        assert_eq!(m["ended"], "ok");
        assert_eq!(m["input_tokens"], Value::Null);
        match ended {
            "prepared" => {
                assert!(review["summary"].as_str().unwrap().starts_with("The agent renamed"));
                assert_eq!(review["notes"][0], "Nobody ran the game after the change.");
                assert_eq!(review["style"], serde_json::json!([]));
            }
            _ => assert_eq!(review["summary"], Value::Null, "voided: {m}"),
        }
        // No brief is left behind.
        assert!(
            !files(home.path()).iter().any(|f| f.starts_with("reviews/")),
            "{:?}",
            files(home.path())
        );
    }
}
