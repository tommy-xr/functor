//! Headsets (D26, D27): the paired ones, each in one of a few connection
//! states with one fix, and the requests to pair. The shell runs the device
//! link and reports what happens as [`LinkEvent`]s; pairing secrets stay in
//! the shell, never in this state (which the page sees).
//!
//! Anything an unpaired peer says (a request to pair, an old version) is
//! unauthenticated: it never changes a paired device's name, `last_seen`
//! or a live link's state.

use serde::{Deserialize, Serialize};

use crate::panel::{Panel, PanelAction};

/// A paired device whose link failed this long ago is away, not failing.
pub const RECENT_MS: u64 = 120_000;
/// Requests and unpaired cards shown at most (the shell caps them lower).
const MAX_UNPAIRED: usize = 8;

/// The oldest headset app this desktop works with. Local and CI builds
/// report `0.0.0` and are always accepted.
pub const MIN_HEADSET_APP: (u64, u64, u64) = (0, 1, 0);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Devices {
    /// Where the device link listens (`host:port`); `None` when it is off.
    pub listening: Option<String>,
    /// Paired devices, and unpaired ones whose app is too old to pair, by
    /// name then id.
    pub list: Vec<Device>,
    /// Devices asking to pair, oldest first: the prompt shows one at a time.
    pub requests: Vec<PairRequest>,
    /// The title bar's one word for them all.
    pub headset: Headset,
    /// `adb` runs here, so headsets on a USB cable are seen.
    pub adb: bool,
    /// Headsets on a USB cable, as adb lists them.
    pub attached: Vec<Attached>,
    /// The link has said what is paired ([`LinkEvent::Started`]).
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub started: bool,
}

/// A headset on a USB cable (task 2.4). Pairing over it needs no code: the
/// cable and the user's adb authorisation are the trust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Attached {
    /// adb's serial for it.
    pub serial: String,
    pub name: String,
    pub state: CableState,
    /// Its pairing is being written.
    pub pairing: bool,
    /// "Pair over the cable" is offered ([`cable_pairable`]).
    pub pairable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CableState {
    /// It has not allowed USB debugging from this computer yet.
    Unauthorized,
    /// adb lists it but cannot reach it.
    Offline,
    /// The Functor app is not installed on it.
    NoApp,
    /// Its Functor app is a release build, whose files adb cannot reach.
    NotDebuggable,
    /// Its id (`None` until its app first runs), and the pairing its app
    /// holds.
    Ready {
        device: Option<String>,
        paired: HeadsetPairing,
    },
}

/// The pairing a headset's app holds, as its file says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "with", rename_all = "snake_case")]
pub enum HeadsetPairing {
    Nobody,
    /// This desktop.
    This,
    /// Another desktop, by the name it stored.
    Other {
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Device {
    /// The device's own random id, hex.
    pub id: String,
    pub name: String,
    /// A pairing is stored for it.
    pub paired: bool,
    pub state: DeviceState,
    /// The headset app's version, as it last said over its link.
    pub app_version: Option<String>,
    /// Wall-clock ms, from its links.
    pub first_seen: Option<u64>,
    pub last_seen: Option<u64>,
    /// Its debug runtime through the link: `http://127.0.0.1:<port>`, kept
    /// while paired (it answers 503 while the device is away).
    pub tunnel: Option<String>,
    /// The shell's connection carrying its link.
    #[serde(skip)]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub conn: Option<u64>,
}

/// A device asking to pair (unauthenticated: its id and name are claims).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PairRequest {
    pub request: u64,
    /// The id it claims, hex.
    pub device: String,
    pub name: String,
    pub app_version: String,
    /// Both sides show it.
    pub code: String,
    /// Wall-clock ms.
    pub expires_at: u64,
    /// The user allowed it; the headset's own confirmation is awaited.
    pub allowed: bool,
    /// It claims the id of a device already paired.
    pub repair: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DeviceState {
    /// Paired, not connected: off, asleep or elsewhere.
    Away,
    Connected {
        link: LinkKind,
        /// Percent.
        battery: Option<u8>,
        charging: bool,
        /// The runtime build it runs.
        build: Option<String>,
        /// The project it runs.
        project: Option<String>,
        /// The last ping's round trip.
        latency_ms: Option<u64>,
    },
    /// It cannot work with this app until one of them is updated (seen at
    /// `since`).
    NeedsUpdate {
        update: Side,
        reason: String,
        since: u64,
    },
    /// Paired, but its link just failed (at `since`).
    CannotReach { reason: String, since: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Wifi,
    /// Over adb: a second way to link (task 2.4).
    Cable,
}

/// Which app needs the update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Headset,
    Desktop,
}

/// The state word for all devices: the one that most needs the user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Headset {
    #[default]
    NotPaired,
    AskingToPair,
    Away,
    Connected,
    NeedsUpdate,
    CannotReach,
}

/// A stored pairing, as the shell reads it (without its secret).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paired {
    pub id: String,
    pub name: String,
    pub first_seen: u64,
    pub last_seen: u64,
}

