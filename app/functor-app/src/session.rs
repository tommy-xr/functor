//! Runs the core's session commands: one `functor run native` child at a
//! time, its output read for the debug server's address, `/state` polled.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use functor_app_core::{listening_url, Command, Event, SessionEvent};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Child;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const POLL_TIMEOUT: Duration = Duration::from_secs(5);
/// Output lines kept for a failure report.
const TAIL_LINES: usize = 20;

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
    /// Run the child with `--headless` (no GL) instead of `--hidden`.
    no_gpu: bool,
    events: UnboundedSender<Event>,
    live: Option<Live>,
}

/// The current child's supervisor. Dropping it kills the child.
struct Live {
    id: u64,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
    tail: Arc<Mutex<VecDeque<String>>>,
}

impl Driver {
    pub fn new(functor: PathBuf, no_gpu: bool, events: UnboundedSender<Event>) -> Self {
        Self {
            functor,
            no_gpu,
            events,
            live: None,
        }
    }

    pub async fn run(&mut self, command: Command) {
        match command {
            Command::StartSession { id, dir } => self.start(id, &dir),
            Command::StopSession { id } => {
                if self.live.as_ref().is_some_and(|l| l.id == id) {
                    self.stop().await;
                }
            }
            Command::PollState { id, debug_url } => {
                let events = self.events.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(POLL_INTERVAL).await;
                    let event = SessionEvent::Polled(poll_frame(&debug_url).await);
                    let _ = events.send(Event::Session { id, event });
                });
            }
            // No clients to answer until the app serves a UI.
            Command::Reply(_) | Command::Broadcast(_) => {}
        }
    }

    /// The current child's last output lines, oldest first.
    pub fn output_tail(&self) -> Vec<String> {
        self.live
            .as_ref()
            .map(|l| l.tail.lock().unwrap().iter().cloned().collect())
            .unwrap_or_default()
    }

    fn start(&mut self, id: u64, dir: &Path) {
        let send = |event| {
            let _ = self.events.send(Event::Session { id, event });
        };
        let display = if self.no_gpu {
            "--headless"
        } else {
            "--hidden"
        };
        let spawned = tokio::process::Command::new(&self.functor)
            .arg("-d")
            .arg(dir)
            .args(["run", "native", display, "--debug-port", "0"])
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
    // The pipes close when the child exits, so its last lines are in the
    // tail before the exit is reported.
    loop {
        tokio::select! {
            _ = &mut stop => return kill(child).await,
            line = lines.recv() => {
                let Some(line) = line else { break };
                if let Some(debug_url) = listening_url(&line) {
                    send(SessionEvent::Listening { debug_url });
                }
                let mut tail = tail.lock().unwrap();
                if tail.len() == TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
    }
    let status = tokio::select! {
        _ = &mut stop => return kill(child).await,
        status = child.wait() => status,
    };
    let status = status.map_or_else(|e| e.to_string(), |s| s.to_string());
    let reason = match first_error(&tail.lock().unwrap()) {
        Some(line) => format!("{status}: {line}"),
        None => status,
    };
    send(SessionEvent::Exited { reason });
}

/// Sends `reader`'s lines until it closes.
fn forward(reader: impl AsyncRead + Unpin + Send + 'static, lines: UnboundedSender<String>) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = lines.send(line);
        }
    });
}

/// The first `error…` line, else the last line: a failed load prints its
/// cause, then `error: cannot load …`, then `✗ failed in 1ms`.
fn first_error(tail: &VecDeque<String>) -> Option<&str> {
    let error = tail.iter().find(|l| l.starts_with("error"));
    error.or(tail.back()).map(String::as_str)
}

async fn kill(mut child: Child) {
    // Kills and reaps.
    let _ = child.kill().await;
}

/// `GET /state`'s `frame`. HTTP/1.0, so the server closes after one response.
async fn poll_frame(debug_url: &str) -> Result<u64, String> {
    let addr = debug_url
        .strip_prefix("http://")
        .ok_or_else(|| format!("not an http url: {debug_url}"))?;
    let request = async {
        let mut stream = TcpStream::connect(addr).await?;
        let head = format!("GET /state HTTP/1.0\r\nHost: {addr}\r\n\r\n");
        stream.write_all(head.as_bytes()).await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };
    let response = tokio::time::timeout(POLL_TIMEOUT, request)
        .await
        .map_err(|_| "GET /state timed out".to_string())?
        .map_err(|e| format!("GET /state: {e}"))?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.lines().next().unwrap_or_default();
    if status.split(' ').nth(1) != Some("200") {
        return Err(format!("GET /state: {status}"));
    }
    let state: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("GET /state: {e}"))?;
    state["frame"]
        .as_u64()
        .ok_or_else(|| "GET /state: no `frame`".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_error_prefers_the_cause() {
        let tail = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect::<VecDeque<_>>();
        let failed = tail(&[
            "▸ run examples/bad (native)",
            "error: ./game.fun:2:1: expected an expression, found end of input",
            "error: cannot load the ./game.fun project",
            "✗ failed in 1ms",
        ]);
        assert_eq!(
            first_error(&failed),
            Some("error: ./game.fun:2:1: expected an expression, found end of input")
        );
        assert_eq!(first_error(&tail(&["a", "b"])), Some("b"));
        assert_eq!(first_error(&tail(&[])), None);
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
