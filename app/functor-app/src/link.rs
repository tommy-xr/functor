//! The device link's desktop side (D9, D26, D27).
//!
//! - A listener of its own on the LAN (`--device-link`, all interfaces on a
//!   free port by default), separate from the loopback UI server: it speaks
//!   only the link protocol at [`PATH`], serves no files, refuses browsers
//!   (any `Origin`), and never sees the UI's token. A peer that has not
//!   proven a pairing can only run the pairing handshake, which shows a
//!   prompt and waits for the user. Unauthenticated connections are capped
//!   in total and per address, and each address has a budget of failures
//!   and pairing attempts that refills slowly.
//! - An mDNS advert (`_functor-link._tcp`) carrying the desktop's display
//!   name, port, protocol version and random id: nothing secret.
//! - Pairings in `<home>/devices.json` (0600), with their secrets; the core
//!   sees everything but the secrets.
//! - Per paired device, a loopback port that tunnels HTTP to its debug
//!   runtime. A device only answers requests: frames from it that would
//!   ask the desktop for anything end its link.
//! - Headsets on a USB cable, seen through adb and paired over it with no
//!   code ([`cable`]).
//! - A linked headset's spoken clips on the voice channel, assembled per
//!   link ([`crate::voice`]), transcribed here and handed to the core; the
//!   progress goes back on the status channel.

mod cable;

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use functor_app_core::devices::{Attached, LinkCommand, LinkEvent, LinkKind, Paired};
use functor_app_core::Event;
use functor_device_link::desktop::{self, Desktop};
use functor_device_link::voice;
use functor_device_link::wire::{self, Frame, Hello, Mode, Reject, Reply};
use functor_device_link::{hex, shared, DesktopId, End, Error, OsEntropy, Secret, VERSION};
use functor_device_link::{HEARTBEAT_MS, MAX_FRAME, MAX_MESSAGE};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request, Response as WsResponse,
};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use crate::server::{now_ms, wall_ms};
use crate::store::{self, StoredPairing};
use crate::voice::{Assembler, Heard, Speech};

pub use functor_device_link::{PATH, SERVICE};

/// Connections not yet authenticated or pairing, at once: all, and from
/// one address (an IPv6 /64 counts as one).
const MAX_PENDING: usize = 64;
const MAX_PENDING_PER_IP: usize = 4;
/// Pairing prompts open at once: all, and from one address.
const MAX_ASKING: usize = 4;
const MAX_ASKING_PER_IP: usize = 2;
/// Tunnelled requests waiting on one device at once; more get 429 before
/// their bodies are read.
const MAX_TUNNEL_IN_FLIGHT: usize = 8;
/// A peer that does not take our messages this long is gone.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// Per address: failures and pairing attempts it may spend, refilled one
/// every `REFILL_MS`.
const BUDGET: u64 = 10;
const REFILL_MS: u64 = 6_000;
/// Requests in flight on one link.
const MAX_IN_FLIGHT: usize = 64;
/// A tunnelled request's body, below a frame's limit with room for the rest.
const MAX_BODY: usize = MAX_FRAME - 64 * 1024;
const GET_TIMEOUT: Duration = Duration::from_secs(30);
/// Pushes are answered when their load lands (the debug runtime's own wait).
const PUSH_TIMEOUT: Duration = Duration::from_secs(180);

/// Where the listener binds and whether it is advertised.
pub struct Config {
    pub bind: SocketAddr,
    pub advertise: bool,
    /// The `adb` to look for headsets on a cable with.
    pub adb: PathBuf,
}

#[derive(Clone)]
pub struct Hub(Arc<Inner>);

struct Inner {
    home: PathBuf,
    /// `<home>/device-link.lock`, held while this process owns the pairings:
    /// only the owner listens or writes them, so app processes sharing a
    /// home never undo each other's pairing or Forget.
    owner: Option<std::fs::File>,
    id: DesktopId,
    name: String,
    events: UnboundedSender<Event>,
    state: Mutex<HubState>,
    /// The advert and its full name, while registered.
    mdns: Mutex<Option<(mdns_sd::ServiceDaemon, String)>>,
    /// `adb`, once the link listens.
    adb: OnceLock<PathBuf>,
    /// Look at the cable again now.
    cable_poke: tokio::sync::Notify,
    /// One write to a headset's pairing file at a time.
    cable_writes: tokio::sync::Mutex<()>,
    /// Writes finished so far: a probe that overlapped one is stale.
    cable_written: AtomicU64,
    /// Turns headsets' clips into text.
    speech: Speech,
}

#[derive(Default)]
struct HubState {
    pairings: Vec<StoredPairing>,
    conns: HashMap<u64, Conn>,
    tunnels: HashMap<String, (String, JoinHandle<()>)>,
    pending: usize,
    pending_per_ip: HashMap<IpAddr, usize>,
    asking: usize,
    asking_per_ip: HashMap<IpAddr, usize>,
    /// Per address: (tokens left, when last refilled).
    budgets: HashMap<IpAddr, (u64, u64)>,
    next_conn: u64,
    /// What adb last showed on the cable.
    attached: Vec<Attached>,
}

struct Conn {
    cmds: UnboundedSender<ConnCmd>,
    /// Hex id, once linked.
    linked: Option<String>,
}

enum ConnCmd {
    Allow,
    Deny,
    Close,
    /// This device's pairing is gone: close if the connection is its.
    Forgotten(String),
    Request(
        wire::Request,
        oneshot::Sender<Result<wire::Response, String>>,
    ),
    /// Send this frame on the link (voice status).
    Send(Frame),
    /// Utterance `utterance` turned into text, or why not.
    Transcribed {
        utterance: u64,
        text: Result<String, String>,
        capsule: String,
    },
}