/// From the shell's device link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkEvent {
    /// The link is listening at `listening` (`None`: turned off), with the
    /// stored pairings.
    Started {
        listening: Option<String>,
        paired: Vec<Paired>,
    },
    /// A device asks to pair; both sides show `code`.
    PairRequest {
        request: u64,
        device: String,
        name: String,
        app_version: String,
        code: String,
        expires_at: u64,
    },
    /// Request `request` was allowed by both users and its pairing stored.
    Paired {
        request: u64,
        device: Paired,
    },
    /// Request `request` ended unpaired: denied, expired, cancelled or dropped.
    PairEnded {
        request: u64,
    },
    /// Connection `conn` authenticated as paired `device`.
    Connected {
        conn: u64,
        device: String,
        name: String,
        app_version: String,
        link: LinkKind,
    },
    Status {
        conn: u64,
        battery: Option<u8>,
        charging: bool,
        build: String,
        project: Option<String>,
    },
    Latency {
        conn: u64,
        ms: u64,
    },
    /// `conn` closed: cleanly (`lost: None`) or with why it failed.
    Disconnected {
        conn: u64,
        lost: Option<String>,
    },
    /// A device speaking link protocol `theirs` (we speak `ours`) was refused.
    Mismatch {
        device: String,
        name: String,
        theirs: u16,
        ours: u16,
    },
    /// Paired `device`'s debug runtime is reachable at `url`.
    TunnelOpened {
        device: String,
        url: String,
    },
    /// `device`'s pairing is deleted: its card goes.
    Forgotten {
        device: String,
    },
    /// Something the user asked for failed; every page is told.
    Failed {
        reason: String,
    },
    /// About once a second: states that age.
    Tick,
    /// What adb sees now: whether it runs, and the headsets on a cable.
    Cable {
        adb: bool,
        attached: Vec<Attached>,
    },
    /// A pairing over `serial`'s cable is stored on both sides and read
    /// back from the headset, or why not (nothing is left stored then).
    CablePaired {
        serial: String,
        result: Result<Paired, String>,
    },
    /// Linked connection `conn`'s headset panel asks for `action`.
    Panel {
        conn: u64,
        action: PanelAction,
    },
    /// `conn`'s headset said `text` (transcribed), as its utterance
    /// `utterance`; `context` is its capsule, rendered.
    Utterance {
        conn: u64,
        utterance: u64,
        text: String,
        context: Option<String>,
    },
}

/// To the shell's device link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkCommand {
    Allow {
        request: u64,
    },
    Deny {
        request: u64,
    },
    /// Delete its pairing (the secret with it), close its link and tunnel.
    Forget {
        device: String,
    },
    /// Serve `device`'s debug runtime on a loopback port.
    OpenTunnel {
        device: String,
    },
    /// Pair the headset on `serial`'s cable: a new secret, stored here and
    /// written to it. One paired with another desktop is replaced only
    /// with `replace`.
    PairByCable {
        serial: String,
        replace: bool,
    },
    /// Send the headset panel on connection `conn` its content.
    Panel { conn: u64, panel: Box<Panel> },
    /// Tell the panel on `conn` its last action was refused.
    PanelRefused { conn: u64, reason: String },
    /// Tell `device` how its utterance is going.
    Voice {
        device: String,
        utterance: u64,
        stage: crate::voice::Stage,
    },
}

impl Devices {
    fn find(&mut self, id: &str) -> Option<&mut Device> {
        self.list.iter_mut().find(|d| d.id == id)
    }

    fn by_conn(&mut self, conn: u64) -> Option<&mut Device> {
        self.list.iter_mut().find(|d| d.conn == Some(conn))
    }

    fn insert(&mut self, device: Device) {
        self.list.push(device);
        self.list
            .sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
    }

    fn unpaired(&self) -> usize {
        self.requests.len() + self.list.iter().filter(|d| !d.paired).count()
    }

    fn end_request(&mut self, request: u64) -> bool {
        let before = self.requests.len();
        self.requests.retain(|r| r.request != request);
        self.requests.len() != before
    }

    fn insert_paired(&mut self, p: Paired) {
        self.insert(Device {
            first_seen: Some(p.first_seen),
            last_seen: Some(p.last_seen),
            ..card(p.id, p.name, true, DeviceState::Away)
        });
    }

