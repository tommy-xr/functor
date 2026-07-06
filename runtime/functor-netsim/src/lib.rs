//! In-process deterministic multiplayer harness (Phase 3 of `docs/multiplayer.md`).
//!
//! `NetSim` drives N game **instances** through the Phase 0 [`VirtualNet`]
//! (latency / jitter / loss / partition), stepped in lockstep. A server + N
//! clients can thus be driven and asserted deterministically — no real sockets,
//! no GL, byte-for-byte reproducible from a seed.
//!
//! An instance is one of two [`Backend`]s:
//!
//! * **`Dylib`** — an F# game dylib loaded as a *private copy*, so its
//!   `currentRunner` / net queues are its own (module statics don't collide
//!   between instances).
//! * **`Producer`** — an in-process [`GameProducer`] (e.g. an MLE game),
//!   driven through the shared runtime. All producers share this process's
//!   *process-global* net-command queue (`net::CONN_OUT`), so the harness must
//!   drain each instance's outbound commands ATOMICALLY right after running its
//!   game code (see [`NetSim::step`]) to keep them isolated per instance.
//!
//! Routing reuses the games' own `Sub.connect` / `Sub.listen`: a client's connect
//! url is matched to a server's listen bind by *authority* (host:port). The
//! `VirtualNet` connection id is used as the shared `ConnectionId` both ends see,
//! so a send from either side routes to the other.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use fable_library_rust::String_::{fromString, LrcStr};
use functor_runtime_common::net::{
    ConnCommand, ConnectionId, LinkProfile, NetEvent, NodeId, VirtualNet,
};
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::FrameTime;
use libloading::{Library, Symbol};

pub use functor_runtime_common::net::LinkProfile as Link;

pub type InstanceId = usize;

/// Whether an instance is acting as a server (another instance connected to a
/// bind it listens on) or a plain client. Derived from the live routing tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientRole {
    Server,
    Client,
}

impl ClientRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ClientRole::Server => "server",
            ClientRole::Client => "client",
        }
    }
}

/// A snapshot of one instance's network-facing state, for visualizers/telemetry.
pub struct ClientInfo {
    pub id: InstanceId,
    pub node: NodeId,
    pub role: ClientRole,
    /// Active connections this instance currently holds.
    pub connections: usize,
    /// In-flight packets addressed to this instance (inbound, not yet delivered).
    pub inbound_in_flight: usize,
}

/// The authority (host:port) of an endpoint, ignoring scheme and path, so a
/// client url ("ws://127.0.0.1:9001/echo") matches a server bind ("127.0.0.1:9001").
fn authority(endpoint: &str) -> &str {
    let after_scheme = endpoint.split("://").last().unwrap_or(endpoint);
    after_scheme.split('/').next().unwrap_or(after_scheme)
}

/// How one instance's game logic is hosted.
enum Backend {
    /// An F# game dylib, loaded as a private copy so its module statics
    /// (`currentRunner`, net queues) don't collide with other instances. The
    /// copy lives at `temp` and is removed on drop.
    Dylib { lib: Library, temp: PathBuf },
    /// An in-process [`GameProducer`] (e.g. an MLE game). Its net-command
    /// queue is PROCESS-GLOBAL and shared with every other producer instance,
    /// so the harness drains it atomically per instance — see [`NetSim::step`].
    Producer(Box<dyn GameProducer>),
}

/// One game instance driven by the sim, over either backend.
struct Instance {
    backend: Backend,
    node: NodeId,
    /// This instance's routing key (the url/bind its game used) per connection.
    keys: HashMap<ConnectionId, String>,
    /// Outbound conn commands this instance's game code has produced but not
    /// yet routed. Captured right after each game-code call (tick, event
    /// delivery) so a shared producer queue stays attributed to the right
    /// instance; routed on the NEXT frame — matching the dylib path, where an
    /// event reply sits in the private queue until the next post-tick drain.
    outbox: Vec<ConnCommand>,
}

