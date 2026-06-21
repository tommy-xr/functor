//! A deterministic, in-memory network for driving multiplayer games headlessly.
//!
//! `VirtualNet` models a set of nodes (each a game instance — server or client),
//! point-to-point connections between them, and per-link impairments (latency,
//! jitter, loss, reordering, partitions). Logical time advances in *ticks*; a
//! packet sent on a link becomes deliverable some number of ticks later, and the
//! caller drains delivered events per node. Given the same seed and the same
//! sequence of calls, behavior is byte-for-byte reproducible.
//!
//! This is the substance of the in-process netsim SDK (Phase 3): the connection
//! manager will talk to a `Transport` trait whose virtual implementation is built
//! on this type. Phase 0 ships and tests it standalone — no game required.

use std::collections::HashMap;

use super::{ConnectionId, LinkProfile, NetEvent, Rng};

/// Identifies one participant in the virtual network (a game instance).
pub type NodeId = u64;

/// A packet in flight: scheduled to land on `dest` at `deliver_tick`.
struct Packet {
    deliver_tick: u64,
    seq: u64,
    dest: NodeId,
    event: NetEvent,
}

struct Connection {
    a: NodeId,
    b: NodeId,
    /// Last scheduled delivery tick per direction, used to preserve FIFO order
    /// when a link's profile has `reorder = false`. Index 0 = toward `a`, 1 = toward `b`.
    last_sched: [u64; 2],
    open: bool,
}

impl Connection {
    fn peer_of(&self, node: NodeId) -> Option<NodeId> {
        if node == self.a {
            Some(self.b)
        } else if node == self.b {
            Some(self.a)
        } else {
            None
        }
    }

    /// Direction index for a packet *destined* to `node` (0 = toward `a`).
    fn dir_to(&self, node: NodeId) -> usize {
        if node == self.a {
            0
        } else {
            1
        }
    }
}

/// Deterministic in-memory network. See module docs.
pub struct VirtualNet {
    now: u64,
    rng: Rng,
    next_node: NodeId,
    next_conn: ConnectionId,
    seq: u64,
    nodes: Vec<NodeId>,
    connections: HashMap<ConnectionId, Connection>,
    /// Per-link impairment, keyed by an unordered node pair. Absent ⇒ `PERFECT`.
    links: HashMap<(NodeId, NodeId), LinkProfile>,
    /// Unordered node pairs that currently cannot exchange packets.
    partitioned: std::collections::HashSet<(NodeId, NodeId)>,
    /// Packets not yet due.
    in_flight: Vec<Packet>,
    /// Delivered events awaiting `poll`, per node.
    delivered: HashMap<NodeId, Vec<NetEvent>>,
}

fn pair(a: NodeId, b: NodeId) -> (NodeId, NodeId) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

impl VirtualNet {
    pub fn new(seed: u64) -> VirtualNet {
        VirtualNet {
            now: 0,
            rng: Rng::new(seed),
            next_node: 1,
            next_conn: 1,
            seq: 0,
            nodes: Vec::new(),
            connections: HashMap::new(),
            links: HashMap::new(),
            partitioned: std::collections::HashSet::new(),
            in_flight: Vec::new(),
            delivered: HashMap::new(),
        }
    }

    /// Current logical tick.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Number of packets currently in flight (sent but not yet delivered) —
    /// a cheap measure of live network activity, for visualizers/telemetry.
    pub fn in_flight_len(&self) -> usize {
        self.in_flight.len()
    }

    /// Number of in-flight packets whose destination is `node` (inbound traffic
    /// not yet delivered to it).
    pub fn in_flight_to(&self, node: NodeId) -> usize {
        self.in_flight.iter().filter(|p| p.dest == node).count()
    }

    /// Register a new node (game instance) and return its id.
    pub fn add_node(&mut self) -> NodeId {
        let id = self.next_node;
        self.next_node += 1;
        self.nodes.push(id);
        self.delivered.insert(id, Vec::new());
        id
    }

    /// Set the impairment profile for the link between `a` and `b` (order-independent).
    pub fn set_link(&mut self, a: NodeId, b: NodeId, profile: LinkProfile) {
        self.links.insert(pair(a, b), profile);
    }

    fn link(&self, a: NodeId, b: NodeId) -> LinkProfile {
        self.links
            .get(&pair(a, b))
            .copied()
            .unwrap_or(LinkProfile::PERFECT)
    }

    /// Cut all traffic between `a` and `b` until [`heal`](Self::heal). In-flight
    /// packets already scheduled still land; new sends are dropped.
    pub fn partition(&mut self, a: NodeId, b: NodeId) {
        self.partitioned.insert(pair(a, b));
    }

