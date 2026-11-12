//! `--headless` without `--print`: serves the UI and one WebSocket carrying
//! the core protocol on 127.0.0.1. One loop owns the state and runs `update`;
//! socket handlers only forward messages in and out.

use std::collections::{HashMap, HashSet};
use std::future::{Future, IntoFuture};
use std::net::Ipv4Addr;
use std::path::{Component, Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use functor_app_core::protocol::{ClientMsg, GameInput, ServerMsg};
use functor_app_core::{update, AppState, Command, Event, GitInfo};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Sender, UnboundedReceiver, UnboundedSender};

use crate::session::Driver;

mod assets {
    include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));
}

/// Messages queued for one client. A client this far behind is dropped
/// rather than buffered without bound; its page reconnects for a fresh state.
const CLIENT_QUEUE: usize = 32;

/// Most events the core loop takes in one batch.
const BATCH: usize = 256;

const NOT_BUILT: &str = "<!doctype html><title>Functor</title>\
    <p>UI not built — run <code>npm run build:app-ui</code>, then rebuild functor-app \
    or pass <code>--ui-dir app/ui/dist</code>.</p>";

/// Where the UI's files come from.
pub enum Ui {
    /// Read per request (`--ui-dir`), so a rebuilt UI shows on reload.
    Dir(PathBuf),
    /// Embedded at build time; empty when the UI was not built.
    Embedded,
}

/// From the socket handlers into the core loop.
enum Input {
    Connect { id: u64, out: Sender<String> },
    Client { id: u64, msg: ClientMsg },
    Disconnect { id: u64 },
}

struct Ctx {
    inputs: UnboundedSender<Input>,
    next_id: AtomicU64,
    /// The page's own origins; see `ws`.
    origins: [String; 2],
    ui: Ui,
}

/// Serves until `shutdown` resolves to a signal number, then stops the
/// session and every agent run and returns 128 + it.
pub async fn serve(
    state: AppState,
    mut driver: Driver,
    events: UnboundedReceiver<Event>,
    port: u16,
    ui: Ui,
    shutdown: impl Future<Output = u8>,
) -> std::io::Result<ExitCode> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    let (inputs, inputs_rx) = mpsc::unbounded_channel();
    let ctx = Arc::new(Ctx {
        inputs,
        next_id: AtomicU64::new(1),
        origins: [
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ],
        ui,
    });
    let app = Router::new()
        .route("/ws", get(ws))
        .fallback(get(asset))
        .with_state(ctx);
    eprintln!("[functor-app] listening on http://127.0.0.1:{port}");
    tokio::select! {
        result = axum::serve(listener, app).into_future() => {
            result?;
            Ok(ExitCode::FAILURE)
        }
        signal = run(state, &mut driver, events, inputs_rx, shutdown) => {
            Ok(ExitCode::from(128 + signal))
        }
    }
}

