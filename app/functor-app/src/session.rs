//! Runs the core's commands other than replies: one `functor run native`
//! child at a time, its output read for the debug server's address, `/state`
//! polled, input posted; agent runs (`agent::Runner`); saving tasks and
//! layouts; project file listings and reads; and tasks' changes
//! (`changes::Git`).

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use functor_app_core::projects::read_project_files;
use functor_app_core::protocol::{GameInput, ServerMsg};
use functor_app_core::{
    listening_url, Command, Display, Event, Harness, Layout, Polled, SessionEvent,
};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::sync::mpsc::{self, Sender, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::agent::{Agents, Runner};
use crate::changes::Git;

/// Each `/state` serializes the whole model on the game's loop, so a UI that
/// keeps polling should weigh this against the model's size.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// How long output written just before an exit is still collected.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
/// Output lines kept for a failure report.
const TAIL_LINES: usize = 20;
/// How long a layout change waits to be written, so a burst (a splitter
/// stepped by key) is one write.
const LAYOUT_DEBOUNCE: Duration = Duration::from_millis(300);

/// The `functor` to run: `--functor`, else `FUNCTOR_BIN`, else one next to
/// this executable (`exe`), else `functor` on `PATH`.
pub fn functor_bin(flag: Option<PathBuf>, env: Option<OsString>, exe: Option<PathBuf>) -> PathBuf {
    let name = format!("functor{}", std::env::consts::EXE_SUFFIX);
    flag.or_else(|| env.filter(|e| !e.is_empty()).map(PathBuf::from))
        .or_else(|| Some(exe?.parent()?.join(&name)).filter(|p| p.is_file()))
        .unwrap_or_else(|| name.into())
}

pub struct Driver {
    functor: PathBuf,
    events: UnboundedSender<Event>,
    live: Option<Live>,
    runs: Runner,
    /// Where tasks are saved (`store::home`).
    home: PathBuf,
    /// Where the next task id reservation starts: above every id loaded
    /// or reserved here.
    next_task: u64,
    /// The repo's git, when changes are tracked.
    git: Option<Arc<Git>>,
    /// Layouts not written yet, by project dir; flushed on drop too.
    layouts: Arc<Mutex<HashMap<PathBuf, Layout>>>,
}

/// Writes and forgets every pending layout.
fn write_layouts(home: &Path, pending: &Mutex<HashMap<PathBuf, Layout>>) {
    let layouts = std::mem::take(&mut *pending.lock().unwrap());
    for (dir, layout) in layouts {
        if let Err(e) = crate::store::save_layout(home, &dir, &layout) {
            eprintln!("functor-app: cannot save the layout: {e}");
        }
    }
}

impl Drop for Driver {
    /// A layout changed just before exit is still written.
    fn drop(&mut self) {
        write_layouts(&self.home, &self.layouts);
    }
}

/// The current child's supervisor. Dropping it kills the child.
struct Live {
    id: u64,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
    tail: Arc<Mutex<VecDeque<String>>>,
    /// The child's input poster, started by its first input.
    input: Option<UnboundedSender<GameInput>>,
}

impl Driver {
    /// `highest` is the highest task id in `home` (`store::load`).
    pub fn new(
        functor: PathBuf,
        agents: Agents,
        home: PathBuf,
        highest: u64,
        git: Option<Arc<Git>>,
        events: UnboundedSender<Event>,
    ) -> Self {
        Self {
            runs: Runner::new(functor.clone(), agents, git.clone(), events.clone()),
            functor,
            events,
            live: None,
            home,
            next_task: highest + 1,
            git,
            layouts: Arc::default(),
        }
    }

    /// A client's new task as the core takes it: with an id reserved in
    /// the store. A task the core then rejects leaves its id unused.
    pub fn new_task(&mut self, message: String, harness: Harness) -> Event {
        let id = crate::store::reserve_id(&self.home, self.next_task);
        if let Ok(id) = id {
            self.next_task = id + 1;
        }
        Event::NewTask {
            id: id.map_err(|e| e.to_string()),
            message,
            harness,
        }
    }

    pub async fn run(&mut self, command: Command) {
        match command {
            Command::StartSession { id, dir, display } => self.start(id, &dir, display),
            Command::StopSession { id } => {
                if self.live.as_ref().is_some_and(|l| l.id == id) {
                    self.stop().await;
                }
            }
            Command::PollState { id, debug_url } => {
                let events = self.events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(POLL_INTERVAL).await;
                    let event = SessionEvent::Polled(poll_state(&debug_url).await);
                    let _ = events.send(Event::Session { id, event });
                });
            }
            Command::Input {
                id,
                debug_url,
                input,
            } => {
                if let Some(live) = self.live.as_mut().filter(|l| l.id == id) {
                    let poster = live.input.get_or_insert_with(|| post_inputs(debug_url));
                    let _ = poster.send(input);
                }
            }
            Command::ListFiles { path, dir } => {
                let files = read_project_files(&dir);
                let _ = self.events.send(Event::Files { path, files });
            }
            Command::StartRun(spec) => self.runs.start(*spec),
            Command::StopRun { task, run } => self.runs.stop(task, run).await,
            Command::SaveTask { dir, task } => {
                if let Err(e) = crate::store::save(&self.home, &dir, &task) {
                    eprintln!("functor-app: cannot save task {}: {e}", task.id);
                }
            }
            Command::SaveHarness { dir, harness } => {
                if let Err(e) = crate::store::save_harness(&self.home, &dir, harness) {
                    eprintln!("functor-app: cannot save the agent choice: {e}");
                }
            }
            Command::SaveLayout { dir, layout } => {
                let first = {
                    let mut pending = self.layouts.lock().unwrap();
                    pending.insert(dir, *layout).is_none() && pending.len() == 1
                };
                if first {
                    let (home, pending) = (self.home.clone(), self.layouts.clone());
                    tokio::spawn(async move {
                        tokio::time::sleep(LAYOUT_DEBOUNCE).await;
                        write_layouts(&home, &pending);
                    });
                }
            }
            // Without a client to answer there is no one to read it for.
            Command::ReadFile { .. } => {}
            // Without a client to answer, only the summary is refreshed.
            Command::Diff { task, baseline, .. } => self.diff(task, baseline, None),
            Command::Revert {
                task,
                baseline,
                paths,
            } => self.revert(task, baseline, paths),
            // Clients are the server's (`server::run`); `--print` has none.
            Command::Reply(_) | Command::Broadcast(_) => {}
        }
    }

    /// Measures `task`'s change since `baseline` in the background; with
    /// `reply`, also sends its full diff there as `ServerMsg::Diff`.
    pub fn diff(&self, task: u64, baseline: String, reply: Option<Sender<String>>) {
        let Some(git) = self.git.clone() else {
            return;
        };
        let events = self.events.clone();
        tokio::spawn(async move {
            let Some(reply) = reply else {
                return crate::changes::measure(&git, task, baseline, &events).await;
            };
            let msg = match git.diff(&baseline).await {
                Ok((change, files, more)) => {
                    let tree = Some(change.tree.clone());
                    let _ = events.send(Event::Change {
                        task,
                        baseline,
                        change,
                    });
                    ServerMsg::Diff {
                        task,
                        tree,
                        files,
                        more,
                        error: None,
                    }
                }
                Err(error) => ServerMsg::Diff {
                    task,
                    tree: None,
                    files: vec![],
                    more: 0,
                    error: Some(error),
                },
            };
            let _ = reply.try_send(serde_json::to_string(&msg).expect("ServerMsg serializes"));
        });
    }

    /// Reads `path` of `project` (at `dir`) in the background and sends it
    /// to `reply` as `ServerMsg::File`.
    pub fn read_file(
        &self,
        project: String,
        path: String,
        dir: PathBuf,
        reply: Option<Sender<String>>,
    ) {
        let Some(reply) = reply else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let (text, error) = match crate::files::read_project_file(&dir, &path) {
                Ok(text) => (Some(text), None),
                Err(error) => (None, Some(error)),
            };
            let msg = ServerMsg::File {
                project,
                path,
                text,
                error,
            };
            let _ = reply.try_send(serde_json::to_string(&msg).expect("ServerMsg serializes"));
        });
    }

    fn revert(&self, task: u64, baseline: String, paths: Vec<String>) {
        let git = self.git.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let result = match git {
                Some(git) => git.revert(&baseline, &paths).await,
                None => Err("changes are tracked only in a git repository".to_string()),
            };
            let _ = events.send(Event::Reverted {
                task,
                baseline,
                result,
            });
        });
    }

    /// The current child's last output lines, oldest first.
    pub fn output_tail(&self) -> Vec<String> {
        self.live
            .as_ref()
            .map(|l| l.tail.lock().unwrap().iter().cloned().collect())
            .unwrap_or_default()
    }

    fn start(&mut self, id: u64, dir: &Path, display: Display) {
        let send = |event| {
            let _ = self.events.send(Event::Session { id, event });
        };
        let spawned = tokio::process::Command::new(&self.functor)
            .arg("-d")
            .arg(dir)
            .args(["run", "native"])
            .args(display.flag())
            .args(["--debug-port", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Backstop for every exit path that drops the child unstopped
            // (a panic, the runtime shutting down).
            .kill_on_drop(true)
            .spawn();
        let child = match spawned {
            Ok(child) => child,
            Err(e) => {
                let reason = format!("cannot run {}: {e}", self.functor.display());
                return send(SessionEvent::StartFailed { reason });
            }
        };
        if let Some(pid) = child.id() {
            send(SessionEvent::Spawned { pid });
        }
        let (stop, stopped) = oneshot::channel();
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let task = tokio::spawn(supervise(
            id,
            child,
            self.events.clone(),
            tail.clone(),
            stopped,
        ));
        // A previous child (none, if the core stopped it first) dies with its `Live`.
        self.live = Some(Live {
            id,
            stop,
            task,
            tail,
            input: None,
        });
    }

    /// Kills the current child and waits until it is gone.
    async fn stop(&mut self) {
        if let Some(live) = self.live.take() {
            let _ = live.stop.send(());
            let _ = live.task.await;
        }
    }
}

/// Reports the child's listening line and exit; kills it when `stop` fires
/// or its sender drops.
async fn supervise(
    id: u64,
    mut child: Child,
    events: UnboundedSender<Event>,
    tail: Arc<Mutex<VecDeque<String>>>,
    mut stop: oneshot::Receiver<()>,
) {
    let send = |event| {
        let _ = events.send(Event::Session { id, event });
    };
    // Both streams: the runtime logs to stderr, but the CLI's own messages
    // (load errors included) go to stdout when it is a pipe.
    let (line_tx, mut lines) = mpsc::unbounded_channel();
    forward(child.stdout.take().unwrap(), line_tx.clone());
    forward(child.stderr.take().unwrap(), line_tx);
    let record = |line: String| {
        if let Some(debug_url) = listening_url(&line) {
            send(SessionEvent::Listening { debug_url });
        }
        let mut tail = tail.lock().unwrap();
        if tail.len() == TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    };
    let mut open = true;
    let status = loop {
        tokio::select! {
            _ = &mut stop => return kill(child).await,
            status = child.wait() => break status,
            line = lines.recv(), if open => match line {
                Some(line) => record(line),
                None => open = false,
            },
        }
    };
    // Lines written just before the exit may still be in the pipes; bounded,
    // in case something else holds them open.
    let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
    while let Ok(Some(line)) = tokio::time::timeout_at(deadline, lines.recv()).await {
        record(line);
    }
    let reason = match status {
        Ok(status) if status.success() => status.to_string(),
        Ok(status) => match cause(&tail.lock().unwrap()) {
            Some(line) => format!("{status}: {line}"),
            None => status.to_string(),
        },
        Err(e) => e.to_string(),
    };
    send(SessionEvent::Exited { reason });
}

/// Sends `reader`'s lines until it closes. Reads to the end whatever the
/// bytes: a closed pipe would fail the child's next write and kill it.
pub(crate) fn forward(
    reader: impl AsyncRead + Unpin + Send + 'static,
    lines: UnboundedSender<String>,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader).split(b'\n');
        while let Ok(Some(line)) = reader.next_segment().await {
            let line = String::from_utf8_lossy(&line);
            let _ = lines.send(line.trim_end_matches('\r').to_string());
        }
    });
}

