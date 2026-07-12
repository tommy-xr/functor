//! Optional HTTP control server for the desktop runtime.
//!
//! When `functor-runner` is started with `--debug-port <PORT>`, [`spawn`] starts
//! a blocking [`tiny_http`] server on a background thread bound to
//! `127.0.0.1:<PORT>` (localhost only). HTTP handlers do **not** touch GL — GL
//! must stay on the main render thread. Instead each handler builds a
//! [`DebugRequest`] carrying a per-request response channel, hands it to the GL
//! loop over an [`mpsc`] channel, and blocks on its own response receiver. The
//! GL loop drains pending requests once per frame (see [`DebugRequest`]'s use in
//! `main.rs`) and fulfills them with framebuffer data / runtime state it can
//! only read on the main thread.
//!
//! Endpoints: `GET /` (index), `POST /capture`, `GET /state`, `GET /scene`,
//! `GET /trace`, `POST /input`, `POST /time`. See `docs/debug-runtime.md` for
//! usage and the observe-vs-drive workflows.

use std::io::Read;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use functor_runtime_common::Key;
use serde::Deserialize;
use tiny_http::{Header, Method, Response, Server};

/// A snapshot of runtime state the GL loop can read on the main thread, returned
/// for `GET /state`. `model` is the live game model rendered with Rust's
/// pretty-`Debug` formatter (via the dylib's `emit_state_debug` export) — readable
/// for any game with zero game-author effort, since Fable derives `Debug` on every
/// generated type. (The model isn't `Serialize`, so this is Debug text, not JSON.)
pub struct RuntimeState {
    pub frame: u64,
    pub tts: f32,
    pub width: u32,
    pub height: u32,
    pub model: String,
    /// Keys currently held, maintained by the runtime (serialized by their
    /// canonical names, ordered by key discriminant, e.g. `["W", "Up"]`).
    pub held_keys: Vec<Key>,
    /// Last known cursor position (x, y) in window pixels.
    pub mouse: (i32, i32),
}

impl RuntimeState {
    /// Built with `serde_json` so the (multi-line, quote-bearing) `model` debug
    /// string is correctly escaped into the JSON.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "frame": self.frame,
            "tts": self.tts,
            "viewport": { "width": self.width, "height": self.height },
            "model": self.model,
            "input": {
                "held_keys": self.held_keys,
                "mouse": { "x": self.mouse.0, "y": self.mouse.1 },
            },
        })
        .to_string()
    }
}

/// An input event to inject via `POST /input`. JSON is tagged by `type`:
/// `{"type":"key","key":"w","down":true}`,
/// `{"type":"mouse_move","x":10,"y":20}`,
/// `{"type":"mouse_wheel","delta":1}`,
/// `{"type":"ui_event","slot":0,"kind":"Clicked"}` (kind also
/// `{"SliderChanged":0.5}` / `{"TextChanged":"hi"}` — the
/// [`functor_runtime_common::ui::UiEventKind`] wire shape), so an agent can
/// drive interactive UI widgets headlessly (docs/ui-interaction.md U2).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputCommand {
    Key { key: String, down: bool },
    MouseMove { x: i32, y: i32 },
    MouseWheel { delta: i32 },
    UiEvent {
        slot: u32,
        kind: functor_runtime_common::ui::UiEventKind,
    },
}

/// A clock command via `POST /time`: `{"type":"set","tts":2.0}` pins the frame
/// time, `{"type":"advance","dts":0.5}` steps it once (with that dt), and
/// `{"type":"resume"}` returns to wall-clock.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimeCommand {
    Set { tts: f32 },
    Advance { dts: f32 },
    Resume,
}

/// A coupled scene rewind via `POST /rewind`: `{"frame": 42}` restores the
/// whole scene — model AND physics world — to the end of that RENDERED frame
/// (docs/time-travel.md T1). Pin the clock first (`POST /time {"type":"set"}`)
/// so the scene stays at the rewound frame instead of simulating forward.
#[derive(Debug, serde::Deserialize)]
pub struct RewindCommand {
    pub frame: u64,
}

/// Why a `POST /capture` couldn't return pixels. Maps to an HTTP status:
/// `Unavailable` → 503 (no GL, e.g. `--headless`), `Failed` → 500 (a real
/// readback/encode error).
pub enum CaptureError {
    Unavailable(String),
    Failed(String),
}

