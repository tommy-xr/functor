//! Native WebSocket host (Phase 2 host side, docs/multiplayer.md).
//!
//! The game declares connections via `Sub.connect` / `Sub.listen`; the executor
//! reconciles them into plain-data [`ConnCommand`]s. Each frame the main loop
//! drains those and hands them to [`WsManager`], which owns the live
//! `tokio-tungstenite` sockets (client connections and server-accepted clients
//! alike). Socket I/O runs on tokio tasks; events come back over a channel and
//! are pushed into the game on the main thread next frame, mirroring HTTP.
//!
//! Live connections share one `Arc<Mutex<HashMap<id, ConnEntry>>>` so an accept
//! task can register a freshly-accepted client that the main thread then sends
//! on. Each event is stamped with its *key* — the endpoint url for a client, the
//! bind address for a server's clients — the routing key the executor expects.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use functor_runtime_common::net::ConnCommand;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

/// A socket event for the main loop to push into the game (keyed by the
/// connection's routing key — endpoint for a client, bind addr for a server).
pub enum HostNetEvent {
    Connected {
        key: String,
        id: u64,
    },
    Message {
        key: String,
        id: u64,
        text: String,
    },
    Disconnected {
        key: String,
        id: u64,
    },
    Error {
        key: String,
        id: u64,
        message: String,
    },
}

/// What a per-connection task should do next.
enum OutMsg {
    Send(Vec<u8>),
    Close,
}

struct ConnEntry {
    out: UnboundedSender<OutMsg>,
    key: String,
}

type SharedConns = Arc<Mutex<HashMap<u64, ConnEntry>>>;

/// Owns the live connections and turns [`ConnCommand`]s into socket operations.
pub struct WsManager {
    conns: SharedConns,
    /// Connect-initiated client connections: url key -> id (for idempotent connect
    /// and CloseKey). Server-accepted clients are not here (many per listener).
    client_by_key: HashMap<String, u64>,
    /// Active listeners: bind key -> accept-loop task.
    listeners: HashMap<String, tokio::task::JoinHandle<()>>,
    next_id: Arc<AtomicU64>,
    events: Sender<HostNetEvent>,
}

impl WsManager {
    pub fn new(events: Sender<HostNetEvent>) -> WsManager {
        WsManager {
            conns: Arc::new(Mutex::new(HashMap::new())),
            client_by_key: HashMap::new(),
            listeners: HashMap::new(),
            next_id: Arc::new(AtomicU64::new(1)),
            events,
        }
    }

    pub fn handle(&mut self, cmd: ConnCommand) {
        match cmd {
            ConnCommand::Connect { key, url } => self.connect(key, url),
            ConnCommand::Listen { key, addr } => self.listen(key, addr),
            ConnCommand::Send { conn, payload } => self.send_to(conn, OutMsg::Send(payload)),
            ConnCommand::CloseConn { conn } => self.send_to(conn, OutMsg::Close),
            ConnCommand::CloseKey { key } => self.close_key(&key),
        }
    }

    fn send_to(&self, conn: u64, msg: OutMsg) {
        if let Some(entry) = self.conns.lock().unwrap().get(&conn) {
            // Drop is harmless: a closed connection just ignores the send.
            let _ = entry.out.send(msg);
        }
    }

    fn connect(&mut self, key: String, url: String) {
        // Idempotent by key: a re-declared connection (e.g. after a hot reload)
        // reattaches to the live socket instead of opening a second one.
        if self.client_by_key.contains_key(&key) {
            return;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.client_by_key.insert(key.clone(), id);
        tokio::spawn(run_client(
            key,
            id,
            url,
            self.events.clone(),
            self.conns.clone(),
        ));
    }

    fn listen(&mut self, key: String, addr: String) {
        if self.listeners.contains_key(&key) {
            return;
        }
        let handle = tokio::spawn(accept_loop(
            key.clone(),
            addr,
            self.next_id.clone(),
            self.events.clone(),
            self.conns.clone(),
        ));
        self.listeners.insert(key, handle);
    }

    fn close_key(&mut self, key: &str) {
        // A client connection for this key.
        if let Some(id) = self.client_by_key.remove(key) {
            self.send_to(id, OutMsg::Close);
        }
        // A listener: stop accepting and close its accepted clients.
        if let Some(handle) = self.listeners.remove(key) {
            handle.abort();
            let conns = self.conns.lock().unwrap();
            for entry in conns.values() {
                if entry.key == key {
                    let _ = entry.out.send(OutMsg::Close);
                }
            }
        }
    }
}

/// A client connection: connect, register, then pump until either side closes.
async fn run_client(
    key: String,
    id: u64,
    url: String,
    events: Sender<HostNetEvent>,
    conns: SharedConns,
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
    serve(ws, key, id, events, conns).await;
}

/// Accept connections until the listener is dropped/aborted; each accepted client
/// becomes its own connection sharing the listener's key.
async fn accept_loop(
    key: String,
    addr: String,
    next_id: Arc<AtomicU64>,
    events: Sender<HostNetEvent>,
    conns: SharedConns,
) {
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            let _ = events.send(HostNetEvent::Error {
                key,
                id: 0,
                message: format!("bind {addr}: {e}"),
            });
            return;
        }
    };
    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let key = key.clone();
                let events = events.clone();
                let conns = conns.clone();
                tokio::spawn(async move {
                    match tokio_tungstenite::accept_async(stream).await {
                        Ok(ws) => serve(ws, key, id, events, conns).await,
                        Err(e) => {
                            let _ = events.send(HostNetEvent::Error {
                                key,
                                id,
                                message: e.to_string(),
                            });
                        }
                    }
                });
            }
            Err(_) => break,
        }
    }
}