    /// Restore traffic between `a` and `b`.
    pub fn heal(&mut self, a: NodeId, b: NodeId) {
        self.partitioned.remove(&pair(a, b));
    }

    fn is_partitioned(&self, a: NodeId, b: NodeId) -> bool {
        self.partitioned.contains(&pair(a, b))
    }

    /// Open a connection from `client` to `server`. Both endpoints observe a
    /// [`NetEvent::Connected`] (queued for the next `poll`). Returns the shared id.
    pub fn connect(&mut self, client: NodeId, server: NodeId) -> ConnectionId {
        let id = self.next_conn;
        self.next_conn += 1;
        self.connections.insert(
            id,
            Connection {
                a: client,
                b: server,
                last_sched: [self.now, self.now],
                open: true,
            },
        );
        self.enqueue(client, NetEvent::Connected(id));
        self.enqueue(server, NetEvent::Connected(id));
        id
    }

    /// Send `payload` from `from` over connection `conn`. The bytes land on the
    /// peer after the link's latency/jitter, unless dropped by loss or a partition.
    /// Sends on a closed/unknown connection, or from a node not on it, are ignored.
    pub fn send(&mut self, from: NodeId, conn: ConnectionId, payload: Vec<u8>) {
        let (dest, profile, dir) = match self.connections.get(&conn) {
            Some(c) if c.open => match c.peer_of(from) {
                Some(peer) => (peer, self.link(c.a, c.b), c.dir_to(peer)),
                None => return,
            },
            _ => return,
        };

        if self.is_partitioned(from, dest) {
            return;
        }
        if profile.loss > 0.0 && self.rng.next_f32() < profile.loss {
            return;
        }

        let extra = if profile.jitter_ticks > 0 {
            self.rng.range_u32(0, profile.jitter_ticks)
        } else {
            0
        };
        let mut deliver_tick = self.now + profile.latency_ticks as u64 + extra as u64;

        if !profile.reorder {
            // Preserve send order on this direction: never deliver before the
            // previously-scheduled packet going the same way.
            let conn_mut = self.connections.get_mut(&conn).unwrap();
            let prev = conn_mut.last_sched[dir];
            if deliver_tick <= prev {
                deliver_tick = prev + 1;
            }
            conn_mut.last_sched[dir] = deliver_tick;
        }

        let seq = self.seq;
        self.seq += 1;
        self.in_flight.push(Packet {
            deliver_tick,
            seq,
            dest,
            event: NetEvent::Message(conn, payload),
        });
    }

    /// Close `conn`. Both still-reachable endpoints observe a
    /// [`NetEvent::Disconnected`] (queued for the next `poll`).
    pub fn disconnect(&mut self, conn: ConnectionId) {
        if let Some(c) = self.connections.get_mut(&conn) {
            if !c.open {
                return;
            }
            c.open = false;
            let (a, b) = (c.a, c.b);
            self.enqueue(a, NetEvent::Disconnected(conn));
            self.enqueue(b, NetEvent::Disconnected(conn));
        }
    }

    /// Advance logical time by `ticks`, delivering every packet now due. Delivery
    /// order is `(deliver_tick, seq)` so it is stable and reproducible.
    pub fn advance(&mut self, ticks: u64) {
        self.now += ticks;

        let mut due: Vec<Packet> = Vec::new();
        let mut still: Vec<Packet> = Vec::new();
        for p in self.in_flight.drain(..) {
            if p.deliver_tick <= self.now {
                due.push(p);
            } else {
                still.push(p);
            }
        }
        self.in_flight = still;

        due.sort_by(|x, y| {
            x.deliver_tick
                .cmp(&y.deliver_tick)
                .then(x.seq.cmp(&y.seq))
        });
        for p in due {
            self.enqueue(p.dest, p.event);
        }
    }

    /// Drain and return the events queued for `node` since the last poll.
    pub fn poll(&mut self, node: NodeId) -> Vec<NetEvent> {
        self.delivered
            .get_mut(&node)
            .map(std::mem::take)
            .unwrap_or_default()
    }

