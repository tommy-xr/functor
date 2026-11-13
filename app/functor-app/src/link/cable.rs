//! Headsets on a USB cable (task 2.4, D26). While the device link listens,
//! adb is asked every [`EVERY`] what is attached, and each headset's app is
//! probed for its id and the pairing it holds. Pairing over the cable
//! needs no code (the cable and the user's adb authorisation are the
//! trust) and only ever follows a click: a headset plugged in is never
//! paired by itself.
//!
//! The secret travels only on adb's stdin and stdout, to and from the
//! app's private storage (`run-as`): never in a command line or a log.

use std::io;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::Duration;

use functor_app_core::devices::{Attached, CableState, HeadsetPairing, LinkEvent, Paired};
use functor_device_link::adb::{self, Entry, READ_PAIRING, WRITE_PAIRING};
use functor_device_link::file::Stored;
use functor_device_link::{hex, os_random, Secret};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{ConnCmd, Hub};

/// How often adb is asked what is attached.
const EVERY: Duration = Duration::from_secs(2);
/// Headsets' apps are probed on every change adb lists (a replug too), after a pairing or
/// Forget, and otherwise every this many looks.
const PROBE_EVERY: u32 = 5;
/// One adb command's limit.
const TIMEOUT: Duration = Duration::from_secs(15);

/// One adb command's outcome.
struct Ran {
    ok: bool,
    stdout: Vec<u8>,
    stderr: String,
}