/// The first `error…` or panic line, else the last line: a failed load
/// prints its cause, then `error: cannot load …`, then `✗ failed in 1ms`.
pub(crate) fn cause(tail: &VecDeque<String>) -> Option<&str> {
    let error = tail
        .iter()
        .find(|l| l.starts_with("error") || l.contains("panicked at"));
    error.or(tail.back()).map(String::as_str)
}

async fn kill(mut child: Child) {
    // Kills and reaps.
    let _ = child.kill().await;
}

/// One HTTP/1.0 request to `url` (`http://host:port`; the server closes
/// after its response): the body of a 2xx response, or why it failed.
pub async fn request(
    url: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<String, String> {
    let what = format!("{method} {path}");
    let addr = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("not an http url: {url}"))?;
    let request = async {
        let mut stream = TcpStream::connect(addr).await?;
        let headers: String = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        let head = format!(
            "{method} {path} HTTP/1.0\r\nHost: {addr}\r\n{headers}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(head.as_bytes()).await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };
    let response = tokio::time::timeout(REQUEST_TIMEOUT, request)
        .await
        .map_err(|_| format!("{what} timed out"))?
        .map_err(|e| format!("{what}: {e}"))?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.lines().next().unwrap_or_default();
    if !status
        .split(' ')
        .nth(1)
        .is_some_and(|code| code.starts_with('2'))
    {
        return Err(format!("{what}: {status}: {}", body.trim()));
    }
    Ok(body.to_string())
}

/// `GET /state`'s `frame` and pointer surface.
async fn poll_state(debug_url: &str) -> Result<Polled, String> {
    let body = request(debug_url, "GET", "/state", &[], "").await?;
    let state: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("GET /state: {e}"))?;
    let frame = state["frame"]
        .as_u64()
        .ok_or_else(|| "GET /state: no `frame`".to_string())?;
    let mouse = &state["input"]["mouse"];
    let size = |key: &str| mouse[key].as_u64().and_then(|n| u32::try_from(n).ok());
    let surface = match (size("surface_width"), size("surface_height")) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some([w, h]),
        _ => None,
    };
    Ok(Polled { frame, surface })
}

