//! The async I/O seam between the pure game loop and the imperative shell
//! (Phase 1 of `docs/multiplayer.md`).
//!
//! Networking is asynchronous, but the MVU loop is a synchronous per-frame step.
//! These two plain-data queues bridge them:
//!
//! * **outbound** — `Effect::run` on a networking effect pushes a [`NetCommand`]
//!   here instead of producing a message in-frame. The host runtime drains it
//!   each frame and performs the real I/O (reqwest on native, `fetch` on wasm).
//! * **inbox** — when that I/O completes (possibly several frames later), the
//!   host pushes a [`NetInbound`] result here. The game executor drains it each
//!   frame and runs the matching `Sub` decoder, correlating by `token`.
//!
//! Both ends carry only plain data — no sockets, no closures — so networking
//! effects keep the hot-reload `effects-plain-data` invariant. Note the queues
//! live on the *game* (dylib) side; the host interacts with them only through the
//! runtime's exported functions, never by sharing this static across the dylib
//! boundary (each linkage has its own copy).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use fable_library_rust::NativeArray_;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// HTTP verb for an outbound request. (Phase 1 wires GET/POST end to end; the
/// rest are here so the wire format is stable as later methods are exercised.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

/// A plain-data networking command produced by an `Effect` and performed by the
/// host shell. `token` is chosen by the game and echoed back on the matching
/// [`NetInbound`] so request and response can be correlated without a closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetCommand {
    HttpRequest {
        token: u64,
        method: HttpMethod,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
}

/// A plain-data result delivered back to the game through the inbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetInbound {
    HttpResponse {
        token: u64,
        status: u16,
        body: Vec<u8>,
    },
    HttpError {
        token: u64,
        message: String,
    },
}

/// Flattened, F#-facing view of a [`NetInbound`]: one struct the game's `Sub`
/// decoder reads via field accessors, instead of a two-variant enum. A transport
/// error is `status == 0` with `error` set (`is_ok() == false`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResult {
    pub token: u64,
    pub status: u16,
    pub body: Vec<u8>,
    pub error: Option<String>,
}

impl HttpResult {
    /// The response body decoded as UTF-8 (lossy). Phase 1 games are text/JSON;
    /// a raw-bytes accessor can come later.
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }

    /// The error message, or an empty string on success.
    pub fn error_text(&self) -> String {
        self.error.clone().unwrap_or_default()
    }
}

impl From<NetInbound> for HttpResult {
    fn from(inbound: NetInbound) -> HttpResult {
        match inbound {
            NetInbound::HttpResponse {
                token,
                status,
                body,
            } => HttpResult {
                token,
                status,
                body,
                error: None,
            },
            NetInbound::HttpError { token, message } => HttpResult {
                token,
                status: 0,
                body: Vec::new(),
                error: Some(message),
            },
        }
    }
}

/// A thread-safe FIFO queue of plain-data items. Native I/O completes on tokio
/// worker threads, so the queue must be `Sync`; on wasm it is single-threaded and
/// the lock is uncontended.
#[derive(Clone)]
pub struct MsgQueue<T> {
    inner: Arc<Mutex<VecDeque<T>>>,
}