impl Hub {
    /// The hub for `home`: its desktop id (made once) and stored pairings.
    pub fn new(home: PathBuf, events: UnboundedSender<Event>) -> std::io::Result<Hub> {
        let id = store::desktop_id(&home)?;
        let pairings = store::load_pairings(&home)?;
        let name = desktop_name();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(home.join("device-link.lock"))?;
        let owner = lock.try_lock().is_ok().then_some(lock);
        Ok(Hub(Arc::new(Inner {
            owner,
            home,
            id,
            name,
            events,
            state: Mutex::new(HubState {
                pairings,
                ..HubState::default()
            }),
            mdns: Mutex::new(None),
            adb: OnceLock::new(),
            cable_poke: tokio::sync::Notify::new(),
            cable_writes: tokio::sync::Mutex::new(()),
            cable_written: AtomicU64::new(0),
            speech: Speech::from_env(),
        })))
    }

    /// What the core starts from: the stored pairings, and where the link
    /// listens (`None`: off).
    pub fn started(&self, listening: Option<String>) -> Event {
        let paired = self
            .0
            .state
            .lock()
            .unwrap()
            .pairings
            .iter()
            .map(paired)
            .collect();
        Event::Link(LinkEvent::Started { listening, paired })
    }

    fn send(&self, event: LinkEvent) {
        let _ = self.0.events.send(Event::Link(event));
    }