/// A request from the HTTP thread to the GL loop. Each variant carries a
/// one-shot `Sender` the GL loop uses to reply; the HTTP handler blocks on the
/// matching `Receiver`.
pub enum DebugRequest {
    /// `POST /capture` — reply with PNG bytes of the next rendered frame, or a
    /// [`CaptureError`] (unavailable in `--headless`, or a readback failure).
    Capture(Sender<Result<Vec<u8>, CaptureError>>),
    /// `GET /state` — reply with the current runtime state.
    State(Sender<RuntimeState>),
    /// `GET /scene` — reply with the current frame (camera + scene) as JSON.
    Scene(Sender<String>),
    /// `GET /trace` — reply with the paused-inspector trace JSON (visual-
    /// debugger PR2): the last real frame's entry-point invocations, replayed
    /// while paused. The GL loop fills in the paused state from its clock.
    Trace(Sender<String>),
    /// `POST /input` — inject a key/mouse event; reply Ok or an error message.
    Input(InputCommand, Sender<Result<(), String>>),
    /// `POST /time` — set/advance/resume the clock; reply once applied.
    Time(TimeCommand, Sender<()>),
    /// `POST /reload-source` — swap the game's logic from the pushed source
    /// (body = the raw `.fun` text), preserving the model. Ok carries a status
    /// line; Err the load error (400) — a broken push keeps the old program.
    ReloadSource(String, Sender<Result<String, String>>),
    /// `POST /rewind` — coupled scene rewind to a rendered frame. Ok carries a
    /// status line; Err (400) if the producer can't rewind or the frame is
    /// unrecorded/pruned.
    Rewind(u64, Sender<Result<String, String>>),
}

