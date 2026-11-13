//! The headset's pairing file. The client reads and writes it; the desktop
//! writes it over adb to pair by cable ([`crate::adb`]).

use std::io::{self, Write};
use std::path::Path;

use crate::{hex, unhex, DesktopId, DeviceId, OsEntropy, Secret};

/// The pairing file, JSON:
/// `{"device": "<32 hex>", "desktop": "<32 hex>", "desktop_name": "…", "secret": "<64 hex>"}`.
/// `device` is made once and kept; the other three are written together by
/// a pairing (here, or from outside before the client reads it again).
/// `Debug` hides the secret.
#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stored {
    #[serde(default)]
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

impl std::fmt::Debug for Stored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stored")
            .field("device", &self.device)
            .field("desktop", &self.desktop)
            .field("desktop_name", &self.desktop_name)
            .field("secret", &self.secret.as_ref().map(|_| ".."))
            .finish()
    }
}

impl Stored {
    /// The file at `path`; a missing one is created with a new device id.
    /// One that does not parse is moved aside (`.bad`) and started afresh,
    /// so the headset can still pair.
    pub fn load(path: &Path) -> io::Result<Stored> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut stored: Stored = match serde_json::from_str(&text) {
            Ok(stored) => stored,
            Err(_) if text.is_empty() => Stored::default(),
            Err(e) => {
                log::error!("device link: {path:?} does not parse ({e}); moved aside");
                std::fs::rename(path, path.with_extension("bad"))?;
                Stored::default()
            }
        };
        if unhex::<16>(&stored.device).is_none() {
            let mut id = [0u8; 16];
            crate::Entropy::fill(&mut OsEntropy, &mut id);
            stored.device = hex(&id);
            stored.save(path)?;
        }
        Ok(stored)
    }

    /// Whole or not at all, readable by this user only.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        let mut file = std::fs::File::create(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(self.to_json().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(tmp, path)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("plain strings")
    }

    pub fn device(&self) -> Option<DeviceId> {
        unhex(&self.device)
    }

    /// The desktop and secret, when all of the pairing is present.
    pub fn pairing(&self) -> Option<(DesktopId, Secret)> {
        let desktop = unhex(self.desktop.as_deref()?)?;
        let secret = Secret::from_hex(self.secret.as_deref()?)?;
        Some((desktop, secret))
    }
}