impl Hub {
    /// Runs adb with `args`, writing `stdin` (then closing it) if given.
    async fn adb(
        &self,
        serial: Option<&str>,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> io::Result<Ran> {
        let Some(program) = self.0.adb.get() else {
            return Err(io::Error::other("the device link is off"));
        };
        let mut cmd = tokio::process::Command::new(program);
        if let Some(serial) = serial {
            cmd.args(["-s", serial]);
        }
        cmd.args(args)
            .stdin(match stdin {
                Some(_) => Stdio::piped(),
                None => Stdio::null(),
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let run = async {
            let mut child = cmd.spawn()?;
            if let Some(bytes) = stdin {
                let mut pipe = child.stdin.take().expect("piped");
                pipe.write_all(bytes).await?;
                // End of input: the headset's `cat` finishes.
                drop(pipe);
            }
            child.wait_with_output().await
        };
        let out = tokio::time::timeout(TIMEOUT, run)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "adb did not answer"))??;
        Ok(Ran {
            ok: out.status.success(),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }

    /// Looks at the cable until the runtime ends, telling the core when what
    /// it sees changes.
    pub(super) async fn watch_cable(self) {
        let client = self.client_version().await;
        let mut seen: Option<Vec<Entry>> = None;
        let mut attached = Vec::new();
        let mut told = None;
        let mut probe = true;
        let mut looks = 0u32;
        loop {
            // A probe that overlaps a write may have read the file before it.
            let generation = self.0.cable_written.load(Ordering::Acquire);
            let (found, entries) = self.list(client).await;
            if probe || seen.as_ref() != Some(&entries) {
                attached = Vec::new();
                for entry in &entries {
                    attached.push(self.probe(entry).await);
                }
                seen = Some(entries);
            }
            if self.0.cable_written.load(Ordering::Acquire) != generation {
                probe = true;
                continue;
            }
            let now = (found, attached.clone());
            if told.as_ref() != Some(&now) {
                self.0.state.lock().unwrap().attached = attached.clone();
                self.send(LinkEvent::Cable {
                    adb: found,
                    attached: attached.clone(),
                });
                told = Some(now);
            }
            looks = looks.wrapping_add(1);
            probe = tokio::select! {
                _ = tokio::time::sleep(EVERY) => looks.is_multiple_of(PROBE_EVERY),
                _ = self.0.cable_poke.notified() => true,
            };
        }
    }

    /// Whether adb runs here, and the headsets on a USB cable (adb over the
    /// network is not a cable). An adb server of another version (one that
    /// another tool started) is left alone: asking it would make this adb
    /// kill it and start its own, dropping that tool's port forwards.
    async fn list(&self, client: Option<u32>) -> (bool, Vec<Entry>) {
        if let (Some(ours), Some(running)) = (client, server_version().await) {
            if ours != running {
                return (true, Vec::new());
            }
        }
        match self.adb(None, &["devices", "-l"], None).await {
            Err(e) if e.kind() == io::ErrorKind::NotFound => (false, Vec::new()),
            Err(_) => (true, Vec::new()),
            Ok(ran) => {
                let all = adb::devices(&String::from_utf8_lossy(&ran.stdout));
                (true, all.into_iter().filter(|e| e.usb).collect())
            }
        }
    }

    /// adb's own version (`adb version`: `… version 1.0.41` is 41), which
    /// starts no server.
    async fn client_version(&self) -> Option<u32> {
        let ran = self.adb(None, &["version"], None).await.ok()?;
        let out = String::from_utf8_lossy(&ran.stdout);
        let line = out.lines().next()?;
        line.rsplit('.').next()?.trim().parse().ok()
    }

    async fn probe(&self, entry: &Entry) -> Attached {
        let state = match entry.state.as_str() {
            "device" => self.probe_app(&entry.serial).await,
            "unauthorized" | "authorizing" => CableState::Unauthorized,
            _ => CableState::Offline,
        };
        Attached {
            serial: entry.serial.clone(),
            name: entry.model.clone().unwrap_or_else(|| "Headset".into()),
            state,
            pairing: false,
            pairable: false,
        }
    }

    async fn probe_app(&self, serial: &str) -> CableState {
        let pm = ["shell", "pm", "list", "packages", adb::PACKAGE];
        match self.adb(Some(serial), &pm, None).await {
            Ok(ran) if ran.ok => {
                if !adb::installed(&String::from_utf8_lossy(&ran.stdout)) {
                    return CableState::NoApp;
                }
            }
            _ => return CableState::Offline,
        }
        match self.read_pairing(serial).await {
            Ok(Some(raw)) => {
                let stored = Stored::parse(&raw);
                CableState::Ready {
                    device: stored.device().map(|_| stored.device.clone()),
                    paired: self.whose(&stored),
                }
            }
            Ok(None) => CableState::NotDebuggable,
            Err(_) => CableState::Offline,
        }
    }

    /// The app's pairing file as it is (empty when there is none);
    /// `Ok(None)` when `run-as` says the app is not debuggable (a release
    /// build); `Err` for any other failure.
    async fn read_pairing(&self, serial: &str) -> Result<Option<Vec<u8>>, String> {
        let ran = self
            .adb(Some(serial), &["shell", READ_PAIRING], None)
            .await
            .map_err(|e| e.to_string())?;
        match (ran.ok, ran.stderr.contains("not debuggable")) {
            (true, _) => Ok(Some(ran.stdout)),
            (false, true) => Ok(None),
            (false, false) => Err(or(ran.stderr, "adb could not read it")),
        }
    }

    /// The pairing file, or why it cannot be read.
    async fn pairing_file(&self, serial: &str) -> Result<Vec<u8>, String> {
        self.read_pairing(serial)
            .await?
            .ok_or_else(|| "its Functor app is not a debuggable build".to_string())
    }

    /// Replaces the app's pairing file with `file`, then reads it back.
    async fn write_pairing(&self, serial: &str, file: &[u8]) -> Result<(), String> {
        let ran = self
            .adb(Some(serial), &["shell", WRITE_PAIRING], Some(file))
            .await
            .map_err(|e| e.to_string())?;
        if !ran.ok {
            return Err(or(ran.stderr, "the headset refused the write"));
        }
        match self.pairing_file(serial).await? {
            back if back == file => Ok(()),
            _ => Err("it read back differently".to_string()),
        }
    }

    fn whose(&self, stored: &Stored) -> HeadsetPairing {
        match stored.pairing() {
            None => HeadsetPairing::Nobody,
            Some((desktop, _)) if desktop == self.0.id => HeadsetPairing::This,
            Some(_) => HeadsetPairing::Other {
                name: stored
                    .desktop_name
                    .clone()
                    .unwrap_or_else(|| "another computer".into()),
            },
        }
    }

    /// Pairs the headset on `serial`'s cable, then tells the core.
    pub(super) async fn pair_by_cable(self, serial: String, replace: bool) {
        let result = {
            let _one = self.0.cable_writes.lock().await;
            self.cable_pair(&serial, replace).await
        };
        let device = result.as_ref().ok().map(|p| p.id.clone());
        self.wrote();
        self.send(LinkEvent::CablePaired { serial, result });
        // A link made during the read-back came before its card: it links
        // again, now that the core has one.
        if let Some(device) = device {
            for c in self.0.state.lock().unwrap().conns.values() {
                if c.linked.as_deref() == Some(device.as_str()) {
                    let _ = c.cmds.send(ConnCmd::Close);
                }
            }
        }
    }

    /// Stores a new pairing here first, then writes it to the headset and
    /// reads it back. On a failure after the store, nothing is left: the
    /// pairing goes from here, and the headset gets back what it held.
    async fn cable_pair(&self, serial: &str, replace: bool) -> Result<Paired, String> {
        let name = {
            let s = self.0.state.lock().unwrap();
            let a = s.attached.iter().find(|a| a.serial == serial);
            a.map_or_else(|| "Headset".to_string(), |a| a.name.clone())
        };
        let cannot = |e: String| format!("cannot pair {name} over the cable: {e}");
        let raw = self.pairing_file(serial).await.map_err(cannot)?;
        let mut stored = Stored::parse(&raw);
        // Asked about before; checked again in case it paired meanwhile.
        if let HeadsetPairing::Other { name: other } = self.whose(&stored) {
            if !replace {
                return Err(cannot(format!("it is paired with {other}")));
            }
        }
        // Its own id, which the desktop knows it by; one made here if its
        // app has not run yet.
        if stored.device().is_none() {
            stored.device = hex(&os_random::<16>());
        }
        let device = stored.device.clone();
        let secret = Secret(os_random::<32>());
        let paired = self
            .store_pairing(device.clone(), &name, &secret)
            .map_err(|e| cannot(format!("cannot store it here: {e}")))?;
        let file = Stored {
            device: device.clone(),
            desktop: Some(hex(&self.0.id)),
            desktop_name: Some(self.0.name.clone()),
            secret: Some(secret.to_hex()),
        }
        .to_json();
        let Err(e) = self.write_pairing(serial, file.as_bytes()).await else {
            return Ok(paired);
        };
        if let Err(e) = self.drop_pairing(&device) {
            eprintln!("functor-app: cannot undo a failed cable pairing: {e}");
        }
        // What it held before, unless something else has replaced ours.
        let before = match raw.is_empty() {
            true => Stored {
                device,
                ..Stored::default()
            }
            .to_json()
            .into_bytes(),
            false => raw,
        };
        let undone = match self.pairing_file(serial).await {
            Ok(now) if Stored::parse(&now).secret == Some(secret.to_hex()) => {
                self.write_pairing(serial, &before).await
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        };
        Err(cannot(match undone {
            Ok(()) => format!("writing it to the headset failed: {e}"),
            Err(again) => format!(
                "writing it to the headset failed ({e}), and so did putting its old \
                 pairing back ({again}); forget it here and pair again"
            ),
        }))
    }

    /// A cable write finished: probes from before it are stale.
    fn wrote(&self) {
        self.0.cable_written.fetch_add(1, Ordering::AcqRel);
        self.0.cable_poke.notify_one();
    }

    /// After Forget: removes the forgotten pairing (its `secret`) from
    /// `device`'s app if it is on a cable, keeping its id. Best effort; the
    /// user is told when it could not.
    pub(super) fn forget_on_cable(&self, device: &str, name: &str, secret: String) {
        let serial = {
            let s = self.0.state.lock().unwrap();
            s.attached
                .iter()
                .find(|a| matches!(&a.state, CableState::Ready { device: Some(d), .. } if d == device))
                .map(|a| a.serial.clone())
        };
        let Some(serial) = serial else {
            return;
        };
        let hub = self.clone();
        let name = name.to_string();
        tokio::spawn(async move {
            let cleared = {
                let _one = hub.0.cable_writes.lock().await;
                hub.clear_pairing(&serial, &secret).await
            };
            if let Err(e) = cleared {
                let reason = format!(
                    "forgot {name} here, but could not remove its pairing from the headset: {e}"
                );
                hub.send(LinkEvent::Failed { reason });
            }
            hub.wrote();
        });
    }

    /// Removes the pairing with `secret` from the headset's file; any other
    /// (a newer one, another desktop's) is not this Forget's to remove.
    async fn clear_pairing(&self, serial: &str, secret: &str) -> Result<(), String> {
        let stored = Stored::parse(&self.pairing_file(serial).await?);
        if stored.secret.as_deref() != Some(secret) {
            return Ok(());
        }
        let cleared = Stored {
            device: stored.device,
            ..Stored::default()
        };
        self.write_pairing(serial, cleared.to_json().as_bytes())
            .await
    }
}

/// The running adb server's version, asked over its socket (`host:version`)
/// so none is started; `None` when none answers.
async fn server_version() -> Option<u32> {
    let port = std::env::var("ANDROID_ADB_SERVER_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5037u16);
    let ask = async {
        let mut s = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await?;
        s.write_all(b"000chost:version").await?;
        let mut reply = [0u8; 12];
        s.read_exact(&mut reply).await?;
        Ok::<_, io::Error>(reply)
    };
    let reply = tokio::time::timeout(Duration::from_secs(1), ask)
        .await
        .ok()?
        .ok()?;
    // `OKAY`, the length (`0004`), the version in hex.
    if &reply[..4] != b"OKAY" {
        return None;
    }
    u32::from_str_radix(std::str::from_utf8(&reply[8..]).ok()?, 16).ok()
}

/// `text`, or `fallback` when it is empty.
fn or(text: String, fallback: &str) -> String {
    match text.is_empty() {
        true => fallback.to_string(),
        false => text,
    }
}
