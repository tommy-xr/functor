//! The headset's side of the device link as one background thread: it finds
//! its desktop by mDNS, pairs or links with [`Device`], forwards each
//! tunnelled request to the local debug runtime over loopback HTTP, sends a
//! [`Status`] every [`HEARTBEAT_MS`] and reconnects with backoff. Blocking
//! `std::net` only, so it runs wherever the debug runtime's own server does.
//!
//! The shell supplies the status ([`Host`]), sends the user's [`Command`]s
//! and draws [`Client::pairing`]; an unpaired, idle client costs the shell
//! one atomic load per call.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

use crate::device::{self, Device, Identity};
pub use crate::file::Stored;
use crate::wire::{Frame, Request, Response, Status};
use crate::{hex, shared, DesktopId, DeviceId, End, Error, OsEntropy, Output, Secret};
use crate::{HEARTBEAT_MS, MAX_FRAME, MAX_MESSAGE, PATH, REPAIRED, SERVICE};

/// How long one mDNS browse looks for a desktop.
const BROWSE: Duration = Duration::from_secs(10);
/// Reconnect backoff: doubles from the first to the last.
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// How often the pairing file is re-read (unpaired, waiting to reconnect,
/// or linked), so one written from outside (the cable path) is picked up
/// without a restart.
const IDLE_RECHECK: Duration = Duration::from_secs(5);
/// Socket read timeout while requests are in flight (how soon an answer is
/// sent) and while none are (how soon a command or heartbeat is handled).
const POLL_BUSY: Duration = Duration::from_millis(5);
const POLL_IDLE: Duration = Duration::from_millis(200);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// A tunnelled request's answer: past the debug runtime's longest (a load,
/// 180 s), so it answers first.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(200);
/// How long a pairing's outcome stays shown.
const OUTCOME_SHOWN: Duration = Duration::from_secs(4);
/// Request headers passed to the debug runtime (the ones it reads).
const FORWARDED: [&str; 3] = ["content-type", "accept", "origin"];
/// Bytes of bounded channel data queued for the desktop, past which
/// [`Client::send_channel`] refuses more: about 8 s of voice audio.
pub const OUTBOX_LIMIT: usize = 256 * 1024;
/// Channel messages from the desktop kept until the shell takes them; the
/// oldest go first.
const INBOX_LIMIT: usize = 32;

pub struct Config {
    /// The pairing file ([`Stored`]).
    pub store: PathBuf,
    /// The device's name in its hello (`Quest 3`).
    pub name: String,
    pub app_version: String,
    /// The debug runtime tunnelled requests are sent to.
    pub debug: SocketAddr,
    /// A desktop's link address (`host:port`) to dial instead of browsing.
    pub desktop: Option<String>,
}

/// What the shell provides.
pub trait Host: Send + 'static {
    /// The heartbeat.
    fn status(&self) -> Status;
    /// `true` before an mDNS browse, `false` after: Android receives
    /// multicast only while the app holds a multicast lock.
    fn browsing(&self, _on: bool) {}
}

/// What the headset's user asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Pair with a desktop found on the network, replacing any pairing once
    /// it succeeds.
    Pair,
    /// The desktop shows the same code.
    Confirm,
    Cancel,
}

/// A pairing, as the headset shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pairing {
    Searching,
    /// Compare `code` with the desktop's; `confirmed` once the user said it
    /// matches (then the desktop's user decides).
    Code {
        desktop: String,
        code: String,
        confirmed: bool,
    },
    Paired {
        desktop: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// The paired desktop's name.
    pub paired: Option<String>,
    pub linked: bool,
    pub pairing: Option<Pairing>,
}

#[derive(Default)]
struct Shown {
    view: View,
    /// When a pairing outcome stops showing.
    hide_at: Option<Instant>,
}

/// Channel messages waiting for the desktop, and their bounded bytes.
#[derive(Default)]
struct Outbox {
    queue: VecDeque<(u16, Vec<u8>)>,
    bounded: usize,
    /// The link they are for, numbered from 1, while one is up.
    link: Option<u64>,
    links: u64,
}