    /// Binds the listener and serves it (and, with `advertise`, the mDNS
    /// advert) until the runtime ends; its bound address.
    pub async fn listen(&self, config: Config) -> std::io::Result<SocketAddr> {
        if self.0.owner.is_none() {
            return Err(not_owner());
        }
        let listener = TcpListener::bind(config.bind).await?;
        let addr = listener.local_addr()?;
        if config.advertise {
            if let Err(e) = self.advertise(addr) {
                eprintln!("functor-app: cannot advertise the device link: {e}");
            }
        }
        let hub = self.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else {
                    // Out of descriptors, say: not a busy loop.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                };
                let ip = key(peer.ip());
                if hub.admit(ip) {
                    tokio::spawn(hub.clone().connection(stream, ip));
                }
            }
        });
        let _ = self.0.adb.set(config.adb);
        tokio::spawn(self.clone().watch_cable());
        // States that age (cannot reach → away) need a clock.
        let hub = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                hub.send(LinkEvent::Tick);
            }
        });
        Ok(addr)
    }

    fn advertise(&self, addr: SocketAddr) -> Result<(), mdns_sd::Error> {
        let daemon = mdns_sd::ServiceDaemon::new()?;
        // A host name of the advert's own, never the machine's: the system's
        // responder owns `<machine>.local`, and a second claim to it is a
        // conflict the OS answers by renaming the computer.
        let host = format!("functor-{}.local.", hex(&self.0.id));
        let props: HashMap<String, String> = [
            ("v".to_string(), VERSION.to_string()),
            ("id".to_string(), hex(&self.0.id)),
        ]
        .into();
        let ip = match addr.ip() {
            ip if ip.is_unspecified() => String::new(),
            ip => ip.to_string(),
        };
        let mut info = mdns_sd::ServiceInfo::new(
            SERVICE,
            &self.0.name,
            &host,
            ip.as_str(),
            addr.port(),
            props,
        )?;
        if addr.ip().is_unspecified() {
            info = info.enable_addr_auto();
        }
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        *self.0.mdns.lock().unwrap() = Some((daemon, fullname));
        Ok(())
    }

    /// Withdraws the advert (waiting briefly for the goodbye) and closes
    /// every link.
    pub fn shutdown(&self) {
        if let Some((daemon, fullname)) = self.0.mdns.lock().unwrap().take() {
            if let Ok(done) = daemon.unregister(&fullname) {
                let _ = done.recv_timeout(Duration::from_secs(1));
            }
            let _ = daemon.shutdown();
        }
        for conn in self.0.state.lock().unwrap().conns.values() {
            let _ = conn.cmds.send(ConnCmd::Close);
        }
    }

    /// Whether a new connection from `ip` may start: within the caps, and
    /// the address has budget left.
    fn admit(&self, ip: IpAddr) -> bool {
        let mut s = self.0.state.lock().unwrap();
        let now = now_ms();
        // Forget addresses back at a full budget, so spoofed ones cannot grow it.
        if s.budgets.len() > 4096 {
            s.budgets.retain(|_, (left, _)| *left < BUDGET);
        }
        if tokens(&mut s.budgets, ip, now) == 0 {
            return false;
        }
        let from_ip = s.pending_per_ip.get(&ip).copied().unwrap_or(0);
        if s.pending >= MAX_PENDING || from_ip >= MAX_PENDING_PER_IP {
            return false;
        }
        s.pending += 1;
        *s.pending_per_ip.entry(ip).or_default() += 1;
        true
    }

    /// One token of `ip`'s budget spent: a failure or a pairing attempt.
    fn spend(&self, ip: IpAddr) {
        let mut s = self.0.state.lock().unwrap();
        let now = now_ms();
        tokens(&mut s.budgets, ip, now);
        if let Some((left, _)) = s.budgets.get_mut(&ip) {
            *left = left.saturating_sub(1);
        }
    }

    fn release_pending(&self, ip: IpAddr) {
        let mut s = self.0.state.lock().unwrap();
        s.pending = s.pending.saturating_sub(1);
        release(&mut s.pending_per_ip, ip);
    }

    /// A pairing prompt for `ip`, if there is room for one.
    fn reserve_prompt(&self, ip: IpAddr) -> bool {
        let mut s = self.0.state.lock().unwrap();
        let from_ip = s.asking_per_ip.get(&ip).copied().unwrap_or(0);
        if s.asking >= MAX_ASKING || from_ip >= MAX_ASKING_PER_IP {
            return false;
        }
        s.asking += 1;
        *s.asking_per_ip.entry(ip).or_default() += 1;
        true
    }

    fn release_prompt(&self, ip: IpAddr) {
        let mut s = self.0.state.lock().unwrap();
        s.asking = s.asking.saturating_sub(1);
        release(&mut s.asking_per_ip, ip);
    }

    fn secret(&self, device: &str) -> Option<Secret> {
        let s = self.0.state.lock().unwrap();
        let p = s.pairings.iter().find(|p| p.id == device)?;
        Secret::from_hex(&p.secret)
    }

    /// Carries the commands the core sends to the link.
    pub fn command(&self, command: LinkCommand) {
        let to = |conn: u64, cmd: ConnCmd| {
            if let Some(c) = self.0.state.lock().unwrap().conns.get(&conn) {
                let _ = c.cmds.send(cmd);
            }
        };
        match command {
            LinkCommand::Allow { request } => to(request, ConnCmd::Allow),
            LinkCommand::Deny { request } => to(request, ConnCmd::Deny),
            LinkCommand::Forget { device } => self.forget(&device),
            LinkCommand::OpenTunnel { device } => {
                let hub = self.clone();
                tokio::spawn(async move { hub.open_tunnel(device).await });
            }
            LinkCommand::PairByCable { serial, replace } => {
                tokio::spawn(self.clone().pair_by_cable(serial, replace));
            }
            LinkCommand::Voice {
                device,
                utterance,
                stage,
            } => {
                use functor_app_core::voice::Stage as Core;
                let stage = match stage {
                    Core::Working => voice::Stage::Working,
                    Core::Reloaded => voice::Stage::Reloaded,
                    Core::Failed(reason) => voice::Stage::Failed { reason },
                };
                let status = voice::Status {
                    utterance: Some(utterance),
                    stage,
                };
                if let Some(link) = self.link_of(&device) {
                    let _ = link.send(ConnCmd::Send(status.frame()));
                }
            }
        }
    }

    /// Changes the stored pairings: read afresh (another app process may
    /// share the home), changed, written, and only then this one's copy.
    fn update_pairings(
        &self,
        s: &mut HubState,
        change: impl FnOnce(&mut Vec<StoredPairing>),
    ) -> std::io::Result<()> {
        if self.0.owner.is_none() {
            return Err(not_owner());
        }
        let mut pairings = store::load_pairings(&self.0.home)?;
        change(&mut pairings);
        store::save_pairings(&self.0.home, &pairings)?;
        s.pairings = pairings;
        Ok(())
    }

    /// Forget: [`Hub::drop_pairing`], then the same pairing off the
    /// headset if it is on a cable. If the pairing cannot be deleted here,
    /// nothing changes and the core is told.
    fn forget(&self, device: &str) {
        match self.drop_pairing(device) {
            Ok(Some(p)) => self.forget_on_cable(device, &p.name, p.secret),
            Ok(None) => {}
            Err(e) => {
                let s = self.0.state.lock().unwrap();
                let p = s.pairings.iter().find(|p| p.id == device);
                let name = p.map_or("the headset", |p| p.name.as_str());
                let reason = format!("cannot forget {name}: {e}");
                self.send(LinkEvent::Failed { reason });
            }
        }
    }

    /// Deletes `device`'s pairing (its secret with it), then closes its
    /// links, half-open ones too, and its tunnel; the pairing deleted.
    fn drop_pairing(&self, device: &str) -> std::io::Result<Option<StoredPairing>> {
        let mut s = self.0.state.lock().unwrap();
        let mut gone = None;
        self.update_pairings(&mut s, |all| {
            gone = all
                .iter()
                .position(|p| p.id == device)
                .map(|i| all.remove(i));
        })?;
        self.send(LinkEvent::Forgotten {
            device: device.to_string(),
        });
        for conn in s.conns.values() {
            let _ = conn.cmds.send(ConnCmd::Forgotten(device.to_string()));
        }
        if let Some((_, task)) = s.tunnels.remove(device) {
            task.abort();
        }
        Ok(gone)
    }

    /// Stores `secret` for device `id` (hex), replacing any pairing it had;
    /// the pairing as the core sees it. An error leaves nothing stored.
    fn store_pairing(&self, id: String, name: &str, secret: &Secret) -> std::io::Result<Paired> {
        let now = wall_ms();
        let mut s = self.0.state.lock().unwrap();
        let mut shown = None;
        self.update_pairings(&mut s, |all| {
            let first_seen = all
                .iter()
                .find(|p| p.id == id)
                .map_or(now, |p| p.first_seen);
            all.retain(|p| p.id != id);
            let pairing = StoredPairing {
                id,
                name: name.to_string(),
                secret: secret.to_hex(),
                first_seen,
                last_seen: now,
            };
            shown = Some(paired(&pairing));
            all.push(pairing);
        })?;
        Ok(shown.expect("just stored"))
    }

    /// Notes that `device` was seen now, kept for its card after a restart.
    fn seen(&self, device: &str) {
        let mut s = self.0.state.lock().unwrap();
        let now = wall_ms();
        let saved = self.update_pairings(&mut s, |all| {
            if let Some(p) = all.iter_mut().find(|p| p.id == device) {
                p.last_seen = now;
            }
        });
        if let Err(e) = saved {
            eprintln!("functor-app: cannot save the pairings: {e}");
        }
    }

    /// Marks `conn` as `device`'s link if the secret it proved is still the
    /// stored one (not forgotten or replaced meanwhile), closing any older
    /// link of the device; whether it was.
    fn promote(&self, conn: u64, device: &str, proved: &Option<Secret>) -> bool {
        let mut s = self.0.state.lock().unwrap();
        let stored = s.pairings.iter().find(|p| p.id == device);
        let current = stored.and_then(|p| Secret::from_hex(&p.secret));
        if current.is_none() || current != *proved {
            return false;
        }
        for (id, c) in s.conns.iter() {
            if *id != conn && c.linked.as_deref() == Some(device) {
                let _ = c.cmds.send(ConnCmd::Close);
            }
        }
        if let Some(c) = s.conns.get_mut(&conn) {
            c.linked = Some(device.to_string());
        }
        true
    }

    /// The live link of `device`.
    fn link_of(&self, device: &str) -> Option<UnboundedSender<ConnCmd>> {
        let s = self.0.state.lock().unwrap();
        s.conns
            .values()
            .find(|c| c.linked.as_deref() == Some(device))
            .map(|c| c.cmds.clone())
    }

    /// Serves one connection until it ends, reporting to the core.
    async fn connection(self, stream: TcpStream, ip: IpAddr) {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE))
            .max_frame_size(Some(MAX_MESSAGE));
        let accept =
            tokio_tungstenite::accept_hdr_async_with_config(stream, check_upgrade, Some(config));
        let ws = match tokio::time::timeout(Duration::from_secs(10), accept).await {
            Ok(Ok(ws)) => ws,
            _ => {
                self.spend(ip);
                self.release_pending(ip);
                return;
            }
        };
        let (cmds, rx) = mpsc::unbounded_channel();
        let me = cmds.clone();
        let id = {
            let mut s = self.0.state.lock().unwrap();
            s.next_conn += 1;
            let id = s.next_conn;
            s.conns.insert(id, Conn { cmds, linked: None });
            id
        };
        let mut link = Connection {
            hub: self.clone(),
            id,
            ip,
            machine: Desktop::new(self.0.id, self.0.name.clone(), shared(OsEntropy), now_ms()),
            pending: true,
            asking: false,
            paired: false,
            proved: None,
            linked: None,
            waiting: HashMap::new(),
            next_request: 0,
            me,
            voice: Assembler::default(),
            outgoing: Vec::new(),
        };
        let end = link.run(ws, rx).await;
        link.finish(end);
        self.0.state.lock().unwrap().conns.remove(&id);
    }

    async fn open_tunnel(self, device: String) {
        let known = {
            let s = self.0.state.lock().unwrap();
            if let Some((url, _)) = s.tunnels.get(&device) {
                Some(Some(url.clone()))
            } else if !s.pairings.iter().any(|p| p.id == device) {
                Some(None)
            } else {
                None
            }
        };
        match known {
            Some(Some(url)) => return self.send(LinkEvent::TunnelOpened { device, url }),
            Some(None) => return,
            None => {}
        }
        let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await {
            Ok(l) => l,
            Err(e) => return eprintln!("functor-app: cannot open a tunnel port: {e}"),
        };
        let Ok(addr) = listener.local_addr() else {
            return;
        };
        let url = format!("http://127.0.0.1:{}", addr.port());
        let tunnel_state = Arc::new(Tunnel {
            hub: self.clone(),
            device: device.clone(),
            port: addr.port(),
            slots: tokio::sync::Semaphore::new(MAX_TUNNEL_IN_FLIGHT),
        });
        let app = Router::new().fallback(tunnel).with_state(tunnel_state);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let url = {
            let mut s = self.0.state.lock().unwrap();
            let raced = s.tunnels.get(&device).map(|(url, _)| url.clone());
            // Forgotten meanwhile, or another open won the race.
            if raced.is_some() || !s.pairings.iter().any(|p| p.id == device) {
                task.abort();
                match raced {
                    Some(url) => url,
                    None => return,
                }
            } else {
                s.tunnels.insert(device.clone(), (url.clone(), task));
                url
            }
        };
        self.send(LinkEvent::TunnelOpened { device, url });
    }
}

