//! In-process deterministic multiplayer harness (Phase 3 of `docs/multiplayer.md`).
//!
//! `NetSim` drives N game **instances** through the Phase 0 [`VirtualNet`]
//! (latency / jitter / loss / partition), stepped in lockstep. A server + N
//! clients can thus be driven and asserted deterministically — no real sockets,
//! no GL, byte-for-byte reproducible from a seed.
//!
//! Each instance is an in-process [`GameProducer`] (a Functor Lang game), driven through
//! the shared runtime. All producers share this process's *process-global*
//! net-command queue (`net::CONN_OUT`), so the harness must drain each instance's
//! outbound commands ATOMICALLY right after running its game code (see
//! [`NetSim::step`]) to keep them isolated per instance.
//!
//! Routing reuses the games' own `Sub.connect` / `Sub.listen`: a client's connect
//! url is matched to a server's listen bind by *authority* (host:port). The
//! `VirtualNet` connection id is used as the shared `ConnectionId` both ends see,
//! so a send from either side routes to the other.

use std::{collections::HashMap, sync::Arc};

use functor_runtime_common::asset::{preload::PreloadCommand, AssetCache};
use functor_runtime_common::net::{
    ConnCommand, ConnectionId, LinkProfile, NetEvent, NodeId, VirtualNet,
};
use functor_runtime_common::protocol::GameProducer;
use functor_runtime_common::timetravel::{History, DEFAULT_HISTORY_FRAMES};
use functor_runtime_common::{FrameTime, SceneContext};

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

/// One game instance driven by the sim — an in-process [`GameProducer`] (a Functor Lang
/// game). Its net-command queue is PROCESS-GLOBAL and shared with every other
/// producer instance, so the harness drains it atomically per instance (see
/// [`NetSim::step`]).
struct Instance {
    producer: Box<dyn GameProducer>,
    asset_cache: Arc<AssetCache>,
    scene_context: SceneContext,
    node: NodeId,
    /// This instance's routing key (the url/bind its game used) per connection.
    keys: HashMap<ConnectionId, String>,
    /// Outbound conn commands this instance's game code has produced but not
    /// yet routed. Captured right after each game-code call (tick, event
    /// delivery) so the shared producer queue stays attributed to the right
    /// instance; routed on the NEXT frame.
    outbox: Vec<ConnCommand>,
}

impl Instance {
    fn from_producer(producer: Box<dyn GameProducer>, node: NodeId) -> Instance {
        // The producer is already constructed (e.g. `FunctorLangGame::create` has
        // evaluated `init`).
        Instance {
            producer,
            asset_cache: Arc::new(AssetCache::new()),
            scene_context: SceneContext::new(),
            node,
            keys: HashMap::new(),
            outbox: Vec::new(),
        }
    }

    fn tick(&mut self, time: FrameTime) {
        self.producer
            .push_asset_progress(self.asset_cache.progress());
        self.producer.tick(time);
        // Functor's preload and terrain request queues are process-global,
        // like the connection queue. Drain immediately after this producer's
        // game code so requests cannot be claimed by the next instance.
        let commands: Vec<PreloadCommand> =
            serde_json::from_str(&self.producer.preload_drain_commands()).unwrap_or_default();
        for token in self
            .scene_context
            .drive_preloads(&self.asset_cache, commands)
        {
            self.producer.preload_push_settled(token);
        }
    }

    fn drain_conn_commands(&self) -> Vec<ConnCommand> {
        let json = self.producer.net_drain_conn_commands();
        serde_json::from_str(&json).unwrap_or_default()
    }

    fn push_connected(&mut self, key: &str, id: i32) {
        self.producer.net_push_connected(key.to_string(), id);
    }

    fn push_message(&mut self, key: &str, id: i32, text: &str) {
        self.producer
            .net_push_conn_message(key.to_string(), id, text.to_string());
    }

    fn push_disconnected(&mut self, key: &str, id: i32) {
        self.producer.net_push_disconnected(key.to_string(), id);
    }

    fn push_error(&mut self, key: &str, id: i32, message: &str) {
        self.producer
            .net_push_conn_error(key.to_string(), id, message.to_string());
    }

    /// The game model, pretty-printed, for assertions. Free-form text —
    /// consumers must not parse it (see `protocol::GameProducer::state_debug`).
    fn state(&self) -> String {
        self.producer.state_debug()
    }