impl Instance {
    fn load_dylib(src: &str, node: NodeId) -> Instance {
        // Each dylib instance loads a private copy so its module statics
        // (currentRunner, net queues) are independent. The temp name is unique
        // process-wide so concurrent sims / tests never clobber each other's
        // copies.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let uid = NEXT.fetch_add(1, Ordering::Relaxed);
        let suffix = std::path::Path::new(src)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("game.dylib");
        let temp = std::env::temp_dir().join(format!(
            "functor-netsim-{}-{uid}-{suffix}",
            std::process::id()
        ));
        std::fs::copy(src, &temp).unwrap_or_else(|e| panic!("copy {src}: {e}"));
        let lib = unsafe { Library::new(&temp).unwrap_or_else(|e| panic!("load {temp:?}: {e}")) };
        unsafe {
            let init: Symbol<fn()> = lib.get(b"init").unwrap();
            init();
        }
        Instance {
            backend: Backend::Dylib { lib, temp },
            node,
            keys: HashMap::new(),
            outbox: Vec::new(),
        }
    }

    fn from_producer(producer: Box<dyn GameProducer>, node: NodeId) -> Instance {
        // The producer is already constructed (e.g. `MleGame::create` has
        // evaluated `init`), mirroring the dylib path's `init()` call above.
        Instance {
            backend: Backend::Producer(producer),
            node,
            keys: HashMap::new(),
            outbox: Vec::new(),
        }
    }

    fn tick(&mut self, time: FrameTime) {
        match &mut self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn(FrameTime)> = lib.get(b"tick").unwrap();
                f(time);
            },
            Backend::Producer(p) => p.tick(time),
        }
    }

    fn drain_conn_commands(&self) -> Vec<ConnCommand> {
        let json = match &self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn() -> LrcStr> = lib.get(b"net_drain_conn_commands_json").unwrap();
                f().to_string()
            },
            Backend::Producer(p) => p.net_drain_conn_commands(),
        };
        serde_json::from_str(&json).unwrap_or_default()
    }

    fn push_connected(&mut self, key: &str, id: i32) {
        match &mut self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn(LrcStr, i32)> = lib.get(b"net_push_connected").unwrap();
                f(fromString(key.to_string()), id);
            },
            Backend::Producer(p) => p.net_push_connected(key.to_string(), id),
        }
    }

    fn push_message(&mut self, key: &str, id: i32, text: &str) {
        match &mut self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn(LrcStr, i32, LrcStr)> = lib.get(b"net_push_conn_message").unwrap();
                f(fromString(key.to_string()), id, fromString(text.to_string()));
            },
            Backend::Producer(p) => p.net_push_conn_message(key.to_string(), id, text.to_string()),
        }
    }

    fn push_disconnected(&mut self, key: &str, id: i32) {
        match &mut self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn(LrcStr, i32)> = lib.get(b"net_push_disconnected").unwrap();
                f(fromString(key.to_string()), id);
            },
            Backend::Producer(p) => p.net_push_disconnected(key.to_string(), id),
        }
    }

    fn push_error(&mut self, key: &str, id: i32, message: &str) {
        match &mut self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn(LrcStr, i32, LrcStr)> = lib.get(b"net_push_conn_error").unwrap();
                f(
                    fromString(key.to_string()),
                    id,
                    fromString(message.to_string()),
                );
            },
            Backend::Producer(p) => p.net_push_conn_error(key.to_string(), id, message.to_string()),
        }
    }

    /// The game model, `Debug`-formatted (dylib) / pretty-printed (producer),
    /// for assertions. Free-form text — consumers must not parse it (see
    /// `protocol::GameProducer::state_debug`).
    fn state(&self) -> String {
        match &self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn() -> LrcStr> = lib.get(b"emit_state_debug").unwrap();
                f().to_string()
            },
            Backend::Producer(p) => p.state_debug(),
        }
    }

    /// The instance's frame (camera + scene) at this time, for a visualizer.
    fn render(&mut self, time: FrameTime) -> functor_runtime_common::Frame {
        match &mut self.backend {
            Backend::Dylib { lib, .. } => unsafe {
                let f: Symbol<fn(FrameTime) -> functor_runtime_common::Frame> =
                    lib.get(b"test_render").unwrap();
                f(time)
            },
            Backend::Producer(p) => p.render(time),
        }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Backend::Dylib { temp, .. } = &self.backend {
            let _ = std::fs::remove_file(temp);
        }
    }
}

