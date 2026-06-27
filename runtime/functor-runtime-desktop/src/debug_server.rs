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
//! `POST /input`, `POST /time`. See `docs/debug-runtime.md` for usage and the
//! observe-vs-drive workflows.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

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
        })
        .to_string()
    }
}

/// An input event to inject via `POST /input`. JSON is tagged by `type`:
/// `{"type":"key","key":"w","down":true}`,
/// `{"type":"mouse_move","x":10,"y":20}`,
/// `{"type":"mouse_wheel","delta":1}`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputCommand {
    Key { key: String, down: bool },
    MouseMove { x: i32, y: i32 },
    MouseWheel { delta: i32 },
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

/// A request from the HTTP thread to the GL loop. Each variant carries a
/// one-shot `Sender` the GL loop uses to reply; the HTTP handler blocks on the
/// matching `Receiver`.
pub enum DebugRequest {
    /// `POST /capture` — reply with PNG bytes of the next rendered frame.
    Capture(Sender<Vec<u8>>),
    /// `GET /state` — reply with the current runtime state.
    State(Sender<RuntimeState>),
    /// `GET /scene` — reply with the current frame (camera + scene) as JSON.
    Scene(Sender<String>),
    /// `POST /input` — inject a key/mouse event; reply Ok or an error message.
    Input(InputCommand, Sender<Result<(), String>>),
    /// `POST /time` — set/advance/resume the clock; reply once applied.
    Time(TimeCommand, Sender<()>),
}

/// Start the debug HTTP server on a background thread. Returns the receiving end
/// of the request channel; the GL loop should `try_recv()` it once per frame.
/// Binds localhost only.
pub fn spawn(port: u16) -> Receiver<DebugRequest> {
    let (tx, rx) = mpsc::channel::<DebugRequest>();

    let addr = format!("127.0.0.1:{}", port);
    let server = match Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[debug-server] failed to bind {}: {}", addr, e);
            std::process::exit(1);
        }
    };
    println!("[debug-server] listening on http://{}", addr);

    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let method = request.method().clone();
            let url = request.url().to_string();
            // Strip any query string for routing.
            let path = url.split('?').next().unwrap_or("").to_string();

            match (&method, path.as_str()) {
                (Method::Post, "/capture") => {
                    let (resp_tx, resp_rx) = mpsc::channel::<Vec<u8>>();
                    if tx.send(DebugRequest::Capture(resp_tx)).is_err() {
                        let _ = request.respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(png) => {
                            let header =
                                Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..]).unwrap();
                            let resp = Response::from_data(png).with_header(header);
                            let _ = request.respond(resp);
                        }
                        Err(_) => {
                            let _ = request
                                .respond(Response::from_string("capture failed").with_status_code(500));
                        }
                    }
                }
                (Method::Get, "/state") => {
                    let (resp_tx, resp_rx) = mpsc::channel::<RuntimeState>();
                    if tx.send(DebugRequest::State(resp_tx)).is_err() {
                        let _ = request.respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(state) => {
                            let header = Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .unwrap();
                            let resp = Response::from_string(state.to_json()).with_header(header);
                            let _ = request.respond(resp);
                        }
                        Err(_) => {
                            let _ = request
                                .respond(Response::from_string("state failed").with_status_code(500));
                        }
                    }
                }
                (Method::Get, "/scene") => {
                    let (resp_tx, resp_rx) = mpsc::channel::<String>();
                    if tx.send(DebugRequest::Scene(resp_tx)).is_err() {
                        let _ = request.respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(json) => {
                            let header = Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .unwrap();
                            let resp = Response::from_string(json).with_header(header);
                            let _ = request.respond(resp);
                        }
                        Err(_) => {
                            let _ = request
                                .respond(Response::from_string("scene failed").with_status_code(500));
                        }
                    }
                }
                (Method::Post, "/input") => {
                    let mut body = String::new();
                    if request.as_reader().read_to_string(&mut body).is_err() {
                        let _ = request.respond(Response::from_string("bad body").with_status_code(400));
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
                        let _ = request.respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(Ok(())) => {
                            let _ = request.respond(Response::from_string("ok"));
                        }
                        Ok(Err(msg)) => {
                            let _ = request.respond(Response::from_string(msg).with_status_code(400));
                        }
                        Err(_) => {
                            let _ = request
                                .respond(Response::from_string("input failed").with_status_code(500));
                        }
                    }
                }
                (Method::Post, "/time") => {
                    let mut body = String::new();
                    if request.as_reader().read_to_string(&mut body).is_err() {
                        let _ = request.respond(Response::from_string("bad body").with_status_code(400));
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
                        let _ = request.respond(Response::from_string("runtime gone").with_status_code(503));
                        continue;
                    }
                    match resp_rx.recv() {
                        Ok(()) => {
                            let _ = request.respond(Response::from_string("ok"));
                        }
                        Err(_) => {
                            let _ = request
                                .respond(Response::from_string("time failed").with_status_code(500));
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
                            "GET /state": "runtime state JSON: frame, tts, viewport, model (Debug text)",
                            "GET /scene": "current frame as JSON: camera + scene + lights",
                            "POST /input": "inject input — {\"type\":\"key\",\"key\":\"w\",\"down\":true} | {\"type\":\"mouse_move\",\"x\":0,\"y\":0} | {\"type\":\"mouse_wheel\",\"delta\":1}",
                            "POST /time": "clock control — {\"type\":\"set\",\"tts\":2.0} (pause) | {\"type\":\"advance\",\"dts\":0.016} (step one frame) | {\"type\":\"resume\"}"
                        }
                    })
                    .to_string();
                    let header =
                        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
                    let _ = request.respond(Response::from_string(body).with_header(header));
                }
                _ => {
                    let _ = request.respond(Response::from_string("not found").with_status_code(404));
                }
            }
        }
    });

    rx
}