/// The handshake's HTTP check: our path, and no browser.
#[allow(clippy::result_large_err)]
fn check_upgrade(req: &Request, res: WsResponse) -> Result<WsResponse, ErrorResponse> {
    let refuse = |status: StatusCode| {
        let mut r = ErrorResponse::new(None);
        *r.status_mut() = status;
        Err(r)
    };
    if req.uri().path() != PATH {
        return refuse(StatusCode::NOT_FOUND);
    }
    if req.headers().contains_key(header::ORIGIN) {
        return refuse(StatusCode::FORBIDDEN);
    }
    Ok(res)
}

fn not_owner() -> std::io::Error {
    std::io::Error::other("another Functor app using this app home has the headsets")
}

/// The address limits count by: itself, or for IPv6 its /64 (one host can
/// take many addresses there).
fn key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6((u128::from(v6) & !((1u128 << 64) - 1)).into()),
        },
        v4 => v4,
    }
}

fn release(counts: &mut HashMap<IpAddr, usize>, ip: IpAddr) {
    if let Some(n) = counts.get_mut(&ip) {
        *n -= 1;
        if *n == 0 {
            counts.remove(&ip);
        }
    }
}

/// `ip`'s tokens after refilling to `now`.
fn tokens(budgets: &mut HashMap<IpAddr, (u64, u64)>, ip: IpAddr, now: u64) -> u64 {
    let (left, at) = budgets.entry(ip).or_insert((BUDGET, now));
    let refill = now.saturating_sub(*at) / REFILL_MS;
    if refill > 0 {
        *left = (*left + refill).min(BUDGET);
        *at += refill * REFILL_MS;
    }
    if *left == BUDGET {
        *at = now;
    }
    *left
}