    /// A pairing stored by either way in: its card, and its tunnel.
    fn paired(&mut self, p: Paired, commands: &mut Vec<LinkCommand>) {
        commands.push(LinkCommand::OpenTunnel {
            device: p.id.clone(),
        });
        match self.find(&p.id) {
            Some(d) => {
                d.paired = true;
                d.name = p.name;
                d.first_seen = Some(p.first_seen);
                d.last_seen = Some(p.last_seen);
            }
            None => self.insert_paired(p),
        }
    }
}

/// Whether the headset on this cable can be paired over it now: its app
/// is reachable, nothing is being written, and it is not already this
/// desktop's on both sides.
pub fn cable_pairable(devices: &Devices, a: &Attached) -> bool {
    let CableState::Ready { device, paired } = &a.state else {
        return false;
    };
    let ours = devices
        .list
        .iter()
        .any(|d| d.paired && Some(&d.id) == device.as_ref());
    !a.pairing && !(ours && *paired == HeadsetPairing::This)
}

/// Pair the headset on `serial`'s cable. One paired with another desktop
/// needs `replace`: the user was asked first.
pub fn pair_by_cable(
    devices: &mut Devices,
    serial: &str,
    replace: bool,
) -> Result<LinkCommand, String> {
    let Some(a) = devices
        .attached
        .iter()
        .find(|a| a.serial == serial && cable_pairable(devices, a))
    else {
        return Err("that headset cannot be paired over the cable now".to_string());
    };
    if let CableState::Ready {
        paired: HeadsetPairing::Other { name },
        ..
    } = &a.state
    {
        if !replace {
            return Err(format!("{} is paired with {name}", a.name));
        }
    }
    if let Some(a) = devices.attached.iter_mut().find(|a| a.serial == serial) {
        a.pairing = true;
    }
    refresh(devices);
    Ok(LinkCommand::PairByCable {
        serial: serial.to_string(),
        replace,
    })
}

fn card(id: String, name: String, paired: bool, state: DeviceState) -> Device {
    Device {
        id,
        name,
        paired,
        state,
        app_version: None,
        first_seen: None,
        last_seen: None,
        tunnel: None,
        conn: None,
    }
}

/// `version` (`major.minor.patch`) is older than [`MIN_HEADSET_APP`];
/// unparsable and `0.0.0` (a local build) are not.
pub fn too_old(version: &str) -> bool {
    let mut parts = version.split('.').map(|p| p.parse::<u64>());
    let parsed = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Ok(a)), Some(Ok(b)), Some(Ok(c)), None) => (a, b, c),
        _ => return false,
    };
    parsed != (0, 0, 0) && parsed < MIN_HEADSET_APP
}

/// The title-bar word: whatever most needs the user.
pub fn headset(devices: &Devices) -> Headset {
    let has = |f: fn(&DeviceState) -> bool| devices.list.iter().any(|d| f(&d.state));
    if !devices.requests.is_empty() {
        Headset::AskingToPair
    } else if has(|s| matches!(s, DeviceState::Connected { .. })) {
        Headset::Connected
    } else if has(|s| matches!(s, DeviceState::NeedsUpdate { .. })) {
        Headset::NeedsUpdate
    } else if has(|s| matches!(s, DeviceState::CannotReach { .. })) {
        Headset::CannotReach
    } else if devices.list.iter().any(|d| d.paired) {
        Headset::Away
    } else {
        Headset::NotPaired
    }
}

