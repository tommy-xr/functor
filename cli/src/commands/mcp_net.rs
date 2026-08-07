//! The NATIVE net coordinator: the MCP host process as a multiplayer game's
//! network.
//!
//! Design note: `~/notes` Addendum 7 — "the embedder is the MCP host process".
//! The web runtime already routes its networking to the page embedding it
//! (`runtime/functor-runtime-web/src/lib.rs`, and the host-page coordinator in
//! `site/src/net-coordinator.ts`). That seam is not browser-specific: a native
//! runtime started with `--net-transport embedder` opens no socket, queues its
//! drained [`ConnCommand`]s for `GET /net/outbound`, and takes events back
//! over `POST /net/deliver`. This module is the thing in between — one
//! coordinator protocol, of which MCP is the headless frontend.
//!
//! The ROUTING MODEL is `site/src/net-coordinator.ts`'s, and through it
//! `functor_runtime_common::net::VirtualNet`'s:
//!
//!   * a listener registry keyed by AUTHORITY (host:port, scheme and path
//!     ignored), so a client url `ws://127.0.0.1:9101/orbs` matches a server
//!     bind `127.0.0.1:9101`;
//!   * ONE connection id per PAIR, shared by both ends, so a send from either
//!     end routes to the other by looking up the same row;
//!   * both ends are told `connected`, each under ITS OWN routing key — the
//!     client under the url it dialled, the server under its bind — because
//!     that key is how each runtime routes the event back to the right
//!     `Sub.connect` / `Sub.listen` tagger;
//!   * FIFO per session: deliveries queue in arrival order and flush as one
//!     batch.
//!
//! PERFECT LINKS ONLY, and delivery is IMMEDIATE: a routed packet is handed to
//! its destination on the coordinator's next pump, in order. The ROUTER never
//! drops or reorders one — a delivery the pump could not complete comes back
//! to the front of its queue ([`NetCoordinator::requeue`]).
//! Everything a link profile would add — latency, jitter, partitions — and
//! step-TIME scheduling ("departs step 610, deliver at 618") arrive with
//! barrier stepping, which is the PR that also gives [`Packet::frame`] a
//! meaning; until then it is `None` rather than a measurement dressed up as a
//! schedule.
//!
//! This module is pure: it performs no I/O and knows nothing about HTTP. The
//! pump that drains and delivers lives in `mcp.rs`.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use functor_runtime_common::net::{ConnCommand, DeliveredEvent};

/// Newest entries win: a long session must not grow without bound.
const PACKET_LOG_CAP: usize = 10_000;

/// …and a second bound, on the payload TEXT the log retains: a count alone is
/// not a memory bound, because a game chooses how big a message is.
const PACKET_LOG_BYTES: usize = 8 << 20;

/// Bound on a single session's undelivered queue.
///
/// The log is capped but the OUTBOX was not: a member that stops answering
/// (its child died without a `stop_game`) keeps having packets routed TO it by
/// peers that are still running, forever. Past this, the oldest queued events
/// are dropped and COUNTED — a coordinator that advertises a perfect link must
/// not discard traffic silently, and a queue that has grown this far belongs
/// to a session that is not coming back.
const OUTBOX_CAP: usize = 4_096;

