//! The device link on the headset: the shared client
//! ([`functor_device_link::client`]) given this device's name, status and a
//! Wi-Fi multicast lock while it browses (Android drops incoming multicast,
//! so mDNS answers, without one).
//!
//! Its pairing file is `<internal data>/device-link.json`, the app's
//! private storage: see [`functor_device_link::client::Stored`] for the
//! format.

use std::net::SocketAddr;
use std::sync::Mutex;

use functor_device_link::client::{Client, Command, Config, Host, Pairing};
use functor_device_link::wire::Status;
use functor_runtime_common::debug_protocol::{
    DeviceLinkState, PairingState, PanelState, VoiceToast,
};
use functor_runtime_common::link_overlay::Gesture;
use jni::objects::JObject;
use jni::refs::Global;
use jni::{jni_sig, jni_str, JValue};

/// The pairing file's name in the app's internal storage.
pub const STORE: &str = "device-link.json";

/// The running project's name, as its `/load-project` declared it; the
/// heartbeat reports it.
static PROJECT: Mutex<Option<String>> = Mutex::new(None);

/// A load landed: `name` (`None` for none declared, or the boot scene).
pub fn set_project(name: Option<String>) {
    *PROJECT.lock().unwrap() = name;
}

struct QuestHost {
    build: String,
    multicast: Mutex<Option<Global<JObject<'static>>>>,
}

impl Host for QuestHost {
    fn status(&self) -> Status {
        let battery = crate::battery::latest();
        Status {
            battery: battery
                .as_ref()
                .and_then(|b| b.level_pct)
                .and_then(|pct| u8::try_from(pct.clamp(0, 100)).ok()),
            charging: battery.is_some_and(|b| b.status == "charging"),
            build: self.build.clone(),
            project: PROJECT.lock().unwrap().clone(),
        }
    }

    fn browsing(&self, on: bool) {
        if let Err(e) = self.multicast_lock(on) {
            log::warn!("device link: multicast lock: {e}");
        }
    }
}

impl QuestHost {
    /// `WifiManager.MulticastLock`, made once, held while browsing.
    fn multicast_lock(&self, on: bool) -> jni::errors::Result<()> {
        let Some((vm, context)) = crate::battery::app_context() else {
            return Ok(());
        };
        let mut lock = self.multicast.lock().unwrap();
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            if lock.is_none() {
                let wifi = env.new_string("wifi")?;
                let manager = env
                    .call_method(
                        context,
                        jni_str!("getSystemService"),
                        jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                        &[JValue::Object(&wifi)],
                    )?
                    .l()?;
                let tag = env.new_string("functor-link")?;
                let made = env
                    .call_method(
                        &manager,
                        jni_str!("createMulticastLock"),
                        jni_sig!(
                            "(Ljava/lang/String;)Landroid/net/wifi/WifiManager$MulticastLock;"
                        ),
                        &[JValue::Object(&tag)],
                    )?
                    .l()?;
                env.call_method(
                    &made,
                    jni_str!("setReferenceCounted"),
                    jni_sig!("(Z)V"),
                    &[JValue::Bool(false)],
                )?;
                *lock = Some(env.new_global_ref(made)?);
            }
            let made = lock.as_ref().expect("made above");
            let method = if on {
                jni_str!("acquire")
            } else {
                jni_str!("release")
            };
            env.call_method(made, method, jni_sig!("()V"), &[])?;
            Ok(())
        })
    }
}

/// Starts the client; `debug` is the debug runtime's listener for tunnelled
/// requests.
pub fn start(
    store_dir: Option<std::path::PathBuf>,
    debug: SocketAddr,
    build: &str,
) -> Option<Client> {
    let Some(dir) = store_dir else {
        log::warn!("device link: no app storage, so no pairing");
        return None;
    };
    let name = crate::system_property("ro.product.model").unwrap_or_else(|| "Quest".into());
    let config = Config {
        store: dir.join(STORE),
        name,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        debug,
        desktop: None,
    };
    let host = QuestHost {
        build: build.to_string(),
        multicast: Mutex::new(None),
    };
    match Client::spawn(config, host) {
        Ok(client) => Some(client),
        Err(e) => {
            log::error!("device link: {e}");
            None
        }
    }
}

/// The client's command for `gesture`; `None` for the panel's, which is
/// the shell's own.
pub fn command(gesture: Gesture) -> Option<Command> {
    match gesture {
        Gesture::Pair => Some(Command::Pair),
        Gesture::Matches => Some(Command::Confirm),
        Gesture::Cancel => Some(Command::Cancel),
        Gesture::Panel => None,
    }
}

pub fn pairing_state(pairing: Pairing) -> PairingState {
    match pairing {
        Pairing::Searching => PairingState::Searching,
        Pairing::Code {
            desktop,
            code,
            confirmed,
        } => PairingState::Code {
            desktop,
            code,
            confirmed,
        },
        Pairing::Paired { desktop } => PairingState::Paired { desktop },
        Pairing::Failed { reason } => PairingState::Failed { reason },
    }
}

/// `/state.link`, with the voice toast showing and the panel.
pub fn state(client: &Client, voice: Option<VoiceToast>, panel: PanelState) -> DeviceLinkState {
    let view = client.view();
    DeviceLinkState {
        paired: view.paired,
        linked: view.linked,
        pairing: view.pairing.map(pairing_state),
        voice,
        panel: Some(panel),
    }
}