/// A deterministic in-process multiplayer simulation.
pub struct NetSim {
    vnet: VirtualNet,
    instances: Vec<Instance>,
    /// authority -> (server instance, its listen key).
    listeners: HashMap<String, (InstanceId, String)>,
    frame: u64,
    dt: f32,
}

impl NetSim {
    pub fn new(seed: u64) -> NetSim {
        NetSim {
            vnet: VirtualNet::new(seed),
            instances: Vec::new(),
            listeners: HashMap::new(),
            frame: 0,
            dt: 1.0 / 60.0,
        }
    }

    /// Load a game dylib as a new instance (independent copy). Returns its id.
    pub fn add(&mut self, dylib_path: &str) -> InstanceId {
        let node = self.vnet.add_node();
        let id = self.instances.len();
        self.instances.push(Instance::load_dylib(dylib_path, node));
        id
    }

    /// Add an already-constructed in-process [`GameProducer`] (e.g. an
    /// `MleGame`) as a new instance. Returns its id.
    ///
    /// Unlike a dylib, producers share this process's runtime — including the
    /// process-global net-command queue — so [`step`](Self::step) drains each
    /// one's outbound commands atomically to keep them isolated.
    pub fn add_producer(&mut self, producer: Box<dyn GameProducer>) -> InstanceId {
        let node = self.vnet.add_node();
        let id = self.instances.len();
        self.instances.push(Instance::from_producer(producer, node));
        id
    }

    /// Set the link impairment (latency/jitter/loss/reorder) between two instances.
    pub fn set_link(&mut self, a: InstanceId, b: InstanceId, profile: LinkProfile) {
        self.vnet
            .set_link(self.instances[a].node, self.instances[b].node, profile);
    }

    /// Cut traffic between two instances until [`heal`](Self::heal).
    pub fn partition(&mut self, a: InstanceId, b: InstanceId) {
        self.vnet
            .partition(self.instances[a].node, self.instances[b].node);
    }

    pub fn heal(&mut self, a: InstanceId, b: InstanceId) {
        self.vnet
            .heal(self.instances[a].node, self.instances[b].node);
    }

    /// The Debug-formatted model of an instance.
    pub fn state(&self, id: InstanceId) -> String {
        self.instances[id].state()
    }

    /// Number of instances.
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// The current simulation frame (number of [`step`](Self::step)s taken).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Total packets in flight across the whole virtual network right now.
    pub fn in_flight(&self) -> usize {
        self.vnet.in_flight_len()
    }

    /// Per-instance network-facing snapshot (role, node, live connections,
    /// inbound traffic) — for a visualizer overlay or a test assertion.
    pub fn client_info(&self, id: InstanceId) -> ClientInfo {
        let node = self.instances[id].node;
        let role = if self.listeners.values().any(|(lid, _)| *lid == id) {
            ClientRole::Server
        } else {
            ClientRole::Client
        };
        ClientInfo {
            id,
            node,
            role,
            connections: self.instances[id].keys.len(),
            inbound_in_flight: self.vnet.in_flight_to(node),
        }
    }

    /// The frame (camera + scene) an instance would render at this time — for a
    /// visualizer that draws each instance's view (see `functor-netsim-viz`).
    /// Takes `&mut self`: a producer evaluates its `draw` (and caches) here.
    pub fn render(&mut self, id: InstanceId, time: FrameTime) -> functor_runtime_common::Frame {
        self.instances[id].render(time)
    }