    /// The instance's frame (camera + scene) at this time, for a visualizer.
    fn render(&mut self, time: FrameTime) -> functor_runtime_common::Frame {
        self.producer.render(time)
    }
}

/// The harness-owned half of a whole-environment snapshot.
///
/// Whole-environment time travel splits in two, along the line the code already
/// draws: **each producer records and restores its own model** (`SceneRecorder`
/// runs inside the frame body `tick` executes, and `seek_scene_to` restores it),
/// so the sim only has to snapshot what lives OUTSIDE the producers — the
/// network and the routing tables built from it.
///
/// This is a plain deep copy, unlike the model ring's `Rc` structural sharing:
/// what keeps it affordable is that the models are not in here at all, and what
/// remains (in-flight packets and undelivered events) is bounded by the traffic
/// actually crossing a frame. It is the part to measure first if the ring ever
/// shows up in a profile.
#[derive(Clone)]
struct SimSnapshot {
    vnet: VirtualNet,
    /// authority -> (server instance, its listen key).
    listeners: HashMap<String, (InstanceId, String)>,
    /// Per-instance `(keys, outbox)`, positionally indexed by [`InstanceId`].
    routes: Vec<(HashMap<ConnectionId, String>, Vec<ConnCommand>)>,
}

/// A deterministic in-process multiplayer simulation.
pub struct NetSim {
    vnet: VirtualNet,
    instances: Vec<Instance>,
    /// authority -> (server instance, its listen key).
    listeners: HashMap<String, (InstanceId, String)>,
    frame: u64,
    dt: f32,
    /// Per-frame ring of the harness state, the network half of a whole-
    /// environment scrub. Shares the producers' horizon so [`frame_range`] and
    /// each instance's `scene_frame_range` agree on what is still reachable.
    ///
    /// [`frame_range`]: NetSim::frame_range
    history: History<SimSnapshot>,
    /// Frame the sim is parked on, if a [`seek`](NetSim::seek) is in effect.
    scrub_pos: Option<u64>,
    /// Link impairment as CONFIGURED (as opposed to as recorded), so a restore
    /// brings back the traffic without undoing the caller's current settings.
    /// See [`reapply_link_config`](NetSim::reapply_link_config).
    links: HashMap<(InstanceId, InstanceId), LinkProfile>,
    partitioned: std::collections::HashSet<(InstanceId, InstanceId)>,
    /// The last error [`step`](NetSim::step) swallowed, for drivers that cannot
    /// see stderr. See [`take_last_error`](NetSim::take_last_error).
    last_error: Option<String>,
}

/// The source list for one instance of a multi-entry project: `entry` first —
/// that is how a producer picks its entry — then every other project file in its
/// original order.
///
/// Every role therefore loads an IDENTICAL module set and differs only in which
/// file leads, which is exactly what a multi-entry project needs: `file = module`
/// means the shared `protocol.fun` loads with both ends, and wire constructors
/// match by canonical module-qualified name, so a per-role copy of the protocol
/// would silently fail to match.
///
/// Errors naming the available files when `entry` isn't one of them — a typo'd
/// role is otherwise a confusing "no listener" much later.
pub fn entry_sources(
    entry: &str,
    project: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    if !project.iter().any(|(path, _)| path == entry) {
        let available: Vec<&str> = project.iter().map(|(p, _)| p.as_str()).collect();
        return Err(format!(
            "no project file named '{entry}' — the project has: {}",
            available.join(", ")
        ));
    }
    let mut sources = Vec::with_capacity(project.len());
    sources.extend(project.iter().filter(|(p, _)| p == entry).cloned());
    sources.extend(project.iter().filter(|(p, _)| p != entry).cloned());
    Ok(sources)
}

/// An unordered instance pair, so link config is keyed the same either way round.
fn pair(a: InstanceId, b: InstanceId) -> (InstanceId, InstanceId) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

impl NetSim {
    pub fn new(seed: u64) -> NetSim {
        NetSim {
            vnet: VirtualNet::new(seed),
            instances: Vec::new(),
            listeners: HashMap::new(),
            frame: 0,
            dt: 1.0 / 60.0,
            history: History::bounded(DEFAULT_HISTORY_FRAMES),
            scrub_pos: None,
            links: HashMap::new(),
            partitioned: std::collections::HashSet::new(),
            last_error: None,
        }
    }