/// Steps the devices on a shell event at wall time `now`; whether they
/// changed, and what the shell must do.
pub fn on_link(devices: &mut Devices, event: LinkEvent, now: u64) -> (bool, Vec<LinkCommand>) {
    let before = devices.clone();
    let mut commands = Vec::new();
    match event {
        LinkEvent::Started { listening, paired } => {
            devices.started = true;
            devices.listening = listening;
            devices.list.clear();
            devices.requests.clear();
            for p in paired {
                commands.push(LinkCommand::OpenTunnel {
                    device: p.id.clone(),
                });
                devices.insert_paired(p);
            }
        }
        LinkEvent::PairRequest {
            request,
            device,
            name,
            app_version,
            code,
            expires_at,
        } => {
            // The shell caps unpaired peers; this is a backstop.
            if devices.unpaired() >= MAX_UNPAIRED {
                commands.push(LinkCommand::Deny { request });
            } else {
                let repair = devices.list.iter().any(|d| d.paired && d.id == device);
                devices.requests.push(PairRequest {
                    request,
                    device,
                    name,
                    app_version,
                    code,
                    expires_at,
                    allowed: false,
                    repair,
                });
            }
        }
        LinkEvent::Paired { request, device } => {
            devices.end_request(request);
            devices.paired(device, &mut commands);
        }
        LinkEvent::PairEnded { request } => {
            devices.end_request(request);
        }
        LinkEvent::Connected {
            conn,
            device,
            name,
            app_version,
            link,
        } => {
            if let Some(d) = devices.find(&device).filter(|d| d.paired) {
                d.state = match too_old(&app_version) {
                    true => DeviceState::NeedsUpdate {
                        update: Side::Headset,
                        reason: format!("its app is {app_version}; this desktop needs a newer one"),
                        since: now,
                    },
                    false => DeviceState::Connected {
                        link,
                        battery: None,
                        charging: false,
                        build: None,
                        project: None,
                        latency_ms: None,
                    },
                };
                d.name = name;
                d.app_version = Some(app_version);
                d.conn = Some(conn);
                d.last_seen = Some(now);
            }
        }
        LinkEvent::Status {
            conn,
            battery,
            charging,
            build,
            project,
        } => {
            if let Some(d) = devices.by_conn(conn) {
                d.last_seen = Some(now);
                if let DeviceState::Connected {
                    battery: b,
                    charging: c,
                    build: v,
                    project: p,
                    ..
                } = &mut d.state
                {
                    (*b, *c, *v, *p) = (battery, charging, Some(build), project);
                }
            }
        }
        LinkEvent::Latency { conn, ms } => {
            if let Some(DeviceState::Connected { latency_ms, .. }) =
                devices.by_conn(conn).map(|d| &mut d.state)
            {
                *latency_ms = Some(ms);
            }
        }
        LinkEvent::Disconnected { conn, lost } => {
            if let Some(d) = devices.by_conn(conn) {
                d.conn = None;
                d.last_seen = Some(now);
                // An old app stays flagged until it comes back updated.
                if !matches!(d.state, DeviceState::NeedsUpdate { .. }) {
                    d.state = match lost {
                        None => DeviceState::Away,
                        Some(reason) => DeviceState::CannotReach { reason, since: now },
                    };
                }
            }
        }
        LinkEvent::Mismatch {
            device,
            name,
            theirs,
            ours,
        } => {
            let (update, older) = match theirs < ours {
                true => (Side::Headset, "the headset's"),
                false => (Side::Desktop, "this app's"),
            };
            let state = DeviceState::NeedsUpdate {
                update,
                reason: format!("link protocol {theirs} against {ours}: {older} is older"),
                since: now,
            };
            let room = devices.unpaired() < MAX_UNPAIRED;
            match devices.find(&device) {
                // A live link proves more than this unauthenticated hello.
                Some(d) if d.conn.is_some() => {}
                // Only the state: it ages back to away unless the claim repeats.
                Some(d) => d.state = state,
                None if room => devices.insert(card(device, name, false, state)),
                None => {}
            }
        }
        LinkEvent::TunnelOpened { device, url } => {
            if let Some(d) = devices.find(&device).filter(|d| d.paired) {
                d.tunnel = Some(url);
            }
        }
        LinkEvent::Forgotten { device } => {
            devices.list.retain(|d| !(d.paired && d.id == device));
        }
        // Nothing changes here; `update` tells the pages.
        LinkEvent::Failed { .. } => {}
        LinkEvent::Tick => {
            for d in &mut devices.list {
                let since = match d.state {
                    DeviceState::CannotReach { since, .. } => since,
                    DeviceState::NeedsUpdate { since, .. } if d.conn.is_none() => since,
                    _ => continue,
                };
                if now.saturating_sub(since) >= RECENT_MS {
                    d.state = DeviceState::Away;
                }
            }
            // An unpaired device's old-app card goes when it ages out.
            devices
                .list
                .retain(|d| d.paired || !matches!(d.state, DeviceState::Away));
        }
        LinkEvent::Cable { adb, attached } => {
            // A pairing being written stays so until its outcome.
            let busy: Vec<String> = devices
                .attached
                .iter()
                .filter(|a| a.pairing)
                .map(|a| a.serial.clone())
                .collect();
            devices.adb = adb;
            devices.attached = attached;
            for a in &mut devices.attached {
                a.pairing = busy.contains(&a.serial);
            }
        }
        LinkEvent::CablePaired { serial, result } => {
            if let Some(a) = devices.attached.iter_mut().find(|a| a.serial == serial) {
                a.pairing = false;
                if let (Ok(_), CableState::Ready { paired, .. }) = (&result, &mut a.state) {
                    *paired = HeadsetPairing::This;
                }
            }
            if let Ok(device) = result {
                devices.paired(device, &mut commands);
            }
        }
        // The core turns these into client messages before they get here.
        LinkEvent::Panel { .. } => {}
        // The core's to act on (`voice::heard`): no device changes.
        LinkEvent::Utterance { .. } => {}
    }
    refresh(devices);
    (*devices != before, commands)
}

/// The state derived from the rest: the title word, what can pair by cable.
fn refresh(devices: &mut Devices) {
    devices.headset = headset(devices);
    let pairable: Vec<bool> = devices
        .attached
        .iter()
        .map(|a| cable_pairable(devices, a))
        .collect();
    for (a, p) in devices.attached.iter_mut().zip(pairable) {
        a.pairable = p;
    }
}

