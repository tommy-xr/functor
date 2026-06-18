//! Connection substrate for persistent transports (WebSocket/TCP/UDP — Phase 2+
//! of `docs/multiplayer.md`). Like the HTTP path, two plain-data queues bridge the
//! async host and the synchronous per-frame loop:
//!
//! * **outbound** — [`ConnCommand`]s produced by reconciling the game's declared
//!   `Sub.connect`/`Sub.listen` against the live connection set, plus
//!   `Effect.send`/`Effect.close`. The host performs them.
//! * **inbound** — [`KeyedEvent`]s: a [`NetEvent`] stamped with the *key* of the
//!   `connect`/`listen` it belongs to. The key (the endpoint for a client, the
//!   bind address for a server) is the stable identity the executor uses to route
//!   an event to the right decoder, since the decoder is recomputed each frame.
//!
//! Connections are owned by the host (which does not hot-reload), so a
//! `ConnectionId` survives a game reload: the model keeps it, the host keeps the
//! socket, and the next frame's reconciliation matches the re-declared key to the
//! still-live connection. Hosts therefore treat `Connect`/`Listen` as idempotent
//! by key.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use super::{ConnectionId, MsgQueue, NetEvent};

/// A networking command for a persistent connection, performed by the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnCommand {
    /// Ensure a client connection to `url` exists, identified by `key`. Idempotent.
    Connect { key: String, url: String },
    /// Ensure a server listener bound to `addr` exists, identified by `key`.
    Listen { key: String, addr: String },
    /// Send bytes on an open connection.
    Send {
        conn: ConnectionId,
        payload: Vec<u8>,
    },
    /// Close one connection (e.g. a server dropping a single client).
    CloseConn { conn: ConnectionId },
    /// Tear down the connection/listener for `key` (no longer declared).
    CloseKey { key: String },
}

/// A [`NetEvent`] tagged with the key of the `connect`/`listen` it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyedEvent {
    pub key: String,
    pub event: NetEvent,
}

// Dylib-side singletons, reached by the host only through the runtime's exports
// (mirrors the HTTP inbox).
static CONN_OUT: Lazy<MsgQueue<ConnCommand>> = Lazy::new(MsgQueue::new);
static CONN_IN: Lazy<MsgQueue<KeyedEvent>> = Lazy::new(MsgQueue::new);

/// Queue an outbound connection command (from reconciliation / send / close).
pub fn push_conn_command(cmd: ConnCommand) {
    CONN_OUT.push(cmd);
}

/// Host: take the connection commands queued since the last drain.
pub fn drain_conn_commands() -> Vec<ConnCommand> {
    CONN_OUT.drain()
}

/// Host: drain the outbound connection commands as JSON (for the real dispatcher
/// and the debug server).
pub fn drain_conn_commands_json() -> String {
    serde_json::to_string(&drain_conn_commands()).unwrap_or_else(|_| "[]".to_string())
}

/// Host: deliver an inbound event for the connection identified by `key`.
pub fn push_conn_event(key: String, event: NetEvent) {
    CONN_IN.push(KeyedEvent { key, event });
}

/// Executor: take the inbound connection events queued since the last frame.
pub fn drain_conn_events() -> Vec<KeyedEvent> {
    CONN_IN.drain()
}

// Host (primitive ABI): one helper per event kind so the dylib's exported shim
// stays plain scalars + bytes.
pub fn push_connected(key: String, conn: ConnectionId) {
    push_conn_event(key, NetEvent::Connected(conn));
}

pub fn push_message(key: String, conn: ConnectionId, payload: Vec<u8>) {
    push_conn_event(key, NetEvent::Message(conn, payload));
}

pub fn push_disconnected(key: String, conn: ConnectionId) {
    push_conn_event(key, NetEvent::Disconnected(conn));
}

pub fn push_conn_error(key: String, conn: ConnectionId, message: String) {
    push_conn_event(key, NetEvent::Error(conn, message));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip_through_json() {
        let cmds = vec![
            ConnCommand::Connect {
                key: "wss://x/".into(),
                url: "wss://x/".into(),
            },
            ConnCommand::Send {
                conn: 7,
                payload: b"hi".to_vec(),
            },
            ConnCommand::CloseConn { conn: 7 },
            ConnCommand::CloseKey { key: "wss://x/".into() },
        ];
        let json = serde_json::to_string(&cmds).unwrap();
        assert_eq!(cmds, serde_json::from_str::<Vec<ConnCommand>>(&json).unwrap());
    }

    #[test]
    fn events_are_stamped_with_their_key_and_drain_fifo() {
        // Use a key unique to this test so it doesn't race other tests on the
        // shared inbound queue.
        let key = "test://stamped";
        push_connected(key.into(), 1);
        push_message(key.into(), 1, b"a".to_vec());
        push_disconnected(key.into(), 1);

        let mine: Vec<KeyedEvent> = drain_conn_events()
            .into_iter()
            .filter(|e| e.key == key)
            .collect();
        assert_eq!(
            mine,
            vec![
                KeyedEvent { key: key.into(), event: NetEvent::Connected(1) },
                KeyedEvent { key: key.into(), event: NetEvent::Message(1, b"a".to_vec()) },
                KeyedEvent { key: key.into(), event: NetEvent::Disconnected(1) },
            ]
        );
    }
}