    /// Take the last error a [`step`](Self::step) reported without failing —
    /// today, a branch commit that could not be honored, which leaves the sim
    /// parked. `step` cannot return a `Result` without churning every caller,
    /// and a driver that cannot see stderr (the browser) must not read a step
    /// that did nothing as a step that worked.
    pub fn take_last_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// Add an already-constructed in-process [`GameProducer`] (e.g. an
    /// `FunctorLangGame`) as a new instance. Returns its id.
    ///
    /// Producers share this process's runtime — including the process-global
    /// net-command queue — so [`step`](Self::step) drains each one's outbound
    /// commands atomically to keep them isolated.
    ///
    /// # Panics
    ///
    /// If the sim has already stepped. Every instance must join before frame 0
    /// so that sim frame `f` and each producer's rendered frame `f` name the
    /// same instant — the alignment [`seek`](Self::seek) rests on. A late joiner
    /// would start recording at ITS frame 0 while the sim was at frame `n`, and
    /// the snapshots taken before it joined describe a network that has no node
    /// for it. Late join is a real feature (a player connecting mid-session),
    /// but it needs per-instance frame offsets in the snapshot; until then this
    /// is a loud error rather than a silently skewed timeline.
    pub fn add_producer(&mut self, producer: Box<dyn GameProducer>) -> InstanceId {
        assert_eq!(
            self.frame, 0,
            "add_producer after the sim has stepped: every instance must join \
             before frame 0, or its recorded frames no longer align with the \
             sim's (see NetSim::seek)"
        );
        let node = self.vnet.add_node();
        let id = self.instances.len();
        self.instances.push(Instance::from_producer(producer, node));
        id
    }

    /// Set the link impairment (latency/jitter/loss/reorder) between two instances.
    pub fn set_link(&mut self, a: InstanceId, b: InstanceId, profile: LinkProfile) {
        self.links.insert(pair(a, b), profile);
        self.vnet
            .set_link(self.instances[a].node, self.instances[b].node, profile);
    }

    /// Cut traffic between two instances until [`heal`](Self::heal).
    pub fn partition(&mut self, a: InstanceId, b: InstanceId) {
        self.partitioned.insert(pair(a, b));
        self.vnet
            .partition(self.instances[a].node, self.instances[b].node);
    }

    pub fn heal(&mut self, a: InstanceId, b: InstanceId) {
        self.partitioned.remove(&pair(a, b));
        self.vnet
            .heal(self.instances[a].node, self.instances[b].node);
    }

    /// Re-apply the CURRENT link configuration over a restored network.
    ///
    /// Impairment is configuration, not simulation state: rewinding must bring
    /// back the packets that were in flight, but not silently undo the profile
    /// the caller has since dialed in. Keeping it live is what makes the
    /// "rewind, worsen the link, watch it again" gesture work — and when
    /// nothing was changed, re-applying is identical to restoring, so exact
    /// replay is unaffected.
    /// The configuration is authoritative over EVERY pair, not just the ones it
    /// names: a heal performed while parked has to survive the restore too, and
    /// that is only expressible by healing the pairs the config doesn't list.
    fn reapply_link_config(&mut self) {
        let links: Vec<((InstanceId, InstanceId), LinkProfile)> =
            self.links.iter().map(|(k, v)| (*k, *v)).collect();
        for ((a, b), profile) in links {
            self.vnet
                .set_link(self.instances[a].node, self.instances[b].node, profile);
        }
        let partitioned = self.partitioned.clone();
        for a in 0..self.instances.len() {
            for b in (a + 1)..self.instances.len() {
                let (na, nb) = (self.instances[a].node, self.instances[b].node);
                if partitioned.contains(&pair(a, b)) {
                    self.vnet.partition(na, nb);
                } else {
                    self.vnet.heal(na, nb);
                }
            }
        }
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
    ///
    /// This is the LIVE frame and does not move while a [`seek`](Self::seek) is
    /// parked — a viewer labelling "you are here" wants
    /// [`scrub_pos`](Self::scrub_pos) first, falling back to this.
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
        // Stepping on from a scrubbed frame commits the branch first. A failed
        // commit leaves the sim parked (nothing was mutated — `commit_branch`
        // preflights) rather than advancing a half-rewound environment.
        if let Some(frame) = self.scrub_pos {
            match self.commit_branch(frame) {
                Ok(()) => self.scrub_pos = None,
                Err(e) => {
                    // Recorded as well as printed: a driver that cannot see
                    // stderr (the browser) would otherwise read a step that did
                    // nothing as a step that succeeded.
                    let message = format!("{e} — staying parked on frame {frame}");
                    eprintln!("[netsim] {message}");
                    self.last_error = Some(message);
                    return;
                }
            }
        }
        let time = FrameTime {
            tts: self.frame as f32 * self.dt,
            dts: self.dt,
        };
        // Tick each instance and drain the commands IT produced ATOMICALLY —
        // so a producer sharing the process-global conn queue (Functor Lang) stays
        // isolated: each drain reads only the instance that just ticked.
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

        // THE SNAPSHOT CUT, and it is load-bearing. Only `deliver` touches a
        // producer's model outside `tick` (`deliver_net_event` folds an inbound
        // message through `update` on the spot), so here — after this frame's
        // sends are routed and the network has advanced, but before delivery —
        // every instance's model is still EXACTLY the one its `SceneRecorder`
        // recorded for this frame. That is what makes a seek coherent: the
        // restored models and the restored network name the same instant.
        //
        // The pending deliveries are captured inside the network snapshot, so
        // committing a branch replays them (see `commit_branch`) and the
        // original frame order is reproduced exactly.
        let settled = self.frame - 1;
        self.history.record(settled, &self.snapshot());

        self.deliver();
    }