/// The core loop: every event goes through `update`, its replies and
/// broadcasts go to the clients, the rest to the session driver.
async fn run(
    mut state: AppState,
    driver: &mut Driver,
    mut events: UnboundedReceiver<Event>,
    mut inputs: UnboundedReceiver<Input>,
    shutdown: impl Future<Output = u8>,
) -> u8 {
    tokio::pin!(shutdown);
    let mut clients: HashMap<u64, Sender<String>> = HashMap::new();
    // Per client, the releases of the keys and buttons it holds down: sent
    // for it if it goes away mid-press, so nothing stays held in the game.
    let mut held: HashMap<u64, HashSet<GameInput>> = HashMap::new();
    loop {
        let (events, from) = tokio::select! {
            biased;
            signal = &mut shutdown => {
                let (_, commands) = update(state, Event::Shutdown);
                for command in commands {
                    driver.run(command).await;
                }
                return signal;
            }
            // Everything already queued goes in one batch, so a burst of
            // streamed pieces of one item coalesces (`queue`) instead of
            // overflowing a client's queue.
            Some(event) = events.recv() => {
                let mut batch = vec![event];
                while batch.len() < BATCH {
                    match events.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                (batch, None)
            }
            Some(input) = inputs.recv() => match input {
                // A new client is answered with the whole state.
                Input::Connect { id, out } => {
                    clients.insert(id, out);
                    (vec![Event::Client(ClientMsg::GetState)], Some(id))
                }
                Input::Client { id, msg } => {
                    if let ClientMsg::Input { input } = &msg {
                        track(held.entry(id).or_default(), input);
                    }
                    (vec![Event::Client(msg)], Some(id))
                }
                Input::Disconnect { id } => {
                    clients.remove(&id);
                    let releases = held.remove(&id).unwrap_or_default();
                    let releases = releases.into_iter().map(|input| Event::Client(ClientMsg::Input { input }));
                    (releases.collect(), None)
                }
            },
        };
        let mut outgoing = Vec::new();
        for event in events {
            let commands;
            (state, commands) = update(state, event);
            for command in commands {
                match command {
                    Command::Reply(msg) => {
                        if let Some(id) = from {
                            if clients
                                .get(&id)
                                .is_some_and(|out| out.try_send(json(&msg)).is_err())
                            {
                                clients.remove(&id);
                            }
                        }
                    }
                    Command::Broadcast(msg) => queue(&mut outgoing, msg),
                    // The diff answers the asking client, not everyone.
                    Command::Diff {
                        task,
                        baseline,
                        reply: true,
                    } => driver.diff(task, baseline, from.and_then(|id| clients.get(&id).cloned())),
                    command => driver.run(command).await,
                }
            }
        }
        for msg in outgoing {
            let text = json(&msg);
            clients.retain(|_, out| out.try_send(text.clone()).is_ok());
        }
    }
}

/// Queues `msg`, dropping the message just before it when `msg` replaces
/// it: the state again, or the same transcript item again.
fn queue(out: &mut Vec<ServerMsg>, msg: ServerMsg) {
    let replaces = match (out.last(), &msg) {
        (Some(ServerMsg::State { .. }), ServerMsg::State { .. }) => true,
        (
            Some(ServerMsg::Item { task, index, .. }),
            ServerMsg::Item {
                task: t, index: i, ..
            },
        ) => (task, index) == (t, i),
        _ => false,
    };
    if replaces {
        out.pop();
    }
    out.push(msg);
}

/// Records the release for a key or button `input` presses, or forgets it
/// when `input` lets go.
fn track(held: &mut HashSet<GameInput>, input: &GameInput) {
    let (release, down) = match input {
        GameInput::Key { key, down } => (
            GameInput::Key {
                key: key.clone(),
                down: false,
            },
            *down,
        ),
        GameInput::MouseButton { button, down } => (
            GameInput::MouseButton {
                button: *button,
                down: false,
            },
            *down,
        ),
        GameInput::MouseMove { .. } | GameInput::MouseWheel { .. } => return,
    };
    if down {
        held.insert(release);
    } else {
        held.remove(&release);
    }
}

fn json(msg: &ServerMsg) -> String {
    serde_json::to_string(msg).expect("ServerMsg serializes")
}

async fn ws(
    State(ctx): State<Arc<Ctx>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    // Any web page can open a WebSocket to loopback; only our own page (or a
    // non-browser client, which sends no Origin) may drive the app.
    if let Some(origin) = headers.get(header::ORIGIN) {
        if !ctx.origins.iter().any(|o| origin == o.as_str()) {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    upgrade.on_upgrade(move |socket| client(ctx, socket))
}

async fn client(ctx: Arc<Ctx>, socket: WebSocket) {
    let id = ctx.next_id.fetch_add(1, Ordering::Relaxed);
    let (out, mut outgoing) = mpsc::channel(CLIENT_QUEUE);
    if ctx.inputs.send(Input::Connect { id, out }).is_err() {
        return;
    }
    let (mut sink, mut stream) = socket.split();
    let send = async {
        while let Some(text) = outgoing.recv().await {
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    };
    let receive = async {
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Text(text) => match serde_json::from_str(&text) {
                    Ok(msg) => {
                        let _ = ctx.inputs.send(Input::Client { id, msg });
                    }
                    Err(e) => eprintln!("functor-app: ignoring a client message: {e}"),
                },
                Message::Close(_) => break,
                _ => {}
            }
        }
    };
    tokio::select! {
        _ = send => {}
        _ = receive => {}
    }
    let _ = ctx.inputs.send(Input::Disconnect { id });
}

async fn asset(State(ctx): State<Arc<Ctx>>, uri: Uri) -> Response {
    let path = match uri.path().trim_start_matches('/') {
        "" => "index.html",
        path => path,
    };
    let body = match &ctx.ui {
        Ui::Dir(dir) => {
            let safe = Path::new(path)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
            match safe {
                true => tokio::fs::read(dir.join(path)).await.ok(),
                false => None,
            }
        }
        Ui::Embedded => assets::UI
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| bytes.to_vec()),
    };
    match body {
        Some(body) => (
            [
                (header::CONTENT_TYPE, content_type(path)),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        )
            .into_response(),
        None if path == "index.html" => Html(NOT_BUILT).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// The checkout at `root`; `None` outside git or before the first commit.
pub fn git_info(root: &Path) -> Option<GitInfo> {
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (out.status.success() && !text.is_empty()).then_some(text)
    };
    Some(GitInfo {
        branch: git(&["rev-parse", "--abbrev-ref", "HEAD"])?,
        commit: git(&["rev-parse", "--short", "HEAD"])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_keys_and_buttons_are_tracked_as_their_releases() {
        use functor_app_core::protocol::MouseButton;
        let key = |key: &str, down| GameInput::Key {
            key: key.to_string(),
            down,
        };
        let left = |down| GameInput::MouseButton {
            button: MouseButton::Left,
            down,
        };
        let mut held = HashSet::new();
        for input in [
            key("w", true),
            key("a", true),
            left(true),
            GameInput::MouseMove { x: 1, y: 2 },
            key("a", false),
            key("w", true),
        ] {
            track(&mut held, &input);
        }
        assert_eq!(held, HashSet::from([key("w", false), left(false)]));
    }

    #[test]
    fn a_repeated_item_or_state_replaces_the_one_before_it() {
        use functor_app_core::Item;
        let item = |task, index, text: &str| ServerMsg::Item {
            task,
            index,
            item: Item::Agent {
                text: text.to_string(),
                partial: true,
            },
        };
        let mut out = Vec::new();
        for msg in [
            item(1, 2, "a"),
            item(1, 2, "ab"),
            item(1, 3, "c"),
            item(1, 2, "abc"),
            item(2, 2, "x"),
        ] {
            queue(&mut out, msg);
        }
        assert_eq!(
            out,
            [
                item(1, 2, "ab"),
                item(1, 3, "c"),
                item(1, 2, "abc"),
                item(2, 2, "x")
            ]
        );
    }

    #[test]
    fn content_types_by_extension() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("icon.svg"), "image/svg+xml");
        assert_eq!(content_type("LICENSE"), "application/octet-stream");
    }

    #[test]
    fn git_info_reads_a_checkout_and_is_none_outside_git() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(git_info(tmp.path()), None);
        let git = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(tmp.path())
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        git(&["init", "-q", "-b", "trunk"]);
        // No commit yet: no info.
        assert_eq!(git_info(tmp.path()), None);
        git(&["commit", "-q", "--allow-empty", "-m", "x"]);
        let info = git_info(tmp.path()).unwrap();
        assert_eq!(info.branch, "trunk");
        assert!(info.commit.len() >= 4, "{info:?}");
    }
}