/// Allow request `request`, unless it has ended or expired; it then waits
/// for the headset's own confirmation if that has not come yet.
pub fn allow(devices: &mut Devices, request: u64, now: u64) -> Result<LinkCommand, String> {
    match devices
        .requests
        .iter_mut()
        .find(|r| r.request == request && now < r.expires_at)
    {
        Some(r) => {
            r.allowed = true;
            Ok(LinkCommand::Allow { request })
        }
        None => Err("that pairing request has ended".to_string()),
    }
}

/// Deny request `request`: its prompt goes now.
pub fn deny(devices: &mut Devices, request: u64) -> Result<LinkCommand, String> {
    match devices.end_request(request) {
        true => {
            devices.headset = headset(devices);
            Ok(LinkCommand::Deny { request })
        }
        false => Err("that pairing request has ended".to_string()),
    }
}

/// Forget paired `device`. Its card stays until the shell has deleted the
/// pairing (`LinkEvent::Forgotten`), so a failed write leaves it as it was.
pub fn forget(devices: &Devices, device: &str) -> Result<LinkCommand, String> {
    match devices.list.iter().any(|d| d.paired && d.id == device) {
        true => Ok(LinkCommand::Forget {
            device: device.to_string(),
        }),
        false => Err(format!("no paired device {device}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(ids: &[&str]) -> Devices {
        let mut d = Devices::default();
        let paired = ids
            .iter()
            .map(|id| Paired {
                id: id.to_string(),
                name: format!("Quest {id}"),
                first_seen: 1,
                last_seen: 2,
            })
            .collect();
        let (_, cmds) = on_link(
            &mut d,
            LinkEvent::Started {
                listening: Some("0.0.0.0:4000".into()),
                paired,
            },
            10,
        );
        assert_eq!(cmds.len(), ids.len());
        d
    }

    fn ask(d: &mut Devices, request: u64, device: &str, now: u64) -> Vec<LinkCommand> {
        on_link(
            d,
            LinkEvent::PairRequest {
                request,
                device: device.into(),
                name: "Stranger".into(),
                app_version: "0.1.0".into(),
                code: "123456".into(),
                expires_at: now + 120_000,
            },
            now,
        )
        .1
    }

    fn connect(d: &mut Devices, conn: u64, device: &str, app: &str, now: u64) {
        on_link(
            d,
            LinkEvent::Connected {
                conn,
                device: device.into(),
                name: "Quest 3".into(),
                app_version: app.into(),
                link: LinkKind::Wifi,
            },
            now,
        );
    }

    fn state(d: &Devices, id: &str) -> DeviceState {
        d.list.iter().find(|x| x.id == id).unwrap().state.clone()
    }

    fn paired(id: &str) -> Paired {
        Paired {
            id: id.into(),
            name: "Quest 3".into(),
            first_seen: 150,
            last_seen: 150,
        }
    }

    #[test]
    fn nothing_paired_is_not_paired_and_stored_ones_start_away() {
        assert_eq!(headset(&Devices::default()), Headset::NotPaired);
        let d = started(&["a", "b"]);
        assert_eq!(d.headset, Headset::Away);
        assert!(d
            .list
            .iter()
            .all(|x| x.paired && x.state == DeviceState::Away));
        assert_eq!(d.listening.as_deref(), Some("0.0.0.0:4000"));
    }

    #[test]
    fn asking_to_pair_then_allowed_becomes_paired_and_away() {
        let mut d = started(&[]);
        ask(&mut d, 7, "new", 100);
        assert_eq!(d.headset, Headset::AskingToPair);
        assert_eq!(d.requests[0].request, 7);
        assert!(d.list.is_empty());
        assert_eq!(allow(&mut d, 7, 100), Ok(LinkCommand::Allow { request: 7 }));
        // Waiting for the headset's own confirmation.
        assert!(d.requests[0].allowed);
        assert!(allow(&mut d, 8, 100).is_err());
        // Expired by the desktop's clock.
        assert!(allow(&mut d, 7, 100 + 120_000).is_err());
        let (changed, cmds) = on_link(
            &mut d,
            LinkEvent::Paired {
                request: 7,
                device: paired("new"),
            },
            150,
        );
        assert!(changed);
        assert_eq!(
            cmds,
            [LinkCommand::OpenTunnel {
                device: "new".into()
            }]
        );
        assert_eq!(state(&d, "new"), DeviceState::Away);
        assert!(d.list[0].paired && d.requests.is_empty());
        assert_eq!(d.headset, Headset::Away);
    }

    #[test]
    fn a_request_that_ends_or_is_denied_leaves_nothing() {
        let mut d = started(&[]);
        ask(&mut d, 1, "x", 0);
        on_link(&mut d, LinkEvent::PairEnded { request: 1 }, 5);
        assert!(d.requests.is_empty());
        assert_eq!(d.headset, Headset::NotPaired);
        ask(&mut d, 2, "x", 0);
        assert_eq!(deny(&mut d, 2), Ok(LinkCommand::Deny { request: 2 }));
        assert!(d.requests.is_empty() && d.list.is_empty());
        assert!(deny(&mut d, 2).is_err());
    }

    #[test]
    fn a_request_claiming_a_paired_id_never_touches_its_card() {
        let mut d = started(&["a"]);
        connect(&mut d, 1, "a", "0.1.0", 5);
        let card = d.list[0].clone();
        ask(&mut d, 3, "a", 6);
        assert!(d.requests[0].repair);
        assert_eq!(d.list[0], card);
        on_link(&mut d, LinkEvent::PairEnded { request: 3 }, 7);
        assert_eq!(d.list[0], card);
        // The live link still updates.
        on_link(&mut d, LinkEvent::Latency { conn: 1, ms: 9 }, 8);
        assert!(matches!(
            state(&d, "a"),
            DeviceState::Connected {
                latency_ms: Some(9),
                ..
            }
        ));
    }

    #[test]
    fn requests_from_one_id_queue_and_never_cancel_each_other() {
        let mut d = started(&[]);
        assert!(ask(&mut d, 1, "x", 0).is_empty());
        assert!(ask(&mut d, 2, "x", 1).is_empty());
        let queued: Vec<u64> = d.requests.iter().map(|r| r.request).collect();
        assert_eq!(queued, [1, 2]);
    }

    #[test]
    fn unpaired_requests_are_capped() {
        let mut d = started(&[]);
        for i in 0..MAX_UNPAIRED as u64 {
            assert!(ask(&mut d, i, &format!("d{i}"), 0).is_empty());
        }
        assert_eq!(
            ask(&mut d, 99, "extra", 0),
            [LinkCommand::Deny { request: 99 }]
        );
        assert_eq!(d.requests.len(), MAX_UNPAIRED);
    }

    #[test]
    fn connected_takes_status_and_latency_then_a_clean_close_is_away() {
        let mut d = started(&["a"]);
        connect(&mut d, 9, "a", "0.1.0", 1000);
        assert_eq!(d.headset, Headset::Connected);
        on_link(
            &mut d,
            LinkEvent::Status {
                conn: 9,
                battery: Some(80),
                charging: true,
                build: "abc".into(),
                project: Some("examples/hello".into()),
            },
            1005,
        );
        on_link(&mut d, LinkEvent::Latency { conn: 9, ms: 12 }, 1006);
        assert_eq!(
            state(&d, "a"),
            DeviceState::Connected {
                link: LinkKind::Wifi,
                battery: Some(80),
                charging: true,
                build: Some("abc".into()),
                project: Some("examples/hello".into()),
                latency_ms: Some(12),
            }
        );
        // Another connection's news changes nothing.
        let (changed, _) = on_link(&mut d, LinkEvent::Latency { conn: 8, ms: 1 }, 1007);
        assert!(!changed);
        on_link(
            &mut d,
            LinkEvent::Disconnected {
                conn: 9,
                lost: None,
            },
            2000,
        );
        assert_eq!(state(&d, "a"), DeviceState::Away);
        assert_eq!(d.list[0].last_seen, Some(2000));
    }

    #[test]
    fn an_unpaired_connection_is_ignored() {
        let mut d = started(&[]);
        ask(&mut d, 1, "x", 0);
        connect(&mut d, 5, "x", "0.1.0", 1);
        assert!(d.list.is_empty());
    }

    #[test]
    fn a_lost_link_cannot_reach_until_it_ages_to_away() {
        let mut d = started(&["a"]);
        connect(&mut d, 1, "a", "0.1.0", 1000);
        on_link(
            &mut d,
            LinkEvent::Disconnected {
                conn: 1,
                lost: Some("timed out".into()),
            },
            2000,
        );
        assert_eq!(
            state(&d, "a"),
            DeviceState::CannotReach {
                reason: "timed out".into(),
                since: 2000
            }
        );
        assert_eq!(d.headset, Headset::CannotReach);
        on_link(&mut d, LinkEvent::Tick, 2000 + RECENT_MS - 1);
        assert!(matches!(state(&d, "a"), DeviceState::CannotReach { .. }));
        on_link(&mut d, LinkEvent::Tick, 2000 + RECENT_MS);
        assert_eq!(state(&d, "a"), DeviceState::Away);
        // Reconnecting fixes it at once.
        connect(&mut d, 2, "a", "0.1.0", 3000);
        assert!(matches!(state(&d, "a"), DeviceState::Connected { .. }));
    }

    #[test]
    fn needs_an_update_for_another_protocol_or_an_old_app() {
        let mismatch = |theirs| LinkEvent::Mismatch {
            device: "a".into(),
            name: "Impostor".into(),
            theirs,
            ours: 2,
        };
        let mut d = started(&["a"]);
        on_link(&mut d, mismatch(1), 10);
        assert!(matches!(
            state(&d, "a"),
            DeviceState::NeedsUpdate {
                update: Side::Headset,
                ..
            }
        ));
        assert_eq!(d.headset, Headset::NeedsUpdate);
        // An unauthenticated claim changes neither name nor last seen.
        assert_eq!(
            (d.list[0].name.as_str(), d.list[0].last_seen),
            ("Quest a", Some(2))
        );
        on_link(&mut d, mismatch(3), 11);
        assert!(matches!(
            state(&d, "a"),
            DeviceState::NeedsUpdate {
                update: Side::Desktop,
                ..
            }
        ));
        // It ages back to away unless the claim repeats.
        on_link(&mut d, LinkEvent::Tick, 11 + RECENT_MS);
        assert_eq!(state(&d, "a"), DeviceState::Away);
        // An unknown device gets a card, which ages out.
        let mut u = started(&[]);
        on_link(
            &mut u,
            LinkEvent::Mismatch {
                device: "z".into(),
                name: "Quest".into(),
                theirs: 1,
                ours: 2,
            },
            10,
        );
        assert!(!u.list[0].paired);
        on_link(&mut u, LinkEvent::Tick, 10 + RECENT_MS);
        assert!(u.list.is_empty());
        // A live link wins over an unauthenticated mismatch.
        let mut l = started(&["a"]);
        connect(&mut l, 1, "a", "0.1.0", 5);
        on_link(&mut l, mismatch(1), 6);
        assert!(matches!(state(&l, "a"), DeviceState::Connected { .. }));
        // An old app links but needs an update; a local build does not.
        let mut o = started(&["a"]);
        connect(&mut o, 1, "a", "0.0.9", 5);
        assert!(matches!(
            state(&o, "a"),
            DeviceState::NeedsUpdate {
                update: Side::Headset,
                ..
            }
        ));
        connect(&mut o, 2, "a", "0.0.0", 6);
        assert!(matches!(state(&o, "a"), DeviceState::Connected { .. }));
    }

    #[test]
    fn version_rule() {
        assert!(too_old("0.0.9"));
        assert!(!too_old("0.0.0"));
        assert!(!too_old("0.1.0"));
        assert!(!too_old("1.0.0"));
        assert!(!too_old("dev"));
        assert!(!too_old("0.1"));
    }

    #[test]
    fn forget_removes_a_paired_device_once_the_shell_has() {
        let mut d = started(&["a"]);
        connect(&mut d, 1, "a", "0.1.0", 0);
        assert_eq!(
            forget(&d, "a"),
            Ok(LinkCommand::Forget { device: "a".into() })
        );
        // Until the shell says so (a failed write keeps it), the card stays whole.
        assert!(matches!(state(&d, "a"), DeviceState::Connected { .. }));
        on_link(&mut d, LinkEvent::Forgotten { device: "a".into() }, 5);
        assert!(d.list.is_empty());
        assert_eq!(d.headset, Headset::NotPaired);
        assert!(forget(&d, "a").is_err());
        // Not a request.
        ask(&mut d, 1, "x", 0);
        assert!(forget(&d, "x").is_err());
    }

    #[test]
    fn the_tunnel_is_kept_for_paired_devices() {
        let mut d = started(&["a"]);
        on_link(
            &mut d,
            LinkEvent::TunnelOpened {
                device: "a".into(),
                url: "http://127.0.0.1:5000".into(),
            },
            1,
        );
        assert_eq!(d.list[0].tunnel.as_deref(), Some("http://127.0.0.1:5000"));
    }

    fn cable(d: &mut Devices, attached: Vec<Attached>) {
        on_link(
            d,
            LinkEvent::Cable {
                adb: true,
                attached,
            },
            0,
        );
    }

    fn on_cable(serial: &str, state: CableState) -> Attached {
        Attached {
            serial: serial.into(),
            name: "Quest 3".into(),
            state,
            pairing: false,
            pairable: false,
        }
    }

    fn ready(device: &str, paired: HeadsetPairing) -> CableState {
        CableState::Ready {
            device: Some(device.into()),
            paired,
        }
    }

    #[test]
    fn only_a_reachable_headset_not_already_ours_pairs_over_the_cable() {
        let mut d = started(&["a"]);
        cable(
            &mut d,
            vec![
                on_cable("s1", CableState::Unauthorized),
                on_cable("s2", CableState::Offline),
                on_cable("s3", CableState::NoApp),
                on_cable("s4", CableState::NotDebuggable),
                on_cable("s5", ready("a", HeadsetPairing::This)),
                on_cable("s6", ready("a", HeadsetPairing::Nobody)),
                on_cable("s7", ready("b", HeadsetPairing::This)),
            ],
        );
        assert!(d.adb);
        for serial in ["s1", "s2", "s3", "s4", "s5"] {
            assert!(pair_by_cable(&mut d, serial, false).is_err(), "{serial}");
        }
        // Paired here but its pairing gone from the headset; or the
        // headset names us but this desktop lost it: either pairs again.
        for serial in ["s6", "s7"] {
            assert_eq!(
                pair_by_cable(&mut d, serial, false),
                Ok(LinkCommand::PairByCable {
                    serial: serial.into(),
                    replace: false
                })
            );
        }
        assert!(pair_by_cable(&mut d, "gone", false).is_err());
    }

    #[test]
    fn a_headset_paired_elsewhere_is_replaced_only_when_asked() {
        let mut d = started(&[]);
        let other = HeadsetPairing::Other {
            name: "Studio Mac".into(),
        };
        cable(&mut d, vec![on_cable("s", ready("x", other))]);
        assert_eq!(
            pair_by_cable(&mut d, "s", false),
            Err("Quest 3 is paired with Studio Mac".into())
        );
        assert!(!d.attached[0].pairing);
        assert!(pair_by_cable(&mut d, "s", true).is_ok());
    }

    #[test]
    fn cable_pairing_is_one_at_a_time_and_its_outcome_adds_the_card() {
        let mut d = started(&[]);
        cable(
            &mut d,
            vec![on_cable("s", ready("x", HeadsetPairing::Nobody))],
        );
        assert!(d.attached[0].pairable);
        assert!(pair_by_cable(&mut d, "s", false).is_ok());
        assert!(!d.attached[0].pairable);
        assert!(
            pair_by_cable(&mut d, "s", false).is_err(),
            "already writing"
        );
        // adb's next look keeps it marked as being written.
        cable(
            &mut d,
            vec![on_cable("s", ready("x", HeadsetPairing::Nobody))],
        );
        assert!(d.attached[0].pairing);
        let (_, cmds) = on_link(
            &mut d,
            LinkEvent::CablePaired {
                serial: "s".into(),
                result: Ok(paired("x")),
            },
            10,
        );
        assert_eq!(cmds, [LinkCommand::OpenTunnel { device: "x".into() }]);
        assert!(!d.attached[0].pairing);
        assert_eq!(d.attached[0].state, ready("x", HeadsetPairing::This));
        assert_eq!(state(&d, "x"), DeviceState::Away);
        assert_eq!(d.headset, Headset::Away);
        // Then it links over Wi-Fi as any paired headset does.
        connect(&mut d, 3, "x", "0.1.0", 20);
        assert!(matches!(state(&d, "x"), DeviceState::Connected { .. }));
        assert!(!d.attached[0].pairable);
        assert!(pair_by_cable(&mut d, "s", false).is_err(), "ours already");
        // Forgotten here: it can pair again.
        on_link(&mut d, LinkEvent::Forgotten { device: "x".into() }, 30);
        assert!(d.attached[0].pairable);
    }

    #[test]
    fn a_failed_cable_pairing_leaves_no_card_and_can_be_tried_again() {
        let mut d = started(&[]);
        cable(
            &mut d,
            vec![on_cable("s", ready("x", HeadsetPairing::Nobody))],
        );
        pair_by_cable(&mut d, "s", false).unwrap();
        let (_, cmds) = on_link(
            &mut d,
            LinkEvent::CablePaired {
                serial: "s".into(),
                result: Err("read back a different file".into()),
            },
            10,
        );
        assert!(cmds.is_empty());
        assert!(d.list.is_empty() && !d.attached[0].pairing);
        assert!(pair_by_cable(&mut d, "s", false).is_ok());
    }

    #[test]
    fn unplugged_headsets_go_and_no_adb_shows_none() {
        let mut d = started(&[]);
        cable(&mut d, vec![on_cable("s", CableState::NoApp)]);
        let (changed, _) = on_link(
            &mut d,
            LinkEvent::Cable {
                adb: false,
                attached: vec![],
            },
            0,
        );
        assert!(changed && !d.adb && d.attached.is_empty());
        assert_eq!(d.headset, Headset::NotPaired);
    }

    #[test]
    fn headset_word_prefers_what_needs_the_user() {
        let mut d = started(&["a", "b"]);
        connect(&mut d, 1, "a", "0.1.0", 0);
        assert_eq!(d.headset, Headset::Connected);
        ask(&mut d, 5, "c", 0);
        assert_eq!(d.headset, Headset::AskingToPair);
    }
}