fn paired(p: &StoredPairing) -> Paired {
    Paired {
        id: p.id.clone(),
        name: p.name.clone(),
        first_seen: p.first_seen,
        last_seen: p.last_seen,
    }
}

/// This machine's name for people, as the headset lists it.
fn desktop_name() -> String {
    let name = sysinfo::System::host_name().unwrap_or_default();
    let name = name.trim_end_matches(".local").trim();
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(wire::MAX_NAME)
        .collect();
    match name.is_empty() {
        true => "Functor desktop".to_string(),
        false => name,
    }
}

/// One connection's loop: the protocol machine between the socket and the
/// core.
struct Connection {
    hub: Hub,
    id: u64,
    ip: IpAddr,
    machine: Desktop,
    /// Still counted as unauthenticated.
    pending: bool,
    /// Counted as an open pairing prompt.
    asking: bool,
    paired: bool,
    /// The secret its hello was checked against (linking).
    proved: Option<Secret>,
    /// The device's hex id, once linked.
    linked: Option<String>,
    waiting: HashMap<u32, oneshot::Sender<Result<wire::Response, String>>>,
    next_request: u32,
    /// Its own command channel, for work it hands off (transcription).
    me: UnboundedSender<ConnCmd>,
    /// The clip being heard on the voice channel.
    voice: Assembler,
    /// Frames to send after this step's events (voice status).
    outgoing: Vec<Frame>,
}

type Ws = tokio_tungstenite::WebSocketStream<TcpStream>;

/// How a connection ended, for the core: `Ok` clean, `Err` why it failed.
type Ending = Result<(), Error>;

impl Connection {
    async fn run(&mut self, mut ws: Ws, mut cmds: UnboundedReceiver<ConnCmd>) -> Ending {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        let mut ping = tokio::time::interval(Duration::from_millis(HEARTBEAT_MS));
        let mut first = true;
        loop {
            let out = tokio::select! {
                msg = ws.next() => match msg {
                    Some(Ok(Message::Binary(bytes))) => {
                        if std::mem::take(&mut first) {
                            if let Some(refused) = self.refuse_pairing(&bytes) {
                                let _ = put(&mut ws, refused).await;
                                return Err(Error::Busy);
                            }
                        }
                        let hub = &self.hub;
                        let proved = &mut self.proved;
                        self.machine.receive(&bytes, now_ms(), |d| {
                            *proved = hub.secret(&hex(d));
                            proved.clone()
                        })
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(_)) => return Err(Error::Unexpected("a text message")),
                    // The socket just went (the app quit, the headset slept):
                    // gone, not failing.
                    Some(Err(WsError::Io(_) | WsError::ConnectionClosed | WsError::Protocol(
                        ProtocolError::ResetWithoutClosingHandshake,
                    ))) => return Ok(()),
                    // Over our message limit: said plainly.
                    Some(Err(WsError::Capacity(_))) => return Err(Error::TooLarge),
                    Some(Err(e)) => return Err(Error::Closed(e.to_string())),
                },
                Some(cmd) = cmds.recv() => match cmd {
                    ConnCmd::Allow => self.machine.allow(now_ms()),
                    ConnCmd::Deny => self.machine.deny(),
                    ConnCmd::Forgotten(device) if self.device() != Some(device.as_str()) => continue,
                    ConnCmd::Close | ConnCmd::Forgotten(_) => {
                        if let Ok(msgs) = self.machine.send(&Frame::Close("closed by the desktop".into())) {
                            for m in msgs {
                                let _ = put(&mut ws, m).await;
                            }
                        }
                        let _ = tokio::time::timeout(WRITE_TIMEOUT, ws.close(None)).await;
                        return Ok(());
                    }
                    ConnCmd::Send(frame) => {
                        self.outgoing.push(frame);
                        desktop_output(Vec::new())
                    }
                    ConnCmd::Transcribed { utterance, text, capsule } => {
                        self.transcribed(utterance, text, &capsule);
                        desktop_output(Vec::new())
                    }
                    ConnCmd::Request(mut request, reply) => {
                        // Requests whose callers gave up free their slots.
                        self.waiting.retain(|_, r| !r.is_closed());
                        if self.waiting.len() >= MAX_IN_FLIGHT {
                            let _ = reply.send(Err("too many requests in flight".into()));
                            continue;
                        }
                        self.next_request = self.next_request.wrapping_add(1);
                        request.id = self.next_request;
                        match self.machine.request(request) {
                            Ok(msgs) => {
                                self.waiting.insert(self.next_request, reply);
                                desktop_output(msgs)
                            }
                            Err(e) => {
                                let _ = reply.send(Err(e.to_string()));
                                continue;
                            }
                        }
                    }
                },
                _ = tick.tick() => {
                    if let Some(e) = self.machine.check(now_ms()) {
                        return Err(e);
                    }
                    if let Some(heard) = self.voice.check(now_ms()) {
                        self.heard(heard);
                    }
                    desktop_output(Vec::new())
                }
                _ = ping.tick(), if self.machine.linked() => {
                    match self.machine.send(&Frame::Ping(now_ms())) {
                        Ok(msgs) => desktop_output(msgs),
                        Err(e) => return Err(e),
                    }
                }
            };
            // Events first: a pairing is stored before its secret is sent.
            for event in out.events {
                self.event(event)?;
            }
            for msg in out.send {
                put(&mut ws, msg).await?;
            }
            for frame in std::mem::take(&mut self.outgoing) {
                // A link not up yet (or gone) has no one to tell.
                for msg in self.machine.send(&frame).unwrap_or_default() {
                    put(&mut ws, msg).await?;
                }
            }
            match out.end {
                None => {}
                Some(End::Done) => {
                    let _ = tokio::time::timeout(WRITE_TIMEOUT, ws.close(None)).await;
                    return Ok(());
                }
                Some(End::Failed(e)) => return Err(e),
            }
        }
    }