/// How long a `Connect` with no listener waits for its server session before it
/// errors.
///
/// It cannot fail fast: a runtime reconciles `Sub.connect` against its DECLARED
/// keys, so a key emitted once is never re-emitted
/// (`functor_lang_producer::reconcile_connections`). An error delivered to a
/// client whose server has not bound yet would therefore be permanent — and so
/// would one delivered while the server session is RELOADING (which is why
/// `close` re-queues the client end, see there).
const CONNECT_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// One routed packet — the wire, as data.
#[derive(Clone, Debug)]
pub struct Packet {
    /// Monotonic within a group, and never reused: `wire_log`'s `since` cursor,
    /// which is what lets a caller ask for exactly the rows a `step_all` round
    /// produced.
    pub seq: u64,
    /// Milliseconds since the coordinator started, for ordering against
    /// wall-clock events. NOT game time.
    pub at_ms: f64,
    /// The session frame this packet belongs to. Always `None` in v1:
    /// delivery is immediate and unscheduled, so there is no step to name.
    /// Barrier stepping fills it in by construction.
    pub frame: Option<u64>,
    /// Session label the packet came from.
    pub from: String,
    /// Session label it was delivered to.
    pub to: String,
    pub conn: i32,
    pub kind: &'static str,
    /// Payload bytes for a message; 0 for the lifecycle kinds.
    pub size: usize,
    /// The message's wire TEXT, exactly as the receiving runtime sees it.
    /// Decoding is deliberately NOT done here: the framing is the game's
    /// (`Effect.sendMsg`'s JSON), and an agent reading the log decodes it —
    /// a coordinator that parsed payloads would be inventing a protocol it
    /// does not own.
    pub payload_text: Option<String>,
}

/// One end of a connection: the session, and the key ITS runtime routes by.
#[derive(Clone, Debug)]
struct Side {
    session: String,
    key: String,
}

#[derive(Clone, Debug)]
struct Conn {
    client: Side,
    server: Side,
}

struct PendingConnect {
    session: String,
    key: String,
    since: Instant,
}

/// The authority (host:port) of an endpoint, ignoring scheme and path — the
/// same rule `VirtualNet` and the TS coordinator apply, so all three agree on
/// what matches.
fn authority_of(endpoint: &str) -> &str {
    let after_scheme = endpoint.rsplit("://").next().unwrap_or(endpoint);
    after_scheme.split('/').next().unwrap_or(after_scheme)
}

pub struct NetCoordinator {
    /// Registered sessions, in launch order (the order the log reads best in).
    sessions: Vec<String>,
    /// Events queued for each session, flushed in order by the pump.
    outbox: BTreeMap<String, VecDeque<DeliveredEvent>>,
    /// authority -> the listening side.
    listeners: BTreeMap<String, Side>,
    /// Shared connection id -> its two ends. Ids start at 1: 0 is the "no
    /// connection" id an unroutable-connect error carries.
    conns: BTreeMap<i32, Conn>,
    pending: Vec<PendingConnect>,
    log: VecDeque<Packet>,
    log_bytes: usize,
    /// Events shed from an over-full outbox, per session, awaiting a report.
    dropped: BTreeMap<String, usize>,
    next_conn: i32,
    next_seq: u64,
    started: Instant,
}