impl Outbox {
    /// A link came up or went: nothing queued carries over.
    fn reset(&mut self, up: bool) {
        self.queue.clear();
        self.bounded = 0;
        self.link = up.then(|| {
            self.links += 1;
            self.links
        });
    }
}

#[derive(Default)]
struct Shared {
    shown: Mutex<Shown>,
    /// A pairing is shown, the link is up, a desktop is paired: the shell's
    /// fast paths.
    pairing: AtomicBool,
    linked: AtomicBool,
    paired: AtomicBool,
    outbox: Mutex<Outbox>,
    inbox: Mutex<VecDeque<(u16, Vec<u8>)>>,
    /// The inbox holds something: the shell's fast path.
    received: AtomicBool,
}

impl Shared {
    fn update(&self, f: impl FnOnce(&mut View)) {
        let mut shown = self.shown.lock().unwrap();
        let before = shown.view.pairing.clone();
        f(&mut shown.view);
        if shown.view.pairing != before {
            shown.hide_at = match shown.view.pairing {
                Some(Pairing::Paired { .. } | Pairing::Failed { .. }) => {
                    Some(Instant::now() + OUTCOME_SHOWN)
                }
                _ => None,
            };
        }
        self.pairing
            .store(shown.view.pairing.is_some(), Ordering::Release);
        self.linked.store(shown.view.linked, Ordering::Release);
        self.paired
            .store(shown.view.paired.is_some(), Ordering::Release);
    }

    fn view(&self) -> View {
        let mut shown = self.shown.lock().unwrap();
        if shown.hide_at.is_some_and(|at| Instant::now() >= at) {
            shown.view.pairing = None;
            shown.hide_at = None;
            self.pairing.store(false, Ordering::Release);
        }
        shown.view.clone()
    }
}

/// The running client. Dropping it stops the thread at its next wait.
pub struct Client {
    commands: Sender<Command>,
    shared: Arc<Shared>,
}