/// Start the debug HTTP server on a background thread. Returns the receiving end
/// of the request channel; the GL loop should `try_recv()` it once per frame.
/// `bind` is the interface to listen on — 127.0.0.1 (the default) keeps it
/// local; 0.0.0.0 exposes it to the LAN for remote develop (`/reload-source`
/// from a dev machine to a runner on another device). There is no auth: only
/// bind wide on networks where arbitrary game-code pushes are acceptable.
pub fn spawn(bind: &str, port: u16) -> Receiver<DebugRequest> {
    let (tx, rx) = mpsc::channel::<DebugRequest>();

    let addr = format!("{}:{}", bind, port);
    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[debug-server] failed to bind {}: {}", addr, e);
            std::process::exit(1);
        }
    };
    // Stderr, not stdout: `--debug-port` is a common `--json` automation combo,
    // so this notice must not land in the CLI's ndjson stream.
    eprintln!("[debug-server] listening on http://{}", addr);

    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            let url = request.url().to_string();
            // Strip any query string for routing.
            let path = url.split('?').next().unwrap_or("").to_string();

            match (&method, path.as_str()) {
                (Method::Post, "/capture") => {
                    let (resp_tx, resp_rx) = mpsc::channel::<Result<Vec<u8>, CaptureError>>();
                    if tx.send(DebugRequest::Capture(resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(Ok(png)) => {
                            let header =
                                Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..])
                                    .unwrap();
                            let resp = Response::from_data(png).with_header(header);
                            let _ = request.respond(resp);
                        }
                        // No GL to read back (e.g. headless) — Service Unavailable.
                        Ok(Err(CaptureError::Unavailable(msg))) => {
                            let _ =
                                request.respond(Response::from_string(msg).with_status_code(503));
                        }
                        // A genuine readback/encode failure — Internal Error.
                        Ok(Err(CaptureError::Failed(msg))) => {
                            let _ =
                                request.respond(Response::from_string(msg).with_status_code(500));
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("capture failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Get, "/state") => {
                    let (resp_tx, resp_rx) = mpsc::channel::<RuntimeState>();
                    if tx.send(DebugRequest::State(resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(state) => {
                            let header =
                                Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                                    .unwrap();
                            let resp = Response::from_string(state.to_json()).with_header(header);
                            let _ = request.respond(resp);
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("state failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Get, "/scene") => {
                    let (resp_tx, resp_rx) = mpsc::channel::<String>();
                    if tx.send(DebugRequest::Scene(resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(json) => {
                            let header =
                                Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                                    .unwrap();
                            let resp = Response::from_string(json).with_header(header);
                            let _ = request.respond(resp);
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("scene failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Get, "/trace") => {
                    let (resp_tx, resp_rx) = mpsc::channel::<String>();
                    if tx.send(DebugRequest::Trace(resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(json) => {
                            let header =
                                Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                                    .unwrap();
                            let resp = Response::from_string(json).with_header(header);
                            let _ = request.respond(resp);
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("trace failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Post, "/input") => {
                    let mut body = String::new();
                    if request.as_reader().read_to_string(&mut body).is_err() {
                        let _ = request
                            .respond(Response::from_string("bad body").with_status_code(400));
                        continue;
                    }
                    let cmd: InputCommand = match serde_json::from_str(&body) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = request.respond(
                                Response::from_string(format!("bad input json: {}", e))
                                    .with_status_code(400),
                            );
                            continue;
                        }
                    };
                    let (resp_tx, resp_rx) = mpsc::channel();
                    if tx.send(DebugRequest::Input(cmd, resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(Ok(())) => {
                            let _ = request.respond(Response::from_string("ok"));
                        }
                        Ok(Err(msg)) => {
                            let _ =
                                request.respond(Response::from_string(msg).with_status_code(400));
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("input failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Post, "/time") => {
                    let mut body = String::new();
                    if request.as_reader().read_to_string(&mut body).is_err() {
                        let _ = request
                            .respond(Response::from_string("bad body").with_status_code(400));
                        continue;
                    }
                    let cmd: TimeCommand = match serde_json::from_str(&body) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = request.respond(
                                Response::from_string(format!("bad time json: {}", e))
                                    .with_status_code(400),
                            );
                            continue;
                        }
                    };
                    let (resp_tx, resp_rx) = mpsc::channel();
                    if tx.send(DebugRequest::Time(cmd, resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(()) => {
                            let _ = request.respond(Response::from_string("ok"));
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("time failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Post, "/rewind") => {
                    let mut body = String::new();
                    if request.as_reader().read_to_string(&mut body).is_err() {
                        let _ = request
                            .respond(Response::from_string("bad body").with_status_code(400));
                        continue;
                    }
                    let cmd: RewindCommand = match serde_json::from_str(&body) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = request.respond(
                                Response::from_string(format!("bad rewind json: {e}"))
                                    .with_status_code(400),
                            );
                            continue;
                        }
                    };
                    let (resp_tx, resp_rx) = mpsc::channel();
                    if tx.send(DebugRequest::Rewind(cmd.frame, resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(Ok(status)) => {
                            let _ = request.respond(Response::from_string(status));
                        }
                        Ok(Err(message)) => {
                            let _ = request.respond(
                                Response::from_string(message).with_status_code(400),
                            );
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("rewind failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Post, "/reload-source") => {
                    // This endpoint can be LAN-exposed (--debug-bind), so
                    // bound the body: game source is KBs, and an unbounded
                    // read_to_string is an OOM invitation. Chunked bodies
                    // (no declared length) are rejected the same way.
                    const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
                    match request.body_length() {
                        Some(len) if len <= MAX_SOURCE_BYTES => {}
                        _ => {
                            let _ = request.respond(
                                Response::from_string(
                                    "source too large (or missing Content-Length); limit is 4MB",
                                )
                                .with_status_code(413),
                            );
                            continue;
                        }
                    }
                    let mut body = String::new();
                    let mut reader = request.as_reader();
                    // (&mut reader): `take` needs Sized, and `as_reader`
                    // hands back `&mut dyn Read`.
                    if std::io::Read::take(&mut reader, MAX_SOURCE_BYTES as u64 + 1)
                        .read_to_string(&mut body)
                        .is_err()
                        || body.len() > MAX_SOURCE_BYTES
                    {
                        let _ = request
                            .respond(Response::from_string("bad body").with_status_code(400));
                        continue;
                    }
                    let (resp_tx, resp_rx) = mpsc::channel();
                    if tx.send(DebugRequest::ReloadSource(body, resp_tx)).is_err() {
                        let _ = request
                            .respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(Ok(status)) => {
                            let _ = request.respond(Response::from_string(status));
                        }
                        // A load error in the pushed source (or a producer
                        // that can't reload) — the pusher's mistake: 400.
                        Ok(Err(msg)) => {
                            let _ =
                                request.respond(Response::from_string(msg).with_status_code(400));
                        }
                        Err(_) => {
                            let _ = request.respond(
                                Response::from_string("reload failed").with_status_code(500),
                            );
                        }
                    }
                }
                (Method::Get, "/") => {
                    // Static endpoint index for discoverability (e.g. an LLM
                    // probing the port). No GL access needed, so reply directly.
                    let body = serde_json::json!({
                        "service": "functor debug runtime",
                        "endpoints": {
                            "GET /": "this endpoint index",
                            "POST /capture": "PNG (image/png) of the next rendered frame",
                            "GET /state": "runtime state JSON: frame, tts, viewport, input (held_keys + mouse), model (Debug text)",
                            "GET /scene": "current frame as JSON: camera + scene + lights",
                            "GET /trace": "paused-inspector trace: last real frame's entry-point invocations (bindings + result) replayed while paused; {paused:false, invocations:[]} while playing",
                            "POST /input": "inject input — {\"type\":\"key\",\"key\":\"w\",\"down\":true} | {\"type\":\"mouse_move\",\"x\":0,\"y\":0} | {\"type\":\"mouse_wheel\",\"delta\":1} | {\"type\":\"ui_event\",\"slot\":0,\"kind\":\"Clicked\"}",
                            "POST /time": "clock control — {\"type\":\"set\",\"tts\":2.0} (pause) | {\"type\":\"advance\",\"dts\":0.016} (step one frame) | {\"type\":\"resume\"}",
                            "POST /reload-source": "swap game logic from the request body (raw .fun source), model preserved — 400 with the load error on a broken push",
                            "POST /rewind": "coupled scene rewind — {\"frame\":42} restores model + physics to that rendered frame (pin the clock first); 400 if unrecorded/pruned"
                        }
                    })
                    .to_string();
                    let header =
                        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let _ = request.respond(Response::from_string(body).with_header(header));
                }
                _ => {
                    let _ =
                        request.respond(Response::from_string("not found").with_status_code(404));
                }
            }
        }
    });

    rx
}