/// Register a connected socket, emit Connected, pump frames until close, then
/// deregister and emit Disconnected. Generic over the underlying stream so it
/// serves both client (`MaybeTlsStream`) and server (`TcpStream`) sockets.
async fn serve<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    key: String,
    id: u64,
    events: Sender<HostNetEvent>,
    conns: SharedConns,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (out_tx, mut out_rx) = unbounded_channel::<OutMsg>();
    conns.lock().unwrap().insert(
        id,
        ConnEntry {
            out: out_tx,
            key: key.clone(),
        },
    );
    let _ = events.send(HostNetEvent::Connected {
        key: key.clone(),
        id,
    });

    let (mut write, mut read) = ws.split();
    loop {
        tokio::select! {
            out = out_rx.recv() => match out {
                Some(OutMsg::Send(payload)) => {
                    // Valid UTF-8 -> text frame, else binary.
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
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    let _ = events.send(HostNetEvent::Error { key: key.clone(), id, message: e.to_string() });
                    break;
                }
            },
        }
    }

    conns.lock().unwrap().remove(&id);
    let _ = events.send(HostNetEvent::Disconnected { key, id });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn recv(rx: &std::sync::mpsc::Receiver<HostNetEvent>) -> HostNetEvent {
        rx.recv_timeout(Duration::from_secs(5))
            .expect("expected a host net event within 5s")
    }

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

    /// A free localhost port (small race window between drop and rebind).
    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_connect_send_and_echo_back() {
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
            other => panic!("expected the echoed Message, got {:?}", debug(&other)),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_to_dead_port_errors() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut mgr = WsManager::new(tx);
        mgr.handle(ConnCommand::Connect {
            key: "ws://127.0.0.1:1/".into(),
            url: "ws://127.0.0.1:1/".into(),
        });
        match recv(&rx) {
            HostNetEvent::Error { .. } => {}
            _ => panic!("expected an Error connecting to a dead port"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn server_accepts_a_client_and_can_reply() {
        let port = free_port();
        let bind = format!("127.0.0.1:{port}");
        let key = format!("ws://{bind}/");
        let (tx, rx) = std::sync::mpsc::channel();
        let mut mgr = WsManager::new(tx);

        // Start listening, then connect a real client to it (retry while the
        // accept loop binds).
        mgr.handle(ConnCommand::Listen {
            key: key.clone(),
            addr: bind.clone(),
        });
        let url = format!("ws://{bind}/");
        let (mut client, _resp) = {
            let mut pair = None;
            for _ in 0..50 {
                if let Ok(ws) = tokio_tungstenite::connect_async(&url).await {
                    pair = Some(ws);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            pair.expect("client could not connect to the listener")
        };

        // The manager sees the accepted client.
        let id = match recv(&rx) {
            HostNetEvent::Connected { key: k, id } => {
                assert_eq!(k, key);
                id
            }
            _ => panic!("expected Connected for the accepted client"),
        };

        // Client -> server message surfaces as a keyed Message.
        client
            .send(Message::Text("hi-server".into()))
            .await
            .unwrap();
        match recv(&rx) {
            HostNetEvent::Message {
                key: k,
                id: i,
                text,
            } => {
                assert_eq!(k, key);
                assert_eq!(i, id);
                assert_eq!(text, "hi-server");
            }
            _ => panic!("expected the client's Message"),
        }

        // Server -> client reply reaches the client.
        mgr.handle(ConnCommand::Send {
            conn: id,
            payload: b"welcome".to_vec(),
        });
        match client.next().await {
            Some(Ok(Message::Text(t))) => assert_eq!(t, "welcome"),
            other => panic!("expected welcome, got {other:?}"),
        }
    }

    fn debug(e: &HostNetEvent) -> String {
        match e {
            HostNetEvent::Connected { id, .. } => format!("Connected({id})"),
            HostNetEvent::Message { id, text, .. } => format!("Message({id}, {text})"),
            HostNetEvent::Disconnected { id, .. } => format!("Disconnected({id})"),
            HostNetEvent::Error { message, .. } => format!("Error({message})"),
        }
    }
}
