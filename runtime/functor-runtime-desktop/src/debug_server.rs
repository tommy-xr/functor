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
//! This is the seed of a richer debug runtime (future `/input`, `/time`, etc.).

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use tiny_http::{Header, Method, Response, Server};

/// A snapshot of runtime state the GL loop can read on the main thread, returned
/// for `GET /state`. Kept deliberately small for this first slice — model/game
/// state is opaque across the dylib boundary (`emit_state` bundles model + effect
/// queue into binary `OpaqueState` for hot-reload, not JSON), so we report the
/// runtime status the shell owns.
pub struct RuntimeState {
    pub frame: u64,
    pub tts: f32,
    pub width: u32,
    pub height: u32,
}

impl RuntimeState {
    /// Hand-rolled JSON so we don't pull in serde just for this.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"frame\":{},\"tts\":{},\"viewport\":{{\"width\":{},\"height\":{}}}}}",
            self.frame, self.tts, self.width, self.height
        )
    }
}

/// A request from the HTTP thread to the GL loop. Each variant carries a
/// one-shot `Sender` the GL loop uses to reply; the HTTP handler blocks on the
/// matching `Receiver`.
pub enum DebugRequest {
    /// `POST /capture` — reply with PNG bytes of the next rendered frame.
    Capture(Sender<Vec<u8>>),
    /// `GET /state` — reply with the current runtime state.
    State(Sender<RuntimeState>),
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
        for request in server.incoming_requests() {
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
                _ => {
                    let _ = request.respond(Response::from_string("not found").with_status_code(404));
                }
            }
        }
    });

    rx
}
