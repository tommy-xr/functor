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

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use functor_app_core::devices::{LinkCommand, LinkEvent, LinkKind, Paired};
use functor_app_core::Event;
use functor_device_link::desktop::{self, Desktop};
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

/// The DNS-SD service type.
pub const SERVICE: &str = "_functor-link._tcp.local.";
/// The one path the listener answers.
pub const PATH: &str = "/link";

/// Connections not yet authenticated, at once: all, and from one address.
const MAX_PENDING: usize = 16;
const MAX_PENDING_PER_IP: usize = 4;
/// Pairing prompts open at once.
const MAX_ASKING: usize = 4;
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
}

#[derive(Clone)]
pub struct Hub(Arc<Inner>);

struct Inner {
    home: PathBuf,
    id: DesktopId,
    name: String,
    events: UnboundedSender<Event>,
    state: Mutex<HubState>,
    /// The advert and its full name, while registered.
    mdns: Mutex<Option<(mdns_sd::ServiceDaemon, String)>>,
}

#[derive(Default)]
struct HubState {
    pairings: Vec<StoredPairing>,
    conns: HashMap<u64, Conn>,
    tunnels: HashMap<String, (String, JoinHandle<()>)>,
    pending: usize,
    pending_per_ip: HashMap<IpAddr, usize>,
    asking: usize,
    /// Per address: (tokens left, when last refilled).
    budgets: HashMap<IpAddr, (u64, u64)>,
    next_conn: u64,
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
    Request(
        wire::Request,
        oneshot::Sender<Result<wire::Response, String>>,
    ),
}