impl NetCoordinator {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            outbox: BTreeMap::new(),
            listeners: BTreeMap::new(),
            conns: BTreeMap::new(),
            pending: Vec::new(),
            log: VecDeque::new(),
            log_bytes: 0,
            dropped: BTreeMap::new(),
            // Ids start at 1: 0 is the "no connection" id an unroutable-connect
            // error carries, so there is no valid `Default` for this type.
            next_conn: 1,
            next_seq: 0,
            started: Instant::now(),
        }
    }

    /// Register a session under the label the wire log shows ("server",
    /// "client1").
    pub fn add_session(&mut self, label: &str) {
        if !self.sessions.iter().any(|s| s == label) {
            self.sessions.push(label.to_string());
        }
        self.outbox.entry(label.to_string()).or_default();
    }

    /// Drop a session and close every connection it held (both ends notified).
    pub fn remove_session(&mut self, label: &str) {
        self.reset_session(label);
        self.sessions.retain(|s| s != label);
        self.outbox.remove(label);
    }

    pub fn labels(&self) -> &[String] {
        &self.sessions
    }

    /// Close everything a session owns without deregistering it — what a
    /// RELOAD of that role means to the network. The peers are told
    /// `disconnected`, and a client whose server went away is re-queued (see
    /// [`Self::close`]) so the role's next `Listen` re-opens it.
    pub fn reset_session(&mut self, label: &str) {
        let owned: Vec<i32> = self
            .conns
            .iter()
            .filter(|(_, row)| row.client.session == label || row.server.session == label)
            .map(|(id, _)| *id)
            .collect();
        for conn in owned {
            self.close(conn, label);
        }
        self.listeners.retain(|_, side| side.session != label);
        self.pending.retain(|p| p.session != label);
        if let Some(outbox) = self.outbox.get_mut(label) {
            outbox.clear();
        }
    }

    /// Route one command a session's runtime emitted.
    pub fn perform(&mut self, session: &str, command: ConnCommand) {
        match command {
            ConnCommand::Listen { key, .. } => {
                // Idempotent by authority, like every host treats `Listen` (a
                // hot reload re-declares it every time). The newest wins.
                self.listeners.insert(
                    authority_of(&key).to_string(),
                    Side {
                        session: session.to_string(),
                        key,
                    },
                );
                // A client that dialled before this role booted is waiting.
                self.retry_pending();
            }
            ConnCommand::Connect { key, .. } => {
                // Idempotent by key: a re-declare while the connection is
                // already up must not open a second one.
                if self.conn_for(session, &key).is_some() {
                    return;
                }
                if self
                    .pending
                    .iter()
                    .any(|p| p.session == session && p.key == key)
                {
                    return;
                }
                if !self.open(session, &key) {
                    self.pending.push(PendingConnect {
                        session: session.to_string(),
                        key,
                        since: Instant::now(),
                    });
                }
            }
            ConnCommand::Send { conn, payload } => {
                // A `ConnectionId` is a u64 while the delivery side is an i32:
                // truncating would let a bogus id (a stale one a model carried
                // across a reload) ALIAS a live connection and be delivered
                // instead of dropped. Refuse it the way `peer_of` would.
                let Ok(conn) = i32::try_from(conn) else {
                    return;
                };
                // A send on a connection this session is not an end of (a
                // closed one, or a stale id a model carried across a reload) is
                // DROPPED, not misrouted — `VirtualNet::send` refuses the same
                // case via `peer_of`.
                let Some(peer) = self.peer_of(conn, session) else {
                    return;
                };
                let size = payload.len();
                let text = String::from_utf8_lossy(&payload).into_owned();
                self.deliver(
                    session,
                    &peer,
                    DeliveredEvent::Message {
                        key: peer.key.clone(),
                        conn,
                        text,
                    },
                    size,
                );
            }
            ConnCommand::CloseConn { conn } => {
                let Ok(conn) = i32::try_from(conn) else {
                    return;
                };
                // Same ownership rule: only an end may close it.
                if self.peer_of(conn, session).is_some() {
                    self.close(conn, session);
                }
            }
            ConnCommand::CloseKey { key } => {
                let authority = authority_of(&key).to_string();
                if self
                    .listeners
                    .get(&authority)
                    .is_some_and(|side| side.session == session && side.key == key)
                {
                    self.listeners.remove(&authority);
                }
                let owned: Vec<i32> = self
                    .conns
                    .iter()
                    .filter(|(_, row)| {
                        (row.client.session == session && row.client.key == key)
                            || (row.server.session == session && row.server.key == key)
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for conn in owned {
                    self.close(conn, session);
                }
                self.pending
                    .retain(|p| !(p.session == session && p.key == key));
            }
        }
    }

    /// Retry connects whose listener had not booted yet; give up after the
    /// grace window with a teaching error on the CALLER's key.
    ///
    /// In ARRIVAL order, so connection ids (and therefore the server's player
    /// order) follow the order the clients asked, not the reverse.
    pub fn retry_pending(&mut self) {
        let now = Instant::now();
        let requests: Vec<PendingConnect> = self.pending.drain(..).collect();
        for request in requests {
            if now.duration_since(request.since) > CONNECT_GRACE {
                let authority = authority_of(&request.key).to_string();
                let side = Side {
                    session: request.session.clone(),
                    key: request.key.clone(),
                };
                self.deliver(
                    &request.session,
                    &side,
                    DeliveredEvent::Error {
                        key: request.key,
                        conn: 0,
                        message: format!(
                            "no session is listening on {authority} — a role must declare \
Sub.listen(\"{authority}\", …) before a client can connect"
                        ),
                    },
                    0,
                );
                continue;
            }
            if !self.open(&request.session, &request.key) {
                self.pending.push(request);
            }
        }
    }

    /// Take everything queued for a session, in arrival order.
    pub fn take_outbox(&mut self, label: &str) -> Vec<DeliveredEvent> {
        self.outbox
            .get_mut(label)
            .map(|queue| queue.drain(..).collect())
            .unwrap_or_default()
    }

    /// Put an undelivered batch back at the FRONT, so a transport hiccup
    /// retries in order rather than losing the packets. The log already
    /// recorded them: the log is what the coordinator ROUTED.
    pub fn requeue(&mut self, label: &str, events: Vec<DeliveredEvent>) {
        let Some(queue) = self.outbox.get_mut(label) else {
            return;
        };
        for event in events.into_iter().rev() {
            queue.push_front(event);
        }
    }

    /// Take the per-session drop counts accumulated since the last call, so
    /// the pump can report them once rather than per packet.
    pub fn take_drops(&mut self) -> BTreeMap<String, usize> {
        std::mem::take(&mut self.dropped)
    }

    /// Every packet routed so far, oldest first (capped).
    pub fn packets(&self) -> impl Iterator<Item = &Packet> {
        self.log.iter()
    }

    /// The oldest sequence number the log still holds — how a reader learns its
    /// `since` cursor fell off the end rather than silently seeing a gap.
    pub fn oldest_seq(&self) -> Option<u64> {
        self.log.front().map(|packet| packet.seq)
    }

    /// How many packets have EVER been routed, including trimmed ones.
    pub fn routed(&self) -> u64 {
        self.next_seq
    }

    /// Live connections per session — the only place that knows who is
    /// actually talking to whom.
    pub fn connection_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for row in self.conns.values() {
            for side in [&row.client, &row.server] {
                *counts.entry(side.session.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    // ------------------------------------------------------------------ routing

    /// Resolve a client key to its listening session and wire the pair up.
    /// False when nobody is listening on that authority (the caller waits).
    ///
    /// KNOWN, inherited from `VirtualNet` (whose `peer_of` has the same
    /// degeneracy): a session that both listens on and connects to its OWN
    /// authority — a host that also plays — gets both ends of one row.
    fn open(&mut self, session: &str, key: &str) -> bool {
        let Some(server) = self.listeners.get(authority_of(key)).cloned() else {
            return false;
        };
        if !self.sessions.iter().any(|s| *s == server.session) {
            return false;
        }
        let conn = self.next_conn;
        self.next_conn += 1;
        let client = Side {
            session: session.to_string(),
            key: key.to_string(),
        };
        self.conns.insert(
            conn,
            Conn {
                client: client.clone(),
                server: server.clone(),
            },
        );
        // BOTH ends learn about the connection, each under its own routing key.
        // Each is logged as coming FROM its peer, so the log reads as traffic
        // between two sessions rather than as one talking to itself.
        self.deliver(
            &server.session,
            &client,
            DeliveredEvent::Connected {
                key: key.to_string(),
                conn,
            },
            0,
        );
        self.deliver(
            session,
            &server,
            DeliveredEvent::Connected {
                key: server.key.clone(),
                conn,
            },
            0,
        );
        true
    }

    fn close(&mut self, conn: i32, by_session: &str) {
        let Some(row) = self.conns.remove(&conn) else {
            return;
        };
        for side in [row.client.clone(), row.server.clone()] {
            self.deliver(
                by_session,
                &side,
                DeliveredEvent::Disconnected {
                    key: side.key.clone(),
                    conn,
                },
                0,
            );
        }
        // A client whose peer went away (the server role reloaded, or closed
        // the connection) can never ask again: its runtime only emits `Connect`
        // for a key it has not already declared, and a `disconnected` does not
        // clear that. So the coordinator re-queues the client end on its
        // behalf, and the server's next `Listen` re-opens it. Without this,
        // reloading the authority kills every client for the rest of the run.
        if by_session != row.client.session
            && self.sessions.iter().any(|s| *s == row.client.session)
            && !self
                .pending
                .iter()
                .any(|p| p.session == row.client.session && p.key == row.client.key)
        {
            self.pending.push(PendingConnect {
                session: row.client.session,
                key: row.client.key,
                since: Instant::now(),
            });
        }
    }

    /// The peer end of `conn` as seen from `session`, or `None` when that
    /// session is not on the connection.
    fn peer_of(&self, conn: i32, session: &str) -> Option<Side> {
        let row = self.conns.get(&conn)?;
        if row.client.session == session {
            return Some(row.server.clone());
        }
        if row.server.session == session {
            return Some(row.client.clone());
        }
        None
    }

    /// The live connection this session holds for `key`, if any.
    fn conn_for(&self, session: &str, key: &str) -> Option<i32> {
        self.conns
            .iter()
            .find(|(_, row)| row.client.session == session && row.client.key == key)
            .map(|(id, _)| *id)
    }

    fn deliver(&mut self, from: &str, to: &Side, event: DeliveredEvent, size: usize) {
        let Some(queue) = self.outbox.get_mut(&to.session) else {
            return;
        };
        let (kind, payload_text) = match &event {
            DeliveredEvent::Connected { .. } => ("connected", None),
            DeliveredEvent::Message { text, .. } => ("message", Some(text.clone())),
            DeliveredEvent::Disconnected { .. } => ("disconnected", None),
            DeliveredEvent::Error { .. } => ("error", None),
        };
        let conn = event.conn();
        queue.push_back(event);
        if queue.len() > OUTBOX_CAP {
            let shed = queue.len() - OUTBOX_CAP;
            queue.drain(..shed);
            *self.dropped.entry(to.session.clone()).or_insert(0) += shed;
        }

        let seq = self.next_seq;
        self.next_seq += 1;
        let packet = Packet {
            seq,
            at_ms: self.started.elapsed().as_secs_f64() * 1000.0,
            frame: None,
            from: from.to_string(),
            to: to.session.clone(),
            conn,
            kind,
            size,
            payload_text,
        };
        self.log_bytes += packet.payload_text.as_ref().map_or(0, String::len);
        self.log.push_back(packet);
        while self.log.len() > PACKET_LOG_CAP {
            if let Some(dropped) = self.log.pop_front() {
                self.log_bytes -= dropped.payload_text.as_ref().map_or(0, String::len);
            }
        }
        // The byte bound sheds one packet at a time: it only bites for a
        // protocol whose messages are large, and there the newest few are what
        // a reader is looking at anyway.
        while self.log_bytes > PACKET_LOG_BYTES && self.log.len() > 1 {
            if let Some(dropped) = self.log.pop_front() {
                self.log_bytes -= dropped.payload_text.as_ref().map_or(0, String::len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listen(key: &str) -> ConnCommand {
        ConnCommand::Listen {
            key: key.to_string(),
            addr: key.to_string(),
        }
    }

    fn connect(key: &str) -> ConnCommand {
        ConnCommand::Connect {
            key: key.to_string(),
            url: key.to_string(),
        }
    }

    fn group(labels: &[&str]) -> NetCoordinator {
        let mut net = NetCoordinator::new();
        for label in labels {
            net.add_session(label);
        }
        net
    }

    /// The routing rule the whole design rests on: a client's URL matches a
    /// server's bind by AUTHORITY, and both ends are told about the connection
    /// under their OWN key — the key each runtime routes the event back by.
    #[test]
    fn a_connect_matches_a_listen_by_authority_and_tells_both_ends() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));

        assert_eq!(
            net.take_outbox("server"),
            vec![DeliveredEvent::Connected {
                key: "127.0.0.1:9101".into(),
                conn: 1
            }],
            "the server hears the event under its own LISTEN key"
        );
        assert_eq!(
            net.take_outbox("client1"),
            vec![DeliveredEvent::Connected {
                key: "ws://127.0.0.1:9101/orbs".into(),
                conn: 1
            }],
            "the client hears it under the url it dialled"
        );
    }

    /// One connection id per PAIR: a send from either end routes to the other
    /// through the same row, and the payload crosses as TEXT.
    #[test]
    fn a_send_routes_to_the_peer_and_lands_in_the_log_as_text() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        net.take_outbox("server");
        net.take_outbox("client1");

        net.perform(
            "client1",
            ConnCommand::Send {
                conn: 1,
                payload: b"{\"Move\":[1,0]}".to_vec(),
            },
        );
        assert_eq!(
            net.take_outbox("server"),
            vec![DeliveredEvent::Message {
                key: "127.0.0.1:9101".into(),
                conn: 1,
                text: "{\"Move\":[1,0]}".into()
            }]
        );
        assert!(net.take_outbox("client1").is_empty(), "no echo to the sender");

        let last = net.packets().last().expect("a routed packet");
        assert_eq!((last.from.as_str(), last.to.as_str()), ("client1", "server"));
        assert_eq!(last.kind, "message");
        assert_eq!(last.size, 14);
        assert_eq!(last.payload_text.as_deref(), Some("{\"Move\":[1,0]}"));
        assert_eq!(last.frame, None, "v1 delivery is unscheduled");
    }

    /// A send on a connection the sender is not an end of is DROPPED, never
    /// misrouted — the stale-id case a model carries across a reload.
    #[test]
    fn a_send_on_a_foreign_connection_is_dropped() {
        let mut net = group(&["server", "client1", "client2"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        net.take_outbox("server");
        net.take_outbox("client1");

        net.perform(
            "client2",
            ConnCommand::Send {
                conn: 1,
                payload: b"spoof".to_vec(),
            },
        );
        assert!(net.take_outbox("server").is_empty());
        assert!(net.take_outbox("client1").is_empty());
    }

    /// A client that dials before its authority exists WAITS, and connects the
    /// moment the listener appears — the launch race a group cannot avoid.
    /// Ids follow ARRIVAL order, so the server seats players in dial order.
    #[test]
    fn a_connect_with_no_listener_waits_and_opens_in_arrival_order() {
        let mut net = group(&["server", "client1", "client2"]);
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        net.perform("client2", connect("ws://127.0.0.1:9101/orbs"));
        assert!(net.take_outbox("client1").is_empty(), "nothing to connect to yet");

        net.perform("server", listen("127.0.0.1:9101"));
        assert_eq!(
            net.take_outbox("client1")
                .into_iter()
                .map(|e| e.conn())
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            net.take_outbox("client2")
                .into_iter()
                .map(|e| e.conn())
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    /// Re-declaring a connect (every frame's reconciliation, or a hot reload)
    /// must not open a second connection.
    #[test]
    fn connect_and_listen_are_idempotent() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        assert_eq!(net.take_outbox("client1").len(), 1);
        assert_eq!(net.connection_counts().get("client1"), Some(&1));
    }

    /// Reloading the AUTHORITY must not kill the session: both ends see the
    /// disconnect, and the client end is re-queued so the role's next `Listen`
    /// re-opens it (its runtime will never re-emit `Connect` on its own).
    #[test]
    fn reloading_the_authority_reconnects_its_clients() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        net.take_outbox("server");
        net.take_outbox("client1");

        net.reset_session("server");
        assert_eq!(
            net.take_outbox("client1"),
            vec![DeliveredEvent::Disconnected {
                key: "ws://127.0.0.1:9101/orbs".into(),
                conn: 1
            }]
        );

        // The reloaded role re-declares its listen — and the client is back,
        // on a NEW connection id, without having asked again.
        net.perform("server", listen("127.0.0.1:9101"));
        assert_eq!(
            net.take_outbox("client1"),
            vec![DeliveredEvent::Connected {
                key: "ws://127.0.0.1:9101/orbs".into(),
                conn: 2
            }]
        );
    }

    /// An undelivered batch goes back at the FRONT: a transport hiccup must not
    /// reorder a reliable channel.
    #[test]
    fn a_requeued_batch_keeps_its_place_at_the_front() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        let first = net.take_outbox("client1");
        assert_eq!(first.len(), 1);
        net.requeue("client1", first);
        net.reset_session("client1");
        // reset clears the outbox, so re-open and check ordering directly.
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        let batch = net.take_outbox("client1");
        net.requeue("client1", batch);
        net.perform(
            "server",
            ConnCommand::Send {
                conn: 2,
                payload: b"after".to_vec(),
            },
        );
        let kinds: Vec<&'static str> = net
            .take_outbox("client1")
            .iter()
            .map(|e| match e {
                DeliveredEvent::Connected { .. } => "connected",
                DeliveredEvent::Message { .. } => "message",
                DeliveredEvent::Disconnected { .. } => "disconnected",
                DeliveredEvent::Error { .. } => "error",
            })
            .collect();
        assert_eq!(kinds, vec!["connected", "message"]);
    }

    #[test]
    fn authority_ignores_scheme_and_path() {
        assert_eq!(authority_of("ws://127.0.0.1:9101/orbs"), "127.0.0.1:9101");
        assert_eq!(authority_of("127.0.0.1:9101"), "127.0.0.1:9101");
        assert_eq!(authority_of("wss://example.test/a/b"), "example.test");
    }

    /// `seq` is the cursor `wire_log`'s `since` filter reads, so it must be
    /// strictly increasing and never reused.
    #[test]
    fn sequence_numbers_are_monotonic_across_the_whole_group() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        for _ in 0..3 {
            net.perform(
                "client1",
                ConnCommand::Send {
                    conn: 1,
                    payload: b"x".to_vec(),
                },
            );
        }
        let seqs: Vec<u64> = net.packets().map(|p| p.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3, 4]);
        assert_eq!(net.routed(), 5);
    }

    /// A member that stops consuming must not grow its queue forever: past the
    /// cap the OLDEST events are shed and COUNTED, because a coordinator that
    /// advertises a perfect link may not discard traffic silently.
    #[test]
    fn an_unconsumed_outbox_is_bounded_and_reports_what_it_shed() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        net.take_outbox("server");
        // client1 never drains again.
        for index in 0..(OUTBOX_CAP + 50) {
            net.perform(
                "server",
                ConnCommand::Send {
                    conn: 1,
                    payload: format!("{index}").into_bytes(),
                },
            );
        }
        // 51: the handshake's `connected` row was still queued too.
        let drops = net.take_drops();
        assert_eq!(drops.get("client1"), Some(&51), "{drops:?}");
        assert!(net.take_drops().is_empty(), "drops are reported once");
        let queued = net.take_outbox("client1");
        assert_eq!(queued.len(), OUTBOX_CAP);
        // The NEWEST survive: the `connected` row and the first 50 sends were
        // shed, so the oldest kept is send #50.
        assert_eq!(
            queued.first(),
            Some(&DeliveredEvent::Message {
                key: "ws://127.0.0.1:9101/orbs".into(),
                conn: 1,
                text: "50".into()
            })
        );
    }

    /// A `conn` that does not fit the delivery side's i32 must be REFUSED, not
    /// truncated: `4_294_967_297` would alias live connection 1 and be
    /// delivered against it.
    #[test]
    fn an_out_of_range_conn_is_refused_rather_than_truncated() {
        let mut net = group(&["server", "client1"]);
        net.perform("server", listen("127.0.0.1:9101"));
        net.perform("client1", connect("ws://127.0.0.1:9101/orbs"));
        net.take_outbox("server");
        net.take_outbox("client1");

        net.perform(
            "client1",
            ConnCommand::Send {
                conn: (1u64 << 32) + 1,
                payload: b"alias".to_vec(),
            },
        );
        assert!(net.take_outbox("server").is_empty());
        net.perform("client1", ConnCommand::CloseConn { conn: (1u64 << 32) + 1 });
        assert_eq!(
            net.connection_counts().get("client1"),
            Some(&1),
            "the live connection survived a forged close"
        );
    }
}