impl<T> Default for MsgQueue<T> {
    fn default() -> Self {
        MsgQueue {
            inner: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

impl<T> MsgQueue<T> {
    pub fn new() -> MsgQueue<T> {
        MsgQueue::default()
    }

    pub fn push(&self, item: T) {
        self.inner.lock().unwrap().push_back(item);
    }

    /// Remove and return everything currently queued, in FIFO order.
    pub fn drain(&self) -> Vec<T> {
        let mut q = self.inner.lock().unwrap();
        q.drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Dylib-side singletons. `Effect::run` and the runtime's exported drain/push
// functions all run inside the game dylib, so they share these instances; the
// host reaches them only through those exports.
static OUTBOUND: Lazy<MsgQueue<NetCommand>> = Lazy::new(MsgQueue::new);
static INBOX: Lazy<MsgQueue<NetInbound>> = Lazy::new(MsgQueue::new);

/// Queue a command produced by a networking `Effect` for the host to perform.
pub fn push_command(cmd: NetCommand) {
    OUTBOUND.push(cmd);
}

/// Host: take every command queued since the last drain.
pub fn drain_commands() -> Vec<NetCommand> {
    OUTBOUND.drain()
}

/// Host: deliver a completed I/O result back to the game.
pub fn push_inbound(item: NetInbound) {
    INBOX.push(item);
}

/// Host (primitive ABI): deliver a successful HTTP response. Kept to plain scalars
/// + bytes so the dylib's exported `no_mangle` shim stays trivial.
pub fn push_http_response(token: u64, status: u16, body: Vec<u8>) {
    push_inbound(NetInbound::HttpResponse {
        token,
        status,
        body,
    });
}

/// Host (primitive ABI): deliver a transport-level failure for a request.
pub fn push_http_error(token: u64, message: String) {
    push_inbound(NetInbound::HttpError { token, message });
}

/// Host: drain the outbound commands as a JSON array. Used both by the real
/// dispatcher and by the debug server to show what the game has requested.
pub fn drain_commands_json() -> String {
    serde_json::to_string(&drain_commands()).unwrap_or_else(|_| "[]".to_string())
}

/// Executor: take every inbound result queued since the last frame.
pub fn drain_inbound() -> Vec<NetInbound> {
    INBOX.drain()
}

/// Executor (F#-facing): drain the inbox as an array of [`HttpResult`], ready for
/// a `Sub` decoder. Returns a Fable `NativeArray` so it crosses to F# as `array`.
pub fn drain_http_results() -> NativeArray_::Array<HttpResult> {
    let results: Vec<HttpResult> = INBOX.drain().into_iter().map(HttpResult::from).collect();
    NativeArray_::array_from(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_is_fifo_and_drains_empty() {
        let q: MsgQueue<u32> = MsgQueue::new();
        assert!(q.is_empty());
        q.push(1);
        q.push(2);
        q.push(3);
        assert_eq!(q.len(), 3);
        assert_eq!(q.drain(), vec![1, 2, 3]);
        assert!(q.is_empty());
        assert_eq!(q.drain(), Vec::<u32>::new());
    }

    #[test]
    fn queue_clone_shares_backing_store() {
        // The host holds a clone of the same queue the I/O tasks push to.
        let a: MsgQueue<u32> = MsgQueue::new();
        let b = a.clone();
        a.push(10);
        b.push(20);
        assert_eq!(a.drain(), vec![10, 20]);
    }

    #[test]
    fn commands_survive_a_serde_roundtrip() {
        // Crossing to the wasm host goes through serde; keep the format stable.
        let cmd = NetCommand::HttpRequest {
            token: 7,
            method: HttpMethod::Post,
            url: "https://example.com/x".to_string(),
            headers: vec![("content-type".into(), "application/json".into())],
            body: b"{}".to_vec(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: NetCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn http_result_flattens_response_and_error() {
        let ok: HttpResult = NetInbound::HttpResponse {
            token: 1,
            status: 200,
            body: b"hi".to_vec(),
        }
        .into();
        assert!(ok.is_ok());
        assert_eq!(ok.status, 200);
        assert_eq!(ok.body_text(), "hi");
        assert_eq!(ok.error_text(), "");

        let err: HttpResult = NetInbound::HttpError {
            token: 2,
            message: "boom".to_string(),
        }
        .into();
        assert!(!err.is_ok());
        assert_eq!(err.status, 0);
        assert_eq!(err.error_text(), "boom");
    }

    #[test]
    fn inbound_survives_a_serde_roundtrip() {
        let ok = NetInbound::HttpResponse {
            token: 7,
            status: 200,
            body: b"hello".to_vec(),
        };
        let err = NetInbound::HttpError {
            token: 8,
            message: "dns failure".to_string(),
        };
        for v in [ok, err] {
            let json = serde_json::to_string(&v).unwrap();
            assert_eq!(v, serde_json::from_str::<NetInbound>(&json).unwrap());
        }
    }
}