    fn enqueue(&mut self, node: NodeId, event: NetEvent) {
        self.delivered.entry(node).or_default().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(net: &mut VirtualNet, c: NodeId, s: NodeId, conn: ConnectionId) {
        // Both ends see Connected immediately (queued at connect time).
        assert_eq!(net.poll(c), vec![NetEvent::Connected(conn)]);
        assert_eq!(net.poll(s), vec![NetEvent::Connected(conn)]);
    }

    #[test]
    fn delivers_message_after_latency() {
        let mut net = VirtualNet::new(1);
        let (c, s) = (net.add_node(), net.add_node());
        net.set_link(c, s, LinkProfile { latency_ticks: 3, jitter_ticks: 0, loss: 0.0, reorder: false });
        let conn = net.connect(c, s);
        connected(&mut net, c, s, conn);

        net.send(c, conn, b"hi".to_vec());

        net.advance(2); // not yet due
        assert!(net.poll(s).is_empty());

        net.advance(1); // tick 3, due
        assert_eq!(net.poll(s), vec![NetEvent::Message(conn, b"hi".to_vec())]);
    }

    #[test]
    fn total_loss_drops_everything() {
        let mut net = VirtualNet::new(1);
        let (c, s) = (net.add_node(), net.add_node());
        net.set_link(c, s, LinkProfile { latency_ticks: 1, jitter_ticks: 0, loss: 1.0, reorder: true });
        let conn = net.connect(c, s);
        connected(&mut net, c, s, conn);

        for _ in 0..50 {
            net.send(c, conn, b"x".to_vec());
        }
        net.advance(100);
        assert!(net.poll(s).is_empty());
    }

    #[test]
    fn partition_blocks_then_heal_restores() {
        let mut net = VirtualNet::new(1);
        let (c, s) = (net.add_node(), net.add_node());
        let conn = net.connect(c, s);
        connected(&mut net, c, s, conn);

        net.partition(c, s);
        net.send(c, conn, b"blocked".to_vec());
        net.advance(10);
        assert!(net.poll(s).is_empty());

        net.heal(c, s);
        net.send(c, conn, b"through".to_vec());
        net.advance(10);
        assert_eq!(net.poll(s), vec![NetEvent::Message(conn, b"through".to_vec())]);
    }

    #[test]
    fn fifo_order_preserved_when_reorder_disabled() {
        // High jitter would shuffle delivery ticks, but reorder=false must keep
        // send order on the link.
        let mut net = VirtualNet::new(99);
        let (c, s) = (net.add_node(), net.add_node());
        net.set_link(c, s, LinkProfile { latency_ticks: 1, jitter_ticks: 20, loss: 0.0, reorder: false });
        let conn = net.connect(c, s);
        connected(&mut net, c, s, conn);

        for i in 0u8..10 {
            net.send(c, conn, vec![i]);
        }
        net.advance(1000);

        let got: Vec<u8> = net
            .poll(s)
            .into_iter()
            .map(|e| match e {
                NetEvent::Message(_, p) => p[0],
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(got, (0u8..10).collect::<Vec<_>>());
    }

    #[test]
    fn disconnect_notifies_both_ends_once() {
        let mut net = VirtualNet::new(1);
        let (c, s) = (net.add_node(), net.add_node());
        let conn = net.connect(c, s);
        connected(&mut net, c, s, conn);

        net.disconnect(conn);
        assert_eq!(net.poll(c), vec![NetEvent::Disconnected(conn)]);
        assert_eq!(net.poll(s), vec![NetEvent::Disconnected(conn)]);

        // Idempotent + no sends after close.
        net.disconnect(conn);
        net.send(c, conn, b"late".to_vec());
        net.advance(10);
        assert!(net.poll(c).is_empty());
        assert!(net.poll(s).is_empty());
    }

    #[test]
    fn bidirectional_traffic_routes_to_the_peer() {
        let mut net = VirtualNet::new(5);
        let (c, s) = (net.add_node(), net.add_node());
        let conn = net.connect(c, s);
        connected(&mut net, c, s, conn);

        net.send(c, conn, b"c2s".to_vec());
        net.send(s, conn, b"s2c".to_vec());
        net.advance(5);

        assert_eq!(net.poll(s), vec![NetEvent::Message(conn, b"c2s".to_vec())]);
        assert_eq!(net.poll(c), vec![NetEvent::Message(conn, b"s2c".to_vec())]);
    }

    #[test]
    fn same_seed_same_sequence_is_reproducible() {
        fn run() -> Vec<(u64, Vec<u8>)> {
            let mut net = VirtualNet::new(0xDEAD_BEEF);
            let (c, s) = (net.add_node(), net.add_node());
            net.set_link(c, s, LinkProfile { latency_ticks: 2, jitter_ticks: 8, loss: 0.25, reorder: true });
            let conn = net.connect(c, s);
            net.poll(c);
            net.poll(s);
            let mut out = Vec::new();
            for i in 0u8..40 {
                net.send(c, conn, vec![i]);
                net.advance(1);
                for e in net.poll(s) {
                    if let NetEvent::Message(_, p) = e {
                        out.push((net.now(), p));
                    }
                }
            }
            out
        }
        assert_eq!(run(), run());
    }
}