impl Client {
    pub fn spawn(config: Config, host: impl Host) -> io::Result<Client> {
        let stored = Stored::load(&config.store)?;
        let (commands, rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        shared.update(|v| v.paired = stored.pairing().and(stored.desktop_name.clone()));
        let worker = Worker {
            config,
            host,
            commands: rx,
            shared: shared.clone(),
            wants_pairing: false,
        };
        std::thread::Builder::new()
            .name("functor-link".into())
            .spawn(move || worker.run())?;
        Ok(Client { commands, shared })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// The pairing to show, if any.
    pub fn pairing(&self) -> Option<Pairing> {
        if !self.shared.pairing.load(Ordering::Acquire) {
            return None;
        }
        self.shared.view().pairing
    }

    pub fn view(&self) -> View {
        self.shared.view()
    }

    pub fn linked(&self) -> bool {
        self.shared.linked.load(Ordering::Acquire)
    }

    /// A desktop is paired, linked or not.
    pub fn paired(&self) -> bool {
        self.shared.paired.load(Ordering::Acquire)
    }

    /// The current link's number, while one is up; a new link has a new one.
    pub fn link(&self) -> Option<u64> {
        self.shared.outbox.lock().unwrap().link
    }

    /// Queues `data` on `channel` for the desktop on `link` ([`Client::link`]).
    /// `false` (dropped) once that link is gone, or when `bounded` and the
    /// queue already holds [`OUTBOX_LIMIT`] bytes of bounded data. A link
    /// drop empties the queue: nothing is resent on the next link.
    pub fn send_channel(&self, link: u64, channel: u16, data: Vec<u8>, bounded: bool) -> bool {
        let mut outbox = self.shared.outbox.lock().unwrap();
        if outbox.link != Some(link) {
            return false;
        }
        if bounded {
            if outbox.bounded >= OUTBOX_LIMIT {
                return false;
            }
            outbox.bounded += data.len();
        }
        outbox.queue.push_back((channel, data));
        true
    }

    /// Channel messages from the desktop since the last call, oldest first.
    pub fn received(&self) -> Vec<(u16, Vec<u8>)> {
        if !self.shared.received.swap(false, Ordering::Acquire) {
            return Vec::new();
        }
        self.shared.inbox.lock().unwrap().drain(..).collect()
    }
}

/// Push-to-talk's messages, on the voice clip channel of one link (the
/// `u64`, [`Client::link`]); audio is bounded.
impl crate::talk::Sink for (&Client, u64) {
    fn send(&mut self, clip: crate::voice::Clip, audio: bool) -> bool {
        clip.encode().is_ok_and(|data| {
            self.0
                .send_channel(self.1, crate::voice::CLIP_CHANNEL, data, audio)
        })
    }
}

/// How a connection ended.
enum Ended {
    Paired,
    /// It was linked (backoff restarts) or not.
    Over {
        linked: bool,
        why: String,
    },
    /// The user asked to pair: drop this link and pair.
    PairRequested,
    /// The user cancelled the pairing.
    Cancelled,
    /// The [`Client`] was dropped.
    Gone,
}

enum Waited {
    Out,
    Changed,
    Gone,
}

enum Found {
    /// Its addresses, to try in turn.
    At(Vec<String>),
    Nothing,
    Command(Command),
    Gone,
}

struct Worker<H> {
    config: Config,
    host: H,
    commands: Receiver<Command>,
    shared: Arc<Shared>,
    wants_pairing: bool,
}

impl<H: Host> Worker<H> {
    fn run(mut self) {
        let mut backoff = BACKOFF_MIN;
        loop {
            // Re-read each time: a pairing written from outside lands here.
            let stored = match Stored::load(&self.config.store) {
                Ok(stored) => stored,
                Err(e) => {
                    log::error!("device link: pairing file {:?}: {e}", self.config.store);
                    Stored::default()
                }
            };
            let Some(device) = stored.device() else {
                if !self.wait(IDLE_RECHECK) {
                    return;
                }
                continue;
            };
            if std::mem::take(&mut self.wants_pairing) {
                if let Ended::Gone = self.pair(device) {
                    return;
                }
                backoff = BACKOFF_MIN;
                continue;
            }
            let Some((desktop, secret)) = stored.pairing() else {
                // Forgotten: push-to-talk stands down.
                self.shared.update(|v| v.paired = None);
                if !self.wait(IDLE_RECHECK) {
                    return;
                }
                continue;
            };
            self.shared
                .update(|v| v.paired = stored.desktop_name.clone());
            let ended = match self.find(Some(desktop)) {
                Found::At(addrs) => self.session(&addrs, device, Some(&secret)),
                Found::Nothing => Ended::Over {
                    linked: false,
                    why: "desktop not found".into(),
                },
                Found::Command(Command::Pair) => Ended::PairRequested,
                Found::Command(_) => continue,
                Found::Gone => Ended::Gone,
            };
            self.shared.update(|v| v.linked = false);
            // Nothing queued for this link reaches the next one.
            self.shared.outbox.lock().unwrap().reset(false);
            match ended {
                Ended::Gone => return,
                Ended::PairRequested => {
                    log::info!("device link: closed to pair again");
                    self.wants_pairing = true;
                    continue;
                }
                Ended::Over { linked, why } => {
                    log::info!("device link: {why}");
                    if linked {
                        backoff = BACKOFF_MIN;
                    }
                }
                Ended::Paired | Ended::Cancelled => {}
            }
            match self.wait_paired(backoff, &secret) {
                Waited::Gone => return,
                // A pairing written from outside: try it now.
                Waited::Changed => backoff = BACKOFF_MIN,
                Waited::Out => backoff = (backoff * 2).min(BACKOFF_MAX),
            }
        }
    }

    /// Whether the pairing file no longer holds `secret` (re-paired, here
    /// or over the cable, or its pairing removed). An unreadable file
    /// changes nothing.
    fn pairing_changed(&self, secret: &Secret) -> bool {
        match Stored::load(&self.config.store) {
            Ok(stored) => stored.pairing().map(|(_, s)| s).as_ref() != Some(secret),
            Err(_) => false,
        }
    }

    /// Waits `d` as [`Worker::wait`] does, looking at the pairing file every
    /// [`IDLE_RECHECK`] so a new one is not left waiting out the backoff.
    fn wait_paired(&mut self, d: Duration, secret: &Secret) -> Waited {
        let until = Instant::now() + d;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() || self.wants_pairing {
                return Waited::Out;
            }
            if !self.wait(left.min(IDLE_RECHECK)) {
                return Waited::Gone;
            }
            if self.pairing_changed(secret) {
                return Waited::Changed;
            }
        }
    }

    /// Waits `d` for a command; `false` once the client is dropped.
    fn wait(&mut self, d: Duration) -> bool {
        match self.commands.recv_timeout(d) {
            Ok(Command::Pair) => {
                self.wants_pairing = true;
                true
            }
            Ok(_) | Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => false,
        }
    }

    fn pair(&mut self, device: DeviceId) -> Ended {
        self.shared.update(|v| v.pairing = Some(Pairing::Searching));
        let ended = match self.find(None) {
            Found::At(addrs) => self.session(&addrs, device, None),
            Found::Nothing => Ended::Over {
                linked: false,
                why: "No Functor desktop found on this network".into(),
            },
            Found::Command(Command::Cancel) => Ended::Cancelled,
            // Pair again: start over.
            Found::Command(_) => return self.pair(device),
            Found::Gone => Ended::Gone,
        };
        match &ended {
            // Unless the connection already said why.
            Ended::Over { why, .. } => self.shared.update(|v| {
                if !matches!(v.pairing, Some(Pairing::Failed { .. })) {
                    v.pairing = Some(Pairing::Failed {
                        reason: why.clone(),
                    });
                }
            }),
            Ended::Cancelled => self.shared.update(|v| v.pairing = None),
            Ended::Paired | Ended::PairRequested | Ended::Gone => {}
        }
        ended
    }

    /// A desktop's link address: the configured one, else by mDNS — the one
    /// whose TXT `id` is `desktop`, or (pairing) the first found.
    fn find(&mut self, desktop: Option<DesktopId>) -> Found {
        if let Some(addr) = &self.config.desktop {
            return Found::At(vec![addr.clone()]);
        }
        self.host.browsing(true);
        let found = self.browse(desktop);
        self.host.browsing(false);
        found
    }

    fn browse(&mut self, desktop: Option<DesktopId>) -> Found {
        let daemon = match mdns_sd::ServiceDaemon::new() {
            Ok(d) => d,
            Err(e) => {
                log::warn!("device link: mDNS: {e}");
                return Found::Nothing;
            }
        };
        let found = match daemon.browse(SERVICE) {
            Ok(found) => found,
            Err(e) => {
                log::warn!("device link: mDNS browse: {e}");
                let _ = daemon.shutdown();
                return Found::Nothing;
            }
        };
        let want = desktop.map(|d| hex(&d));
        let deadline = Instant::now() + BROWSE;
        // What answered, for the log when ours did not.
        let mut seen = Vec::new();
        let result = loop {
            match self.commands.try_recv() {
                // Nothing to confirm yet.
                Ok(Command::Confirm) => {}
                Ok(command) => break Found::Command(command),
                Err(mpsc::TryRecvError::Disconnected) => break Found::Gone,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                log::info!("device link: no desktop {want:?} among {seen:?}");
                break Found::Nothing;
            };
            let info = match found.recv_timeout(left.min(Duration::from_millis(250))) {
                Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => info,
                _ => continue,
            };
            let id = info.get_property_val_str("id");
            seen.push(format!(
                "{} {id:?} {:?}",
                info.get_fullname(),
                info.get_addresses_v4()
            ));
            if want.is_some() && id != want.as_deref() {
                continue;
            }
            let port = info.get_port();
            let addrs: Vec<String> = info
                .get_addresses_v4()
                .into_iter()
                .map(|ip| format!("{ip}:{port}"))
                .collect();
            if !addrs.is_empty() {
                break Found::At(addrs);
            }
        };
        let _ = daemon.shutdown();
        result
    }

    /// One connection, to the first of `addrs` that answers: pairs without
    /// a secret, else links.
    fn session(&mut self, addrs: &[String], device: DeviceId, secret: Option<&Secret>) -> Ended {
        let mut tried = Vec::new();
        let Some(mut ws) = addrs.iter().find_map(|addr| {
            connect(addr)
                .map_err(|why| tried.push(format!("{addr}: {why}")))
                .ok()
        }) else {
            return Ended::over(
                false,
                format!("cannot reach the desktop ({})", tried.join("; ")),
            );
        };
        let identity = Identity {
            device,
            name: self.config.name.clone(),
            app_version: self.config.app_version.clone(),
        };
        let entropy = shared(OsEntropy);
        let (mut machine, first) = match secret {
            Some(secret) => Device::link(&identity, secret, self.host.status(), entropy, now()),
            None => Device::pair(&identity, entropy, now()),
        };
        let mut conn = Connection {
            ws: &mut ws,
            responses: mpsc::channel(),
            in_flight: 0,
            linked: false,
            paired: false,
            cancelled: false,
            debug: self.config.debug,
        };
        let result = (|| -> Result<std::convert::Infallible, Ended> {
            conn.apply(first, self)?;
            let mut next_beat = now() + HEARTBEAT_MS;
            let mut next_recheck = now() + IDLE_RECHECK.as_millis() as u64;
            loop {
                let queued = !self.shared.outbox.lock().unwrap().queue.is_empty();
                let poll = if conn.in_flight > 0 || queued {
                    POLL_BUSY
                } else {
                    POLL_IDLE
                };
                let _ = conn.ws.get_ref().set_read_timeout(Some(poll));
                match conn.ws.read() {
                    Ok(Message::Binary(bytes)) => {
                        let out = machine.receive(&bytes, now());
                        conn.apply(out, self)?;
                    }
                    Ok(Message::Close(_)) => {
                        return Err(Ended::over(conn.linked, "closed by the desktop"))
                    }
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(e))
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) => {}
                    Err(e) => return Err(Ended::over(conn.linked, e.to_string())),
                }
                while let Ok(response) = conn.responses.1.try_recv() {
                    conn.in_flight -= 1;
                    conn.respond(&mut machine, response)?;
                }
                loop {
                    let command = match self.commands.try_recv() {
                        Ok(command) => command,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return Err(Ended::Gone),
                    };
                    match command {
                        Command::Confirm => {
                            let out = machine.confirm();
                            if out.end.is_none() {
                                self.shared.update(|v| {
                                    if let Some(Pairing::Code { confirmed, .. }) = &mut v.pairing {
                                        *confirmed = true;
                                    }
                                });
                            }
                            conn.apply(out, self)?;
                        }
                        // Pairing, before the code shows too: hang up.
                        Command::Cancel if secret.is_none() => {
                            conn.cancelled = true;
                            let out = machine.cancel();
                            conn.apply(out, self)?;
                            return Err(Ended::Cancelled);
                        }
                        // Linking, or about to: drop it and pair.
                        Command::Pair if secret.is_some() => {
                            if let Ok(msgs) = machine.send(&Frame::Close("pairing again".into())) {
                                let _ = conn.send_all(msgs);
                            }
                            return Err(Ended::PairRequested);
                        }
                        Command::Cancel | Command::Pair => {}
                    }
                }
                let t = now();
                // Linked, and the file was paired again meanwhile: say so to
                // this desktop and start over with the new pairing.
                if let Some(secret) = secret.filter(|_| t >= next_recheck) {
                    next_recheck = t + IDLE_RECHECK.as_millis() as u64;
                    if self.pairing_changed(secret) {
                        if let Ok(msgs) = machine.send(&Frame::Close(REPAIRED.into())) {
                            let _ = conn.send_all(msgs);
                        }
                        return Err(Ended::over(true, "the pairing file changed"));
                    }
                }
                if machine.linked() {
                    let queued = {
                        let mut outbox = self.shared.outbox.lock().unwrap();
                        outbox.bounded = 0;
                        std::mem::take(&mut outbox.queue)
                    };
                    for (channel, data) in queued {
                        let msgs = machine
                            .send(&Frame::Channel { channel, data })
                            .map_err(|e| Ended::over(true, e.to_string()))?;
                        conn.send_all(msgs)?;
                    }
                }
                if machine.linked() && t >= next_beat {
                    next_beat = t + HEARTBEAT_MS;
                    let msgs = machine
                        .send(&Frame::Status(self.host.status()))
                        .map_err(|e| Ended::over(true, e.to_string()))?;
                    conn.send_all(msgs)?;
                }
                if let Some(e) = machine.check(t) {
                    return Err(conn.failed(e, self));
                }
            }
        })();
        let _ = ws.close(None);
        let _ = ws.flush();
        match result {
            Ok(never) => match never {},
            Err(ended) => ended,
        }
    }
}