    /// Advance the simulation by one frame: tick every instance, route the
    /// commands they produced through the virtual network, advance it one tick,
    /// and deliver the resulting events back to each instance.
    pub fn step(&mut self) {
        let time = FrameTime {
            tts: self.frame as f32 * self.dt,
            dts: self.dt,
        };
        // Tick each instance and drain the commands IT produced ATOMICALLY —
        // so a producer sharing the process-global conn queue (MLE) stays
        // isolated: each drain reads only the instance that just ticked. The
        // dylib path has a private queue, so the sequencing is a harmless
        // no-op there.
        for inst in &mut self.instances {
            inst.tick(time.clone());
            let cmds = inst.drain_conn_commands();
            inst.outbox.extend(cmds);
        }
        self.frame += 1;

        // Collect all pending commands (this frame's ticks + last frame's
        // delivered-event replies), then register listeners before connects so
        // order within a frame doesn't matter.
        let mut commands: Vec<(InstanceId, ConnCommand)> = Vec::new();
        for (id, inst) in self.instances.iter_mut().enumerate() {
            for cmd in inst.outbox.drain(..) {
                commands.push((id, cmd));
            }
        }
        for (id, cmd) in &commands {
            if let ConnCommand::Listen { key, .. } = cmd {
                self.listeners
                    .insert(authority(key).to_string(), (*id, key.clone()));
            }
        }
        for (id, cmd) in commands {
            match cmd {
                ConnCommand::Listen { .. } => {}
                ConnCommand::Connect { key, .. } => self.open(id, key),
                ConnCommand::Send { conn, payload } => {
                    self.vnet.send(self.instances[id].node, conn, payload);
                }
                ConnCommand::CloseConn { conn } => self.vnet.disconnect(conn),
                ConnCommand::CloseKey { key } => self.close_key(id, &key),
            }
        }

        self.vnet.advance(1);
        self.deliver();
    }

    /// Advance the simulation by `n` frames.
    pub fn step_n(&mut self, n: usize) {
        for _ in 0..n {
            self.step();
        }
    }

    fn open(&mut self, client: InstanceId, client_key: String) {
        let Some((server, server_key)) = self.listeners.get(authority(&client_key)).cloned() else {
            // No matching listener: surface a connection error to the client.
            // This runs the client's game code (a producer's `update`), which
            // may queue an outbound command onto the shared queue — drain it
            // into THIS client's outbox atomically, so the deliver phase can't
            // misattribute it to another instance (matches the tick/deliver
            // drains). [xreview]
            self.instances[client].push_error(&client_key, 0, "no listener for endpoint");
            let cmds = self.instances[client].drain_conn_commands();
            self.instances[client].outbox.extend(cmds);
            return;
        };
        let client_node = self.instances[client].node;
        let server_node = self.instances[server].node;
        let conn = self.vnet.connect(client_node, server_node);
        self.instances[client].keys.insert(conn, client_key);
        self.instances[server].keys.insert(conn, server_key);
    }

    fn close_key(&mut self, id: InstanceId, key: &str) {
        let conns: Vec<ConnectionId> = self.instances[id]
            .keys
            .iter()
            .filter(|(_, k)| k.as_str() == key)
            .map(|(c, _)| *c)
            .collect();
        for conn in conns {
            self.vnet.disconnect(conn);
        }
    }

    fn deliver(&mut self) {
        for id in 0..self.instances.len() {
            let node = self.instances[id].node;
            for event in self.vnet.poll(node) {
                match event {
                    NetEvent::Connected(conn) => {
                        let key = self.key_for(id, conn);
                        self.instances[id].push_connected(&key, conn as i32);
                    }
                    NetEvent::Message(conn, bytes) => {
                        let key = self.key_for(id, conn);
                        let text = String::from_utf8_lossy(&bytes).to_string();
                        self.instances[id].push_message(&key, conn as i32, &text);
                    }
                    NetEvent::Disconnected(conn) => {
                        let key = self.key_for(id, conn);
                        self.instances[id].push_disconnected(&key, conn as i32);
                    }
                    NetEvent::Error(conn, message) => {
                        let key = self.key_for(id, conn);
                        self.instances[id].push_error(&key, conn as i32, &message);
                    }
                }
            }
            // Capture any outbound commands the delivered events produced (e.g.
            // an MLE `update` replying with `Effect.send`) BEFORE moving to the
            // next instance — again atomic, so a shared producer queue stays
            // attributed here. Held in this instance's outbox and routed next
            // frame, matching the dylib path where such a reply waits in the
            // private queue until the next post-tick drain.
            let cmds = self.instances[id].drain_conn_commands();
            self.instances[id].outbox.extend(cmds);
        }
    }

    fn key_for(&self, id: InstanceId, conn: ConnectionId) -> String {
        self.instances[id]
            .keys
            .get(&conn)
            .cloned()
            .unwrap_or_default()
    }
}