    /// The device's hex id, once linked.
    fn device(&self) -> Option<&str> {
        self.linked.as_deref()
    }

    /// A pairing hello takes a prompt slot (it is then no longer pending),
    /// or gets the refusal to send.
    fn refuse_pairing(&mut self, first: &[u8]) -> Option<Vec<u8>> {
        let Ok((hello, _)) = Hello::decode(first) else {
            return None;
        };
        if hello.mode != Mode::Pair {
            return None;
        }
        // Every pairing attempt costs, so prompts cannot be spammed.
        self.hub.spend(self.ip);
        if !self.hub.reserve_prompt(self.ip) {
            return Some(Reply::Reject(Reject::Busy).encode());
        }
        self.asking = true;
        self.unpend();
        None
    }

    /// Acts on one event; an error ends the connection.
    fn event(&mut self, event: desktop::Event) -> Result<(), Error> {
        let hub = self.hub.clone();
        match event {
            desktop::Event::PairRequest {
                hello,
                code,
                expires_at,
            } => {
                let expires_at = wall_ms() + expires_at.saturating_sub(now_ms());
                hub.send(LinkEvent::PairRequest {
                    request: self.id,
                    device: hex(&hello.device),
                    name: hello.name,
                    app_version: hello.app_version,
                    code,
                    expires_at,
                });
            }
            desktop::Event::Paired { hello, secret } => {
                let device = match hub.store_pairing(hex(&hello.device), &hello.name, &secret) {
                    Ok(device) => device,
                    Err(e) => {
                        let reason = format!("cannot store the pairing with {}: {e}", hello.name);
                        hub.send(LinkEvent::Failed { reason });
                        return Err(Error::Closed("the pairing could not be stored".into()));
                    }
                };
                self.paired = true;
                hub.send(LinkEvent::Paired {
                    request: self.id,
                    device,
                });
            }
            desktop::Event::Linked { hello } => {
                let device = hex(&hello.device);
                // The newest link of a device wins, if its secret still stands.
                if !hub.promote(self.id, &device, &self.proved) {
                    return Err(Error::NotPaired);
                }
                self.unpend();
                hub.seen(&device);
                self.linked = Some(device.clone());
                hub.send(LinkEvent::Connected {
                    conn: self.id,
                    device,
                    name: hello.name,
                    app_version: hello.app_version,
                    link: LinkKind::Wifi,
                });
            }
            desktop::Event::Response(r) => {
                if let Some(reply) = self.waiting.remove(&r.id) {
                    let _ = reply.send(Ok(r));
                }
            }
            desktop::Event::Status(s) => hub.send(LinkEvent::Status {
                conn: self.id,
                battery: s.battery,
                charging: s.charging,
                build: s.build,
                project: s.project,
            }),
            desktop::Event::Latency(ms) => hub.send(LinkEvent::Latency { conn: self.id, ms }),
            // Only a linked device's frames reach here as events.
            desktop::Event::Channel {
                channel: voice::CLIP_CHANNEL,
                data,
            } => {
                if let Some(heard) = self.voice.receive(&data, now_ms()) {
                    self.heard(heard);
                }
            }
            desktop::Event::Channel { .. } => {}
            desktop::Event::Mismatch(pre) => hub.send(LinkEvent::Mismatch {
                device: hex(&pre.device),
                name: pre.name,
                theirs: pre.version,
                ours: VERSION,
            }),
        }
        Ok(())
    }

    /// Acts on what the voice channel heard: a status to send, or a whole
    /// clip to transcribe (off this loop; the text comes back as
    /// `ConnCmd::Transcribed`).
    fn heard(&mut self, heard: Heard) {
        let (utterance, pcm, capsule) = match heard {
            Heard::Status(status) => return self.outgoing.push(status.frame()),
            Heard::Clip {
                utterance,
                pcm,
                capsule,
            } => (utterance, pcm, capsule),
        };
        self.say(utterance, voice::Stage::Heard);
        let speech = self.hub.0.speech.clone();
        let me = self.me.clone();
        let name = format!("{}-{utterance}", self.linked.as_deref().unwrap_or("device"));
        let home = self.hub.0.home.clone();
        tokio::spawn(async move {
            crate::voice::keep_audio(&home, &name, &pcm);
            let text = speech.transcribe(pcm).await;
            let _ = me.send(ConnCmd::Transcribed {
                utterance,
                text,
                capsule,
            });
        });
    }