impl Ended {
    fn over(linked: bool, why: impl Into<String>) -> Ended {
        Ended::Over {
            linked,
            why: why.into(),
        }
    }
}

type Socket = WebSocket<TcpStream>;

fn connect(addr: &str) -> Result<Socket, String> {
    let target = addr
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("no address")?;
    let stream = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    stream
        .set_read_timeout(Some(CONNECT_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(WRITE_TIMEOUT)))
        .map_err(|e| e.to_string())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    tungstenite::client::client_with_config(format!("ws://{addr}{PATH}"), stream, Some(config))
        .map(|(ws, _)| ws)
        .map_err(|e| e.to_string())
}

/// One connection's sockets and progress.
struct Connection<'a> {
    ws: &'a mut Socket,
    responses: (Sender<Response>, Receiver<Response>),
    in_flight: usize,
    linked: bool,
    paired: bool,
    /// Our user cancelled.
    cancelled: bool,
    debug: SocketAddr,
}

impl Connection<'_> {
    fn send_all(&mut self, msgs: Vec<Vec<u8>>) -> Result<(), Ended> {
        for msg in msgs {
            self.ws
                .send(Message::Binary(msg.into()))
                .map_err(|e| Ended::over(self.linked, e.to_string()))?;
        }
        Ok(())
    }

    fn respond(&mut self, machine: &mut Device, response: Response) -> Result<(), Ended> {
        let id = response.id;
        let msgs = match machine.respond(response) {
            Ok(msgs) => msgs,
            // Too large or not text the link carries: say so instead.
            Err(e) => machine
                .respond(text(
                    id,
                    502,
                    &format!("the response cannot cross the link: {e}"),
                ))
                .map_err(|e| Ended::over(self.linked, e.to_string()))?,
        };
        self.send_all(msgs)
    }

    /// Sends what the machine produced and acts on its events; `Err` once
    /// the connection is over.
    fn apply<H: Host>(
        &mut self,
        out: Output<device::Event>,
        worker: &Worker<H>,
    ) -> Result<(), Ended> {
        // Events first: a pairing is stored even if its PairDone fails to send.
        for event in out.events {
            match event {
                device::Event::Code {
                    desktop_name, code, ..
                } => {
                    worker.shared.update(|v| {
                        v.pairing = Some(Pairing::Code {
                            desktop: desktop_name,
                            code,
                            confirmed: false,
                        })
                    });
                }
                device::Event::Paired {
                    desktop,
                    desktop_name,
                    secret,
                } => {
                    let stored = Stored::load(&worker.config.store).and_then(|mut stored| {
                        stored.desktop = Some(hex(&desktop));
                        stored.desktop_name = Some(desktop_name.clone());
                        stored.secret = Some(secret.to_hex());
                        stored.save(&worker.config.store)
                    });
                    if let Err(e) = stored {
                        let reason = format!("Could not store the pairing: {e}");
                        worker
                            .shared
                            .update(|v| v.pairing = Some(Pairing::Failed { reason }));
                        return Err(Ended::over(false, "pairing not stored"));
                    }
                    log::info!("device link: paired with {desktop_name}");
                    self.paired = true;
                    worker.shared.update(|v| {
                        v.paired = Some(desktop_name.clone());
                        v.pairing = Some(Pairing::Paired {
                            desktop: desktop_name,
                        });
                    });
                }
                device::Event::Linked { desktop_name, .. } => {
                    log::info!("device link: linked to {desktop_name}");
                    self.linked = true;
                    worker.shared.outbox.lock().unwrap().reset(true);
                    worker.shared.update(|v| {
                        v.linked = true;
                        v.paired = Some(desktop_name);
                    });
                }
                device::Event::Request(request) => {
                    self.in_flight += 1;
                    let (tx, debug) = (self.responses.0.clone(), self.debug);
                    std::thread::spawn(move || {
                        let _ = tx.send(forward(debug, &request));
                    });
                }
                device::Event::Channel { channel, data } => {
                    let mut inbox = worker.shared.inbox.lock().unwrap();
                    if inbox.len() == INBOX_LIMIT {
                        inbox.pop_front();
                    }
                    inbox.push_back((channel, data));
                    worker.shared.received.store(true, Ordering::Release);
                }
                // Forgeable: report, never drop the stored secret over it.
                device::Event::Rejected(reject) => {
                    log::warn!("device link: the desktop refused: {reject:?}")
                }
            }
        }
        let sent = self.send_all(out.send);
        // Stored: a lost PairDone is harmless.
        if !self.paired {
            sent?;
        }
        match out.end {
            None => Ok(()),
            Some(End::Done) if self.paired => Err(Ended::Paired),
            Some(End::Done) => Err(Ended::over(self.linked, "closed")),
            Some(End::Failed(e)) => Err(self.failed(e, worker)),
        }
    }

    fn failed<H: Host>(&self, e: Error, worker: &Worker<H>) -> Ended {
        if self.cancelled {
            return Ended::Cancelled;
        }
        // Pairing: the user sees why.
        if matches!(
            worker.shared.view().pairing,
            Some(Pairing::Code { .. } | Pairing::Searching)
        ) {
            let reason = match &e {
                Error::Denied => "The desktop said the code was not its own".to_string(),
                Error::Expired => "The code expired".to_string(),
                Error::Cancelled => "Pairing was cancelled on the desktop".to_string(),
                Error::Busy => "The desktop is busy; try again".to_string(),
                other => format!("Pairing failed: {other}"),
            };
            worker
                .shared
                .update(|v| v.pairing = Some(Pairing::Failed { reason }));
        }
        Ended::over(self.linked, e.to_string())
    }
}