    /// The harness state as of right now.
    fn snapshot(&self) -> SimSnapshot {
        SimSnapshot {
            vnet: self.vnet.clone(),
            listeners: self.listeners.clone(),
            routes: self
                .instances
                .iter()
                .map(|inst| (inst.keys.clone(), inst.outbox.clone()))
                .collect(),
        }
    }

    fn restore(&mut self, snapshot: SimSnapshot) {
        self.vnet = snapshot.vnet;
        self.listeners = snapshot.listeners;
        for (inst, (keys, outbox)) in self.instances.iter_mut().zip(snapshot.routes) {
            inst.keys = keys;
            inst.outbox = outbox;
        }
        self.reapply_link_config();
    }

    /// The frames a [`seek`](Self::seek) can currently reach, or `None` before
    /// the first [`step`](Self::step).
    pub fn frame_range(&self) -> Option<(u64, u64)> {
        self.history.recorded_range()
    }

    /// Verify every instance can actually restore `frame` BEFORE anything is
    /// mutated, so a seek/branch is all-or-nothing.
    ///
    /// This cannot be delegated to the producers: `SceneRecorder::seek_scene_to`
    /// and `rewind_scene_to` **clamp** to their own recorded range and return
    /// `Ok` (`timetravel.rs`), and their only hard failures are an empty history
    /// or a pruned *physics* frame — which a game without physics never reaches.
    /// So a producer whose ring has pruned past `frame` would silently restore a
    /// DIFFERENT frame and report success, quietly desynchronizing the panes
    /// from each other and from the network. Checking the ranges here turns that
    /// into a loud error.
    fn preflight(&self, frame: u64, op: &str) -> Result<(), String> {
        for (id, inst) in self.instances.iter().enumerate() {
            let range = inst.producer.scene_frame_range().ok_or_else(|| {
                format!("{op}: instance {id} has recorded no frames to restore")
            })?;
            if frame < range.0 || frame > range.1 {
                return Err(format!(
                    "{op}: instance {id} cannot restore sim frame {frame} — it \
                     retains frames {}..={} (its history was pruned, or it did \
                     not step with the sim)",
                    range.0, range.1
                ));
            }
        }
        Ok(())
    }

    /// The frame a [`seek`](Self::seek) has parked the sim on, if any.
    pub fn scrub_pos(&self) -> Option<u64> {
        self.scrub_pos
    }