impl Hub {
    /// The hub for `home`: its desktop id (made once) and stored pairings.
    pub fn new(home: PathBuf, events: UnboundedSender<Event>) -> std::io::Result<Hub> {
        let id = store::desktop_id(&home)?;
        let pairings = store::load_pairings(&home);
        let name = desktop_name();
        Ok(Hub(Arc::new(Inner {
            home,
            id,
            name,
            events,
            state: Mutex::new(HubState {
                pairings,
                ..HubState::default()
            }),
            mdns: Mutex::new(None),
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
                    continue;
                };
                if hub.admit(peer.ip()) {
                    tokio::spawn(hub.clone().connection(stream, peer.ip()));
                }
            }
        });
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
        let host = format!("{}.local.", host_label(&self.0.name));
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
        if let Some(n) = s.pending_per_ip.get_mut(&ip) {
            *n -= 1;
            if *n == 0 {
                s.pending_per_ip.remove(&ip);
            }
        }
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
        }
    }

    /// Deletes `device`'s pairing (its secret with it), then closes its
    /// links and its tunnel.
    fn forget(&self, device: &str) {
        let mut s = self.0.state.lock().unwrap();
        s.pairings.retain(|p| p.id != device);
        if let Err(e) = store::save_pairings(&self.0.home, &s.pairings) {
            eprintln!("functor-app: cannot save the pairings: {e}");
        }
        for conn in s
            .conns
            .values()
            .filter(|c| c.linked.as_deref() == Some(device))
        {
            let _ = conn.cmds.send(ConnCmd::Close);
        }
        if let Some((_, task)) = s.tunnels.remove(device) {
            task.abort();
        }
    }

    fn store_pairing(&self, hello: &Hello, secret: &Secret) -> Paired {
        let now = wall_ms();
        let id = hex(&hello.device);
        let mut s = self.0.state.lock().unwrap();
        let first_seen = s
            .pairings
            .iter()
            .find(|p| p.id == id)
            .map_or(now, |p| p.first_seen);
        s.pairings.retain(|p| p.id != id);
        let pairing = StoredPairing {
            id,
            name: hello.name.clone(),
            secret: secret.to_hex(),
            first_seen,
            last_seen: now,
        };
        let shown = paired(&pairing);
        s.pairings.push(pairing);
        if let Err(e) = store::save_pairings(&self.0.home, &s.pairings) {
            eprintln!("functor-app: cannot save the pairings: {e}");
        }
        shown
    }

    /// Notes that `device` was seen now, kept for its card after a restart.
    fn seen(&self, device: &str) {
        let mut s = self.0.state.lock().unwrap();
        if let Some(p) = s.pairings.iter_mut().find(|p| p.id == device) {
            p.last_seen = wall_ms();
            if let Err(e) = store::save_pairings(&self.0.home, &s.pairings) {
                eprintln!("functor-app: cannot save the pairings: {e}");
            }
        }
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
            linked: None,
            waiting: HashMap::new(),
            next_request: 0,
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
        let app = Router::new()
            .fallback(tunnel)
            .layer(DefaultBodyLimit::max(MAX_BODY))
            .with_state((self.clone(), device.clone()));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        {
            let mut s = self.0.state.lock().unwrap();
            // Forgotten meanwhile.
            if !s.pairings.iter().any(|p| p.id == device) {
                task.abort();
                return;
            }
            s.tunnels.insert(device.clone(), (url.clone(), task));
        }
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

/// A DNS label for `name`: `Bryan's Mac` → `Bryan-s-Mac`.
fn host_label(name: &str) -> String {
    let label: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(63)
        .collect();
    match label.trim_matches('-') {
        "" => "functor".to_string(),
        l => l.to_string(),
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
    /// The device's hex id, once linked.
    linked: Option<String>,
    waiting: HashMap<u32, oneshot::Sender<Result<wire::Response, String>>>,
    next_request: u32,
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
                                let _ = ws.send(Message::Binary(refused.into())).await;
                                return Err(Error::Busy);
                            }
                        }
                        let hub = &self.hub;
                        self.machine.receive(&bytes, now_ms(), |d| hub.secret(&hex(d)))
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(_)) => return Err(Error::Unexpected("a text message")),
                    // The socket just went (the app quit, the headset slept):
                    // gone, not failing.
                    Some(Err(WsError::Io(_) | WsError::ConnectionClosed | WsError::Protocol(
                        ProtocolError::ResetWithoutClosingHandshake,
                    ))) => return Ok(()),
                    Some(Err(e)) => return Err(Error::Closed(e.to_string())),
                },
                Some(cmd) = cmds.recv() => match cmd {
                    ConnCmd::Allow => self.machine.allow(now_ms()),
                    ConnCmd::Deny => self.machine.deny(),
                    ConnCmd::Close => {
                        if let Ok(msgs) = self.machine.send(&Frame::Close("closed by the desktop".into())) {
                            for m in msgs {
                                let _ = ws.send(Message::Binary(m.into())).await;
                            }
                        }
                        let _ = ws.close(None).await;
                        return Ok(());
                    }
                    ConnCmd::Request(mut request, reply) => {
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
                _ = tick.tick() => match self.machine.check(now_ms()) {
                    Some(e) => return Err(e),
                    None => continue,
                },
                _ = ping.tick(), if self.machine.linked() => {
                    match self.machine.send(&Frame::Ping(now_ms())) {
                        Ok(msgs) => desktop_output(msgs),
                        Err(e) => return Err(e),
                    }
                }
            };
            for msg in out.send {
                if ws.send(Message::Binary(msg.into())).await.is_err() {
                    return Err(Error::Closed("the connection dropped".into()));
                }
            }
            for event in out.events {
                self.event(event);
            }
            match out.end {
                None => {}
                Some(End::Done) => {
                    let _ = ws.close(None).await;
                    return Ok(());
                }
                Some(End::Failed(e)) => return Err(e),
            }
        }
    }

    /// A pairing hello while too many prompts are open: the refusal to send.
    fn refuse_pairing(&mut self, first: &[u8]) -> Option<Vec<u8>> {
        let Ok((hello, _)) = Hello::decode(first) else {
            return None;
        };
        if hello.mode != Mode::Pair {
            return None;
        }
        // Every pairing attempt costs, so prompts cannot be spammed.
        self.hub.spend(self.ip);
        let busy = self.hub.0.state.lock().unwrap().asking >= MAX_ASKING;
        busy.then(|| Reply::Reject(Reject::Busy).encode())
    }

    fn event(&mut self, event: desktop::Event) {
        let hub = self.hub.clone();
        match event {
            desktop::Event::PairRequest {
                hello,
                code,
                expires_at,
            } => {
                hub.0.state.lock().unwrap().asking += 1;
                self.asking = true;
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
                self.paired = true;
                let device = hub.store_pairing(&hello, &secret);
                hub.send(LinkEvent::Paired {
                    request: self.id,
                    device,
                });
            }
            desktop::Event::Linked { hello } => {
                let device = hex(&hello.device);
                self.unpend();
                // The newest link of a device wins.
                {
                    let mut s = hub.0.state.lock().unwrap();
                    for (id, c) in s.conns.iter() {
                        if *id != self.id && c.linked.as_deref() == Some(&device) {
                            let _ = c.cmds.send(ConnCmd::Close);
                        }
                    }
                    if let Some(c) = s.conns.get_mut(&self.id) {
                        c.linked = Some(device.clone());
                    }
                }
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
            // No channels are defined yet.
            desktop::Event::Channel { .. } => {}
            desktop::Event::Mismatch(pre) => hub.send(LinkEvent::Mismatch {
                device: hex(&pre.device),
                name: pre.name,
                theirs: pre.version,
                ours: VERSION,
            }),
        }
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
            hub.0.state.lock().unwrap().asking -= 1;
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

/// One HTTP request on a device's tunnel port, forwarded over its link.
async fn tunnel(
    State((hub, device)): State<(Hub, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // As the debug runtime itself: no page from another origin.
    if let Some(origin) = headers.get(header::ORIGIN) {
        if !origin.to_str().is_ok_and(loopback_origin) {
            return (StatusCode::FORBIDDEN, "origin is not allowed").into_response();
        }
    }
    if uri.path() == "/stream" {
        return (StatusCode::NOT_IMPLEMENTED, "streams are not tunnelled yet").into_response();
    }
    let Some(link) = hub.link_of(&device) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "the headset is not connected",
        )
            .into_response();
    };
    let request = wire::Request {
        id: 0,
        method: method.as_str().to_string(),
        target: uri.path_and_query().map_or("/", |p| p.as_str()).to_string(),
        headers: REQUEST_HEADERS
            .iter()
            .filter_map(|h| {
                Some((
                    h.as_str().to_string(),
                    headers.get(h)?.to_str().ok()?.to_string(),
                ))
            })
            .collect(),
        body: body.to_vec(),
    };
    let (reply, answer) = oneshot::channel();
    if link.send(ConnCmd::Request(request, reply)).is_err() {
        return (StatusCode::BAD_GATEWAY, "the headset disconnected").into_response();
    }
    let wait = match method {
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
        Ok(Err(_)) => (StatusCode::BAD_GATEWAY, "the headset disconnected").into_response(),
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, "the headset did not answer").into_response(),
    }
}

/// `http://127.0.0.1:5173`, `http://localhost`, `http://[::1]:8080`.
fn loopback_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    let host = match rest.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map_or("", |(h, _)| h),
        None => rest.split(':').next().unwrap_or(""),
    };
    match host {
        "localhost" => true,
        host => host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()),
    }
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
            "null",
        ] {
            assert!(!loopback_origin(bad), "{bad}");
        }
    }

    #[test]
    fn host_labels_are_dns_safe() {
        assert_eq!(host_label("Bryan's Mac"), "Bryan-s-Mac");
        assert_eq!(host_label("!!"), "functor");
    }
}