    /// Utterance `utterance`'s text: shown to the headset and handed to the core.
    fn transcribed(&mut self, utterance: u64, text: Result<String, String>, capsule: &str) {
        self.voice.transcribed(utterance);
        match text {
            Ok(text) if !text.is_empty() => {
                let stage = voice::Stage::Transcribed { text: text.clone() };
                self.say(utterance, stage);
                self.hub.send(LinkEvent::Utterance {
                    conn: self.id,
                    utterance,
                    text,
                    context: crate::voice::render(capsule),
                });
            }
            Ok(_) => self.say(
                utterance,
                voice::Stage::Failed {
                    reason: "heard no words".to_string(),
                },
            ),
            Err(reason) => self.say(utterance, voice::Stage::Failed { reason }),
        }
    }

    fn say(&mut self, utterance: u64, stage: voice::Stage) {
        let status = voice::Status {
            utterance: Some(utterance),
            stage,
        };
        self.outgoing.push(status.frame());
    }

    fn unpend(&mut self) {
        if std::mem::take(&mut self.pending) {
            self.hub.release_pending(self.ip);
        }
    }

    fn finish(&mut self, end: Ending) {
        let hub = self.hub.clone();
        for (_, reply) in self.waiting.drain() {
            let _ = reply.send(Err("the headset disconnected".into()));
        }
        if self.asking {
            hub.release_prompt(self.ip);
            if !self.paired {
                hub.send(LinkEvent::PairEnded { request: self.id });
            }
        }
        // An unauthenticated peer's failure spends its address's budget;
        // the outcomes of a real pairing do not.
        let benign = matches!(
            end,
            Ok(()) | Err(Error::Denied | Error::Cancelled | Error::Expired | Error::Busy)
        );
        if self.linked.is_none() && !benign {
            hub.spend(self.ip);
        }
        self.unpend();
        if let Some(device) = self.linked.take() {
            hub.seen(&device);
            hub.send(LinkEvent::Disconnected {
                conn: self.id,
                lost: end.err().map(|e| e.to_string()),
            });
        }
    }
}

/// Sends `msg`, giving up on a peer that will not take it.
async fn put(ws: &mut Ws, msg: Vec<u8>) -> Result<(), Error> {
    match tokio::time::timeout(WRITE_TIMEOUT, ws.send(Message::Binary(msg.into()))).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => Err(Error::Closed("the connection dropped".into())),
        Err(_) => Err(Error::Timeout),
    }
}

fn desktop_output(send: Vec<Vec<u8>>) -> functor_device_link::Output<desktop::Event> {
    functor_device_link::Output {
        send,
        ..Default::default()
    }
}

/// Request headers the debug runtime reads.
const REQUEST_HEADERS: [HeaderName; 3] = [header::CONTENT_TYPE, header::ACCEPT, header::ORIGIN];
/// Response headers passed back.
const RESPONSE_HEADERS: [HeaderName; 4] = [
    header::CONTENT_TYPE,
    header::CACHE_CONTROL,
    header::ACCESS_CONTROL_ALLOW_ORIGIN,
    header::VARY,
];

/// One device's tunnel port.
struct Tunnel {
    hub: Hub,
    device: String,
    port: u16,
    /// Requests waiting on the device (`MAX_TUNNEL_IN_FLIGHT`).
    slots: tokio::sync::Semaphore,
}

/// One HTTP request on a device's tunnel port, forwarded over its link.
async fn tunnel(State(t): State<Arc<Tunnel>>, request: axum::extract::Request) -> Response {
    let refuse = |status: StatusCode, why: &'static str| (status, why).into_response();
    let (parts, body) = request.into_parts();
    // Only this port's own name: a rebound DNS name (a web page's) is not.
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    let own = [
        format!("127.0.0.1:{}", t.port),
        format!("localhost:{}", t.port),
    ];
    if !host.is_some_and(|h| own.iter().any(|o| o.eq_ignore_ascii_case(h))) {
        return refuse(StatusCode::FORBIDDEN, "host is not allowed");
    }
    // As the debug runtime itself: no page from another origin.
    if let Some(origin) = parts.headers.get(header::ORIGIN) {
        if !origin.to_str().is_ok_and(loopback_origin) {
            return refuse(StatusCode::FORBIDDEN, "origin is not allowed");
        }
    }
    if parts.uri.path() == "/stream" {
        return refuse(StatusCode::NOT_IMPLEMENTED, "streams are not tunnelled yet");
    }
    let target = parts.uri.path_and_query().map_or("/", |p| p.as_str());
    if target.len() > wire::MAX_TARGET {
        return refuse(StatusCode::URI_TOO_LONG, "the path is too long");
    }
    if parts.method.as_str().len() > wire::MAX_METHOD {
        return refuse(StatusCode::METHOD_NOT_ALLOWED, "unknown method");
    }
    // A slot before the body is read, so waiting requests stay bounded.
    let Ok(_slot) = t.slots.try_acquire() else {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "too many requests at once");
    };
    let Ok(body) = axum::body::to_bytes(body, MAX_BODY).await else {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "the body is too large");
    };
    let Some(link) = t.hub.link_of(&t.device) else {
        return refuse(
            StatusCode::SERVICE_UNAVAILABLE,
            "the headset is not connected",
        );
    };
    let request = wire::Request {
        id: 0,
        method: parts.method.as_str().to_string(),
        target: target.to_string(),
        headers: REQUEST_HEADERS
            .iter()
            .filter_map(|h| {
                Some((
                    h.as_str().to_string(),
                    parts.headers.get(h)?.to_str().ok()?.to_string(),
                ))
            })
            .collect(),
        body: body.to_vec(),
    };
    let (reply, answer) = oneshot::channel();
    if link.send(ConnCmd::Request(request, reply)).is_err() {
        return refuse(StatusCode::BAD_GATEWAY, "the headset disconnected");
    }
    let wait = match parts.method {
        Method::GET | Method::HEAD => GET_TIMEOUT,
        _ => PUSH_TIMEOUT,
    };
    match tokio::time::timeout(wait, answer).await {
        Ok(Ok(Ok(res))) => {
            let status = StatusCode::from_u16(res.status).unwrap_or(StatusCode::BAD_GATEWAY);
            let mut out = (status, res.body).into_response();
            for (name, value) in res.headers {
                let Ok(name) = HeaderName::try_from(name.as_str()) else {
                    continue;
                };
                if RESPONSE_HEADERS.contains(&name) {
                    if let Ok(value) = HeaderValue::from_str(&value) {
                        out.headers_mut().insert(name, value);
                    }
                }
            }
            out
        }
        Ok(Ok(Err(e))) => (StatusCode::BAD_GATEWAY, e).into_response(),
        Ok(Err(_)) => refuse(StatusCode::BAD_GATEWAY, "the headset disconnected"),
        Err(_) => refuse(StatusCode::GATEWAY_TIMEOUT, "the headset did not answer"),
    }
}

