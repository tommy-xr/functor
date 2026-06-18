//! Native WebSocket host (Phase 2 host side, docs/multiplayer.md).
//!
//! The game declares connections via `Sub.connect`; the executor reconciles them
//! into plain-data [`ConnCommand`]s. Each frame the main loop drains those and
//! hands them to [`WsManager`], which owns the live `tokio-tungstenite` sockets.
//! Socket I/O runs on tokio tasks; events come back over a channel and are pushed
//! into the game on the main thread next frame (so all dylib calls stay on one
//! thread), mirroring the HTTP dispatch.

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use functor_runtime_common::net::ConnCommand;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

/// A socket event for the main loop to push into the game (keyed by the
/// connection's endpoint, the routing key the executor expects).
pub enum HostNetEvent {
    Connected { key: String, id: u64 },
    Message { key: String, id: u64, text: String },
    Disconnected { key: String, id: u64 },
    Error { key: String, id: u64, message: String },
}

/// What the per-connection task should do next.
enum OutMsg {
    Send(Vec<u8>),
    Close,
}

struct Conn {
    id: u64,
    out: UnboundedSender<OutMsg>,
}

/// Owns the live connections and turns [`ConnCommand`]s into socket operations.
pub struct WsManager {
    by_key: HashMap<String, Conn>,
    key_by_id: HashMap<u64, String>,
    next_id: u64,
    events: Sender<HostNetEvent>,
}

impl WsManager {
    pub fn new(events: Sender<HostNetEvent>) -> WsManager {
        WsManager {
            by_key: HashMap::new(),
            key_by_id: HashMap::new(),
            next_id: 1,
            events,
        }
    }

    pub fn handle(&mut self, cmd: ConnCommand) {
        match cmd {
            ConnCommand::Connect { key, url } => self.connect(key, url),
            // Server (`listen`) is native-only and lands in a later PR.
            ConnCommand::Listen { .. } => {}
            ConnCommand::Send { conn, payload } => self.send(conn, payload),
            ConnCommand::CloseConn { conn } => {
                if let Some(key) = self.key_by_id.get(&conn).cloned() {
                    self.close_key(&key);
                }
            }
            ConnCommand::CloseKey { key } => self.close_key(&key),
        }
    }

    fn connect(&mut self, key: String, url: String) {
        // Idempotent by key: a re-declared connection (e.g. after a hot reload)
        // reattaches to the live socket instead of opening a second one.
        if self.by_key.contains_key(&key) {
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let (out_tx, out_rx) = unbounded_channel::<OutMsg>();
        self.by_key.insert(key.clone(), Conn { id, out: out_tx });
        self.key_by_id.insert(id, key.clone());
        tokio::spawn(run_connection(key, id, url, out_rx, self.events.clone()));
    }

    fn send(&mut self, conn: u64, payload: Vec<u8>) {
        if let Some(key) = self.key_by_id.get(&conn) {
            if let Some(c) = self.by_key.get(key) {
                // Drop is harmless: a closed connection just ignores the send.
                let _ = c.out.send(OutMsg::Send(payload));
            }
        }
    }

    fn close_key(&mut self, key: &str) {
        if let Some(c) = self.by_key.remove(key) {
            self.key_by_id.remove(&c.id);
            let _ = c.out.send(OutMsg::Close);
        }
    }
}

/// One connection's lifetime: connect, then pump outgoing sends and incoming
/// frames until either side closes. Emits Connected/Message/Disconnected/Error.
async fn run_connection(
    key: String,
    id: u64,
    url: String,
    mut out_rx: UnboundedReceiver<OutMsg>,
    events: Sender<HostNetEvent>,
) {
    let ws = match tokio_tungstenite::connect_async(&url).await {
        Ok((ws, _resp)) => ws,
        Err(e) => {
            let _ = events.send(HostNetEvent::Error {
                key,
                id,
                message: e.to_string(),
            });
            return;
        }
    };
    let _ = events.send(HostNetEvent::Connected {
        key: key.clone(),
        id,
    });

    let (mut write, mut read) = ws.split();
    loop {
        tokio::select! {
            out = out_rx.recv() => match out {
                Some(OutMsg::Send(payload)) => {
                    // Send valid UTF-8 as a text frame, else binary.
                    let msg = match String::from_utf8(payload) {
                        Ok(text) => Message::Text(text),
                        Err(e) => Message::Binary(e.into_bytes()),
                    };
                    if write.send(msg).await.is_err() {
                        break;
                    }
                }
                Some(OutMsg::Close) | None => {
                    let _ = write.send(Message::Close(None)).await;
                    break;
                }
            },
            incoming = read.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let _ = events.send(HostNetEvent::Message { key: key.clone(), id, text });
                }
                Some(Ok(Message::Binary(data))) => {
                    let _ = events.send(HostNetEvent::Message {
                        key: key.clone(),
                        id,
                        text: String::from_utf8_lossy(&data).to_string(),
                    });
                }
                Some(Ok(Message::Close(_))) | None => break,
                // Ping/Pong/frame: handled by tungstenite / ignored here.
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    let _ = events.send(HostNetEvent::Error {
                        key: key.clone(),
                        id,
                        message: e.to_string(),
                    });
                    break;
                }
            },
        }
    }

    let _ = events.send(HostNetEvent::Disconnected { key, id });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A localhost WebSocket echo server on an OS-assigned port. Hermetic.
    async fn spawn_echo_server() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    while let Some(Ok(msg)) = ws.next().await {
                        if msg.is_text() || msg.is_binary() {
                            let _ = ws.send(msg).await;
                        }
                    }
                });
            }
        });
        port
    }

    fn recv(rx: &std::sync::mpsc::Receiver<HostNetEvent>) -> HostNetEvent {
        rx.recv_timeout(Duration::from_secs(5))
            .expect("expected a host net event within 5s")
    }

    // Multi-thread so the test thread can block on recv while tasks run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_send_and_echo_back() {
        let port = spawn_echo_server().await;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut mgr = WsManager::new(tx);
        let key = format!("ws://127.0.0.1:{port}/");

        mgr.handle(ConnCommand::Connect {
            key: key.clone(),
            url: key.clone(),
        });

        let id = match recv(&rx) {
            HostNetEvent::Connected { key: k, id } => {
                assert_eq!(k, key);
                id
            }
            _ => panic!("expected Connected first"),
        };

        mgr.handle(ConnCommand::Send {
            conn: id,
            payload: b"ping".to_vec(),
        });

        match recv(&rx) {
            HostNetEvent::Message { id: i, text, .. } => {
                assert_eq!(i, id);
                assert_eq!(text, "ping");
            }
            _ => panic!("expected the echoed Message"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_to_dead_port_errors() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut mgr = WsManager::new(tx);
        // Port 1 is privileged and almost certainly closed.
        mgr.handle(ConnCommand::Connect {
            key: "ws://127.0.0.1:1/".into(),
            url: "ws://127.0.0.1:1/".into(),
        });
        match recv(&rx) {
            HostNetEvent::Error { .. } => {}
            _ => panic!("expected an Error connecting to a dead port"),
        }
    }
}
