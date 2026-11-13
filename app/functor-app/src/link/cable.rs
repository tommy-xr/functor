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
use std::time::Duration;

use functor_app_core::devices::{Attached, CableState, HeadsetPairing, LinkEvent, Paired};
use functor_device_link::adb::{self, Entry, READ_PAIRING, WRITE_PAIRING};
use functor_device_link::file::Stored;
use functor_device_link::{hex, Entropy, OsEntropy, Secret};
use tokio::io::AsyncWriteExt;

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
        let mut seen: Option<Vec<Entry>> = None;
        let mut attached = Vec::new();
        let mut told = None;
        let mut probe = true;
        let mut looks = 0u32;
        loop {
            let (found, entries) = match self.adb(None, &["devices", "-l"], None).await {
                Err(e) if e.kind() == io::ErrorKind::NotFound => (false, Vec::new()),
                Err(_) => (true, Vec::new()),
                Ok(ran) => {
                    let all = adb::devices(&String::from_utf8_lossy(&ran.stdout));
                    // adb over the network is not a cable.
                    (true, all.into_iter().filter(|e| e.usb).collect())
                }
            };
            if probe || seen.as_ref() != Some(&entries) {
                attached = Vec::new();
                for entry in &entries {
                    attached.push(self.probe(entry).await);
                }
                seen = Some(entries);
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
                let stored = parse(&raw);
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
    /// `Ok(None)` when `run-as` refuses (a release build); `Err` when adb
    /// fails.
    async fn read_pairing(&self, serial: &str) -> Result<Option<Vec<u8>>, String> {
        let ran = self
            .adb(Some(serial), &["shell", READ_PAIRING], None)
            .await
            .map_err(|e| e.to_string())?;
        Ok(ran.ok.then_some(ran.stdout))
    }

    /// Replaces the app's pairing file with `file`, then reads it back.
    async fn write_pairing(&self, serial: &str, file: &str) -> Result<(), String> {
        let ran = self
            .adb(
                Some(serial),
                &["shell", WRITE_PAIRING],
                Some(file.as_bytes()),
            )
            .await
            .map_err(|e| e.to_string())?;
        if !ran.ok {
            return Err(match ran.stderr.is_empty() {
                true => "the headset refused the write".to_string(),
                false => ran.stderr,
            });
        }
        match self.read_pairing(serial).await? {
            Some(back) if back == file.as_bytes() => Ok(()),
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
        let result = self.cable_pair(&serial, replace).await;
        self.send(LinkEvent::CablePaired { serial, result });
        self.0.cable_poke.notify_one();
    }

    /// Stores a new pairing here first, then writes it to the headset and
    /// reads it back; on any failure the desktop's is undone.
    async fn cable_pair(&self, serial: &str, replace: bool) -> Result<Paired, String> {
        let name = {
            let s = self.0.state.lock().unwrap();
            let a = s.attached.iter().find(|a| a.serial == serial);
            a.map_or_else(|| "Headset".to_string(), |a| a.name.clone())
        };
        let cannot = |e: String| format!("cannot pair {name} over the cable: {e}");
        let raw = self.read_pairing(serial).await.map_err(cannot)?;
        let raw = raw.ok_or_else(|| cannot("its Functor app is not a debuggable build".into()))?;
        let mut stored = parse(&raw);
        // Asked about before; checked again in case it paired meanwhile.
        if let HeadsetPairing::Other { name: other } = self.whose(&stored) {
            if !replace {
                return Err(cannot(format!("it is paired with {other}")));
            }
        }
        // Its own id, which the desktop knows it by; one made here if its
        // app has not run yet.
        if stored.device().is_none() {
            stored.device = hex(&random::<16>());
        }
        let device = stored.device.clone();
        let secret = Secret(random::<32>());
        let (paired, previous) = self
            .store_pairing(device.clone(), &name, &secret)
            .map_err(|e| cannot(format!("cannot store it here: {e}")))?;
        let file = Stored {
            device: device.clone(),
            desktop: Some(hex(&self.0.id)),
            desktop_name: Some(self.0.name.clone()),
            secret: Some(secret.to_hex()),
        }
        .to_json();
        if let Err(e) = self.write_pairing(serial, &file).await {
            self.unstore(&device, previous);
            return Err(cannot(format!("writing it to the headset failed: {e}")));
        }
        Ok(paired)
    }

    /// Puts back `device`'s pairing as it was before a failed cable pairing
    /// and closes any link made with the new one.
    fn unstore(&self, device: &str, previous: Option<crate::store::StoredPairing>) {
        let mut s = self.0.state.lock().unwrap();
        let undone = self.update_pairings(&mut s, |all| {
            all.retain(|p| p.id != device);
            all.extend(previous);
        });
        if let Err(e) = undone {
            eprintln!("functor-app: cannot undo a failed cable pairing: {e}");
        }
        for conn in s.conns.values() {
            let _ = conn.cmds.send(ConnCmd::Forgotten(device.to_string()));
        }
    }

    /// After Forget: removes this desktop's pairing from `device`'s app if
    /// it is on a cable, keeping its id. Best effort; the user is told when
    /// it could not.
    pub(super) fn forget_on_cable(&self, device: &str, name: &str) {
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
            if let Err(e) = hub.clear_pairing(&serial).await {
                let reason = format!(
                    "forgot {name} here, but could not remove its pairing from the headset: {e}"
                );
                hub.send(LinkEvent::Failed { reason });
            }
            hub.0.cable_poke.notify_one();
        });
    }

    async fn clear_pairing(&self, serial: &str) -> Result<(), String> {
        let raw = self.read_pairing(serial).await?;
        let raw = raw.ok_or("its Functor app is not a debuggable build")?;
        let stored = parse(&raw);
        // Another desktop's pairing is not ours to remove.
        if self.whose(&stored) != HeadsetPairing::This {
            return Ok(());
        }
        let cleared = Stored {
            device: stored.device,
            ..Stored::default()
        };
        self.write_pairing(serial, &cleared.to_json()).await
    }
}

/// A pairing file's contents; one that does not parse counts as empty, as
/// the headset app treats it (it moves it aside and starts afresh).
fn parse(raw: &[u8]) -> Stored {
    serde_json::from_slice(raw).unwrap_or_default()
}

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0; N];
    OsEntropy.fill(&mut out);
    out
}