/// Posts each input to `/input` in order, one request at a time: the debug
/// server answers each on its next frame, so moves that queue meanwhile are
/// collapsed to the latest. Ends when the sender drops (the session stops).
fn post_inputs(debug_url: String) -> UnboundedSender<GameInput> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut queue = VecDeque::new();
        loop {
            while let Ok(input) = rx.try_recv() {
                enqueue(&mut queue, input);
            }
            let input = match queue.pop_front() {
                Some(input) => input,
                None => match rx.recv().await {
                    Some(input) => input,
                    None => return,
                },
            };
            let body = serde_json::to_string(&input).expect("GameInput serializes");
            if let Err(e) = request(&debug_url, "POST", "/input", &[], &body).await {
                eprintln!("functor-app: {e}");
            }
        }
    });
    tx
}

/// Queues `input`, replacing a mouse move still waiting at the back.
fn enqueue(queue: &mut VecDeque<GameInput>, input: GameInput) {
    if let (Some(GameInput::MouseMove { .. }), GameInput::MouseMove { .. }) = (queue.back(), &input)
    {
        queue.pop_back();
    }
    queue.push_back(input);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `GameInput` the app posts reads back as the debug server's
    /// `InputCommand` with the same fields: the hand mirror cannot drift.
    #[test]
    fn game_inputs_are_debug_server_input_commands() {
        use functor_app_core::protocol::MouseButton;
        use functor_runtime_common::debug_protocol::InputCommand;
        let buttons = [MouseButton::Left, MouseButton::Right, MouseButton::Middle];
        let inputs = [
            GameInput::Key {
                key: "space".into(),
                down: true,
            },
            GameInput::MouseMove { x: -3, y: 7 },
            GameInput::MouseWheel { delta: -1 },
        ]
        .into_iter()
        .chain(buttons.map(|button| GameInput::MouseButton {
            button,
            down: false,
        }));
        for input in inputs {
            // Exhaustive: a new variant fails to compile until it is mapped.
            let expected = match input.clone() {
                GameInput::Key { key, down } => InputCommand::Key { key, down },
                GameInput::MouseMove { x, y } => InputCommand::MouseMove { x, y },
                GameInput::MouseWheel { delta } => InputCommand::MouseWheel { delta },
                GameInput::MouseButton { button, down } => {
                    let name = serde_json::to_value(button).unwrap();
                    let name = name.as_str().unwrap().to_string();
                    assert!(functor_runtime_common::MouseButton::from_name(&name).is_some());
                    InputCommand::MouseButton { button: name, down }
                }
            };
            let wire = serde_json::to_value(&input).unwrap();
            let read: InputCommand = serde_json::from_value(wire.clone())
                .unwrap_or_else(|e| panic!("{wire} is not an InputCommand: {e}"));
            assert_eq!(read, expected, "{wire}");
        }
    }

    #[test]
    fn cause_prefers_an_error_or_panic_line() {
        let tail = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<VecDeque<_>>();
        let failed = tail(&[
            "▸ run examples/bad (native)",
            "error: ./game.fun:2:1: expected an expression, found end of input",
            "error: cannot load the ./game.fun project",
            "✗ failed in 1ms",
        ]);
        assert_eq!(
            cause(&failed),
            Some("error: ./game.fun:2:1: expected an expression, found end of input")
        );
        let panicked = tail(&[
            "thread 'main' panicked at src/run.rs:10:5:",
            "note: run with `RUST_BACKTRACE=1`",
        ]);
        assert_eq!(
            cause(&panicked),
            Some("thread 'main' panicked at src/run.rs:10:5:")
        );
        assert_eq!(cause(&tail(&["a", "b"])), Some("b"));
        assert_eq!(cause(&tail(&[])), None);
    }

    #[test]
    fn queued_mouse_moves_collapse_to_the_latest() {
        let moved = |x| GameInput::MouseMove { x, y: 0 };
        let key = |down| GameInput::Key {
            key: "w".to_string(),
            down,
        };
        let mut queue = VecDeque::new();
        for input in [
            moved(1),
            moved(2),
            key(true),
            moved(3),
            moved(4),
            key(false),
        ] {
            enqueue(&mut queue, input);
        }
        // A move before a key stays, so the key lands where the pointer was.
        assert_eq!(
            Vec::from(queue),
            [moved(2), key(true), moved(4), key(false)]
        );
    }

    #[test]
    fn functor_bin_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("functor{}", std::env::consts::EXE_SUFFIX);
        let exe = dir.path().join("functor-app");
        let flag = || Some(PathBuf::from("/flag/functor"));
        let env = || Some(OsString::from("/env/functor"));
        // The flag, then the env var, win over everything after them.
        assert_eq!(
            functor_bin(flag(), env(), Some(exe.clone())),
            PathBuf::from("/flag/functor")
        );
        assert_eq!(
            functor_bin(None, env(), Some(exe.clone())),
            PathBuf::from("/env/functor")
        );
        // Without a sibling `functor`, an empty env var falls through to PATH.
        assert_eq!(
            functor_bin(None, Some(OsString::new()), Some(exe.clone())),
            PathBuf::from(&name)
        );
        std::fs::write(dir.path().join(&name), "").unwrap();
        assert_eq!(
            functor_bin(None, None, Some(exe.clone())),
            dir.path().join(&name)
        );
        assert_eq!(functor_bin(None, None, None), PathBuf::from(&name));
    }
}