/// Milliseconds on a monotonic clock: the machine's deadlines are relative,
/// and a headset's wall clock jumps when it syncs.
fn now() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn text(id: u32, status: u16, body: &str) -> Response {
    Response {
        id,
        status,
        headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
        body: body.as_bytes().to_vec(),
    }
}

/// Runs a tunnelled request against the debug runtime at `debug` over
/// loopback HTTP, so every route behaves exactly as it does locally.
/// `/stream` is refused: it never ends, and the link has no streaming
/// frames yet.
pub fn forward(debug: SocketAddr, request: &Request) -> Response {
    let path = request.target.split('?').next().unwrap_or("");
    if path == "/stream" {
        return text(request.id, 501, "/stream is not tunnelled yet");
    }
    let clean = |t: &str| !t.is_empty() && !t.contains([' ', '\r', '\n']);
    if !request.target.starts_with('/') || !clean(&request.target) || !clean(&request.method) {
        return text(request.id, 400, "bad request line");
    }
    exchange(debug, request).unwrap_or_else(|e| {
        text(
            request.id,
            502,
            &format!("the headset's debug runtime did not answer: {e}"),
        )
    })
}

fn exchange(debug: SocketAddr, request: &Request) -> io::Result<Response> {
    let mut stream = TcpStream::connect_timeout(&debug, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(FORWARD_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {debug}\r\nConnection: close\r\nContent-Length: {}\r\n",
        request.method,
        request.target,
        request.body.len()
    );
    for (name, value) in &request.headers {
        if FORWARDED.contains(&name.to_ascii_lowercase().as_str()) && !value.contains(['\r', '\n'])
        {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&request.body)?;
    let mut bytes = Vec::new();
    // One byte past the link's limit is enough to know it is too large.
    stream.take(MAX_FRAME as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FRAME {
        return Ok(text(
            request.id,
            502,
            "the response is too large for the link",
        ));
    }
    parse_response(request.id, &bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed response"))
}

/// An HTTP/1.1 response the debug runtime wrote (it always closes and sets
/// `Content-Length`).
fn parse_response(id: u32, bytes: &[u8]) -> Option<Response> {
    let split = bytes.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&bytes[..split]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let mut body = &bytes[split + 4..];
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':')?;
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_string());
        match name.as_str() {
            "content-length" => {
                let len: usize = value.parse().ok()?;
                body = body.get(..len)?;
            }
            "connection" | "transfer-encoding" => {}
            _ => headers.push((name, value)),
        }
    }
    Some(Response {
        id,
        status,
        headers,
        body: body.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// An HTTP server: reads each whole request (head, then `Content-Length`
    /// bytes, as the debug runtime does), hands it back, answers `reply`.
    /// Answering before the body is read would reset the connection on
    /// Windows, losing the reply.
    fn server(reply: &'static str) -> (SocketAddr, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut seen = Vec::new();
                let mut byte = [0u8; 1];
                while !seen.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    seen.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&seen).to_string();
                let len = head
                    .lines()
                    .find_map(|l| l.strip_prefix("Content-Length: "))
                    .map_or(0, |n| n.trim().parse().unwrap());
                let mut body = vec![0u8; len];
                stream.read_exact(&mut body).unwrap();
                let _ = tx.send(head + &String::from_utf8_lossy(&body));
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        (addr, rx)
    }

    fn request(method: &str, target: &str, headers: &[(&str, &str)], body: &[u8]) -> Request {
        Request {
            id: 7,
            method: method.into(),
            target: target.into(),
            headers: headers
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect(),
            body: body.to_vec(),
        }
    }

    #[test]
    fn forwards_a_request_and_its_answer() {
        let (addr, seen) = server(
            "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}trailing",
        );
        let r = request(
            "POST",
            "/time?x=1",
            &[
                ("content-type", "application/json"),
                ("cookie", "no"),
                ("origin", "http://127.0.0.1:3000"),
            ],
            b"{\"pause\":true}",
        );
        let response = forward(addr, &r);
        assert_eq!(response.id, 7);
        assert_eq!(response.status, 201);
        assert_eq!(response.body, b"{}");
        assert_eq!(
            response.headers,
            vec![("content-type".to_string(), "application/json".to_string())]
        );
        let head = seen.recv().unwrap();
        assert!(head.starts_with("POST /time?x=1 HTTP/1.1\r\n"), "{head}");
        assert!(head.contains("Content-Length: 14\r\n") && head.ends_with("{\"pause\":true}"));
        assert!(head.contains("origin: http://127.0.0.1:3000\r\n"));
        assert!(
            !head.contains("cookie"),
            "only the headers the runtime reads: {head}"
        );
    }

    #[test]
    fn stream_is_refused_and_a_missing_runtime_is_a_502() {
        let (addr, seen) = server("unused");
        assert_eq!(
            forward(addr, &request("GET", "/stream?w=2", &[], b"")).status,
            501
        );
        assert!(seen.try_recv().is_err(), "never forwarded");
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert_eq!(
            forward(closed, &request("GET", "/state", &[], b"")).status,
            502
        );
        assert_eq!(
            forward(addr, &request("GET", "state", &[], b"")).status,
            400
        );
    }

    #[test]
    fn a_garbled_answer_is_a_502() {
        let (addr, _seen) = server("not http");
        assert_eq!(forward(addr, &request("GET", "/", &[], b"")).status, 502);
    }

    #[test]
    fn the_store_makes_a_device_id_once_and_keeps_it() {
        let dir = std::env::temp_dir().join(format!("functor-link-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("device-link.json");
        let _ = std::fs::remove_file(&path);
        let first = Stored::load(&path).unwrap();
        assert!(first.device().is_some() && first.pairing().is_none());
        assert_eq!(Stored::load(&path).unwrap().device, first.device);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A pairing written from outside (the cable path) reads back.
        let written = format!(
            r#"{{"device":"{}","desktop":"{}","desktop_name":"Mac","secret":"{}"}}"#,
            first.device,
            "ab".repeat(16),
            "cd".repeat(32)
        );
        std::fs::write(&path, written).unwrap();
        let paired = Stored::load(&path).unwrap();
        assert_eq!(paired.pairing().unwrap().0, [0xab; 16]);
        // Garbage is moved aside, and the headset starts afresh.
        std::fs::write(&path, "{").unwrap();
        let fresh = Stored::load(&path).unwrap();
        assert!(fresh.device().is_some() && fresh.pairing().is_none());
        assert_eq!(
            std::fs::read_to_string(path.with_extension("bad")).unwrap(),
            "{"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
