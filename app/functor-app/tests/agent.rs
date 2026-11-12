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
    assert_eq!(
        context,
        serde_json::json!({"project": "a", "session": null, "session_mode": "hidden"})
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
        pub addr: String,
    }

    impl Drop for App {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub fn serve(repo: &Path, home: &Path, functor: &Path, fixture: &Path) -> App {
        let mut child = Command::new(env!("CARGO_BIN_EXE_functor-app"))
            .args(["--headless", "--port", "0", "-d"])
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
        let addr = line
            .trim()
            .strip_prefix("[functor-app] listening on http://")
            .unwrap_or_else(|| panic!("unexpected first line: {line:?}"))
            .to_string();
        // Kept draining, so the app never blocks on its stderr.
        std::thread::spawn(move || for _ in stderr.lines() {});
        App { child, addr }
    }

    pub type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    pub async fn connect(addr: &str) -> Socket {
        tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .unwrap()
            .0
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
    let mut ws = connect(&app.addr).await;
    assert_eq!(next(&mut ws).await["state"]["tasks"], json!([]));

    // A new task: its items stream in order and it ends waiting on the user.
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
    let (state, rest) = until(&mut ws, status_is("waiting_on_you")).await;
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

    // A follow-up continues the stored session; meanwhile more messages are refused.
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
    for (msg, reason) in [
        (
            json!({"type": "send_message", "task": 1, "message": "more"}),
            "task 1 is still running: wait for it to finish or stop it",
        ),
        (
            json!({"type": "new_task", "message": "other", "harness": "codex"}),
            "task 1 is running in a: one run per project at a time",
        ),
    ] {
        send(&mut ws, msg).await;
        let (rejected, _) = until(&mut ws, |m| m["type"] == "rejected").await;
        assert_eq!(rejected["reason"], reason);
    }
    until(&mut ws, status_is("waiting_on_you")).await;
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
    let mut ws = connect(&app.addr).await;
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
    let mut ws = connect(&app.addr).await;
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
        task["change"],
        json!({"files": [{"path": "a/game.fun", "from": null, "added": 1, "removed": 1,
            "binary": false, "large": false}], "more": 0})
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

    send(&mut ws, json!({"type": "revert_change", "task": 1})).await;
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
    let mut ws = connect(&app.addr).await;
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
        text.ends_with(
            "tasks: 1\n  #1 waiting_on_you a \"hi\" (claude claude-model): 3 items, 1 tool calls\n"
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