/// The debug runtime's own rule (it cannot be shared: the app does not
/// depend on the runtime): `http(s)://` and `127.0.0.1`, `localhost` or
/// `[::1]`, an optional port, nothing else.
fn loopback_origin(origin: &str) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    if authority.is_empty()
        || authority.contains(['/', '?', '#', '@'])
        || authority.chars().any(char::is_whitespace)
    {
        return false;
    }
    let (host, port) = match authority.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some(("::1", "")) => ("[::1]", None),
            Some(("::1", suffix)) => match suffix.strip_prefix(':') {
                Some(port) => ("[::1]", Some(port)),
                None => return false,
            },
            _ => return false,
        },
        None => match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };
    if !host.eq_ignore_ascii_case("localhost") && host != "127.0.0.1" && host != "[::1]" {
        return false;
    }
    port.is_none_or(|p| !p.is_empty() && p.parse::<u16>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_refill_slowly_and_cap() {
        let ip: IpAddr = Ipv4Addr::new(10, 0, 0, 2).into();
        let mut b = HashMap::new();
        assert_eq!(tokens(&mut b, ip, 0), BUDGET);
        b.get_mut(&ip).unwrap().0 = 0;
        assert_eq!(tokens(&mut b, ip, REFILL_MS - 1), 0);
        assert_eq!(tokens(&mut b, ip, REFILL_MS), 1);
        assert_eq!(tokens(&mut b, ip, 100 * REFILL_MS), BUDGET);
    }

    #[test]
    fn only_loopback_origins_pass() {
        for ok in [
            "http://127.0.0.1:5173",
            "http://localhost",
            "http://[::1]:8080",
        ] {
            assert!(loopback_origin(ok), "{ok}");
        }
        for bad in [
            "https://example.com",
            "http://192.168.1.2",
            "http://localhost.evil.com",
            "http://localhost:1@evil.com",
            "http://127.0.0.2",
            "http://localhost:",
            "null",
        ] {
            assert!(!loopback_origin(bad), "{bad}");
        }
    }

    /// Registered (and, where multicast loops back, seen by a browser
    /// carrying only the version and id), then withdrawn.
    #[test]
    fn the_advert_is_registered_and_withdrawn() {
        let home = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let hub = Hub::new(home.path().into(), tx).unwrap();
        let port = 45_871;
        if let Err(e) = hub.advertise(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port))) {
            eprintln!("SKIPPED: no mDNS socket here: {e}");
            return;
        }
        let browser = mdns_sd::ServiceDaemon::new().unwrap();
        let found = browser.browse(SERVICE).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = None;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            match found.recv_timeout(left) {
                Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) if info.port == port => {
                    seen = Some(info);
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        match seen {
            Some(info) => {
                let mut keys: Vec<&str> = info.txt_properties.iter().map(|p| p.key()).collect();
                keys.sort();
                assert_eq!(keys, ["id", "v"]);
                assert_eq!(
                    info.get_property_val_str("id"),
                    Some(hex(&hub.0.id).as_str())
                );
            }
            None => eprintln!("multicast does not loop back here: browse check skipped"),
        }
        let _ = browser.shutdown();
        let (daemon, fullname) = hub.0.mdns.lock().unwrap().take().unwrap();
        let status = daemon.unregister(&fullname).unwrap();
        let status = status.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            matches!(status, mdns_sd::UnregisterStatus::OK),
            "{status:?}"
        );
        let _ = daemon.shutdown();
    }

    #[test]
    fn limits_count_an_ipv6_slash_64_as_one_address() {
        let a: IpAddr = "2001:db8:1:2:aaaa::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:bbbb::9".parse().unwrap();
        let c: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(key(a), key(b));
        assert_ne!(key(a), key(c));
        let v4: IpAddr = "10.0.0.2".parse().unwrap();
        let mapped: IpAddr = "::ffff:10.0.0.2".parse().unwrap();
        assert_eq!(key(mapped), v4);
    }
}