    /// **Whole-environment seek.** Restore every instance's model AND the
    /// network to the state they settled in at `frame` — so a rewound frame
    /// shows each client's own lagging view, the server's authoritative one, and
    /// the packets that were genuinely mid-flight between them at that frame.
    ///
    /// Non-destructive **while parked**, in the same sense as the single-game
    /// scrubber: the recorded future is retained on both sides (the sim's ring
    /// and every producer's), so the caller can drag freely. The next
    /// [`step`](Self::step) is what commits the branch and drops that future.
    /// Because a scrub-back is a plain restore and never a re-step, **this needs
    /// no determinism at all** — the same property `History` relies on for one
    /// game holds for N games plus the network.
    ///
    /// The parked state is the frame as a viewer SAW it — the snapshot cut is
    /// taken before delivery, so the seek finishes the frame by replaying its
    /// pending deliveries. Seeking is therefore idempotent (each seek restores
    /// from the recorded frame and re-delivers), and `seek(f)` then
    /// [`state`](Self::state) equals what `state` returned live at frame `f`.
    ///
    /// `frame` is clamped into [`frame_range`](Self::frame_range). Errors if
    /// nothing is recorded yet, or if any instance refuses the seek (reported
    /// with the instance id; instances that already seeked are left where they
    /// are, since the caller's recourse is another seek).
    pub fn seek(&mut self, frame: u64) -> Result<String, String> {
        let (lo, hi) = self
            .frame_range()
            .ok_or_else(|| "seek: nothing recorded yet".to_string())?;
        let frame = frame.clamp(lo, hi);
        self.preflight(frame, "seek")?;
        for (id, inst) in self.instances.iter_mut().enumerate() {
            inst.producer
                .seek_scene_to(frame)
                .map_err(|e| format!("seek: instance {id}: {e}"))?;
        }
        let snapshot = self.history.seek(frame).clone();
        self.restore(snapshot);
        self.deliver();
        self.scrub_pos = Some(frame);
        Ok(format!("scrubbed to sim frame {frame}"))
    }

    /// Commit the branch a [`seek`](Self::seek) parked on: drop the recorded
    /// future beyond the scrubbed frame on both sides (the sim's ring and every
    /// producer's), and finish that frame by running its pending deliveries.
    ///
    /// Called automatically by [`step`](Self::step) — resuming from a scrubbed
    /// frame is what commits the branch, matching the single-game scrubber
    /// (`run.rs`, "resuming from there is what commits the branch").
    ///
    /// Rebuilt from the RECORDED frame rather than from whatever the seek left
    /// on screen: `rewind_scene_to` resets each model to its recorded frame,
    /// the network snapshot is restored again, and the frame's pending
    /// deliveries are replayed. So the branch point is the genuine end of frame
    /// `frame` no matter how the user dragged the scrubber to get there, and
    /// stepping on from an untouched scrub reproduces the original timeline
    /// exactly instead of shifting the network by a frame.
    fn commit_branch(&mut self, frame: u64) -> Result<(), String> {
        self.preflight(frame, "branch")?;
        let snapshot = self.history.seek(frame).clone();
        for (id, inst) in self.instances.iter_mut().enumerate() {
            inst.producer
                .rewind_scene_to(frame)
                .map_err(|e| format!("branch: instance {id}: {e}"))?;
        }
        self.restore(snapshot);
        self.history.truncate_from(frame + 1);
        self.frame = frame + 1;
        self.deliver();
        Ok(())
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
            // a Functor Lang `update` replying with `Effect.send`) BEFORE moving to the
            // next instance — again atomic, so the shared producer queue stays
            // attributed here. Held in this instance's outbox and routed next
            // frame.
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

#[cfg(test)]
mod tests {
    use super::entry_sources;

    fn project() -> Vec<(String, String)> {
        vec![
            ("client.fun".to_string(), "c".to_string()),
            ("protocol.fun".to_string(), "p".to_string()),
            ("server.fun".to_string(), "s".to_string()),
        ]
    }

    #[test]
    fn the_chosen_entry_leads_and_every_sibling_follows() {
        let server = entry_sources("server.fun", &project()).expect("server is a project file");
        assert_eq!(server[0].0, "server.fun", "the entry must lead the list");
        assert_eq!(server.len(), 3, "all siblings load with it (file = module)");
    }

    #[test]
    fn every_role_loads_an_identical_module_set() {
        // Only the ORDER differs — which is what lets both ends match the same
        // `protocol.fun` constructors (they are module-qualified).
        let mut client: Vec<String> = entry_sources("client.fun", &project())
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        let mut server: Vec<String> = entry_sources("server.fun", &project())
            .unwrap()
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        client.sort();
        server.sort();
        assert_eq!(client, server);
    }

    #[test]
    fn an_unknown_entry_names_what_the_project_actually_has() {
        let err = entry_sources("sever.fun", &project()).unwrap_err();
        assert!(err.contains("no project file named 'sever.fun'"), "{err}");
        assert!(
            err.contains("server.fun"),
            "the typo's intended target is listed: {err}"
        );
    }
}
