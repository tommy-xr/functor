//! The cable side: what `adb` prints, read the same way by the CLI's `run vr`
//! and the desktop app's cable pairing, and the shell commands that reach
//! the headset app's pairing file ([`crate::file::Stored`]).
//!
//! Those commands run through `run-as`, which works only for a debuggable
//! build of the app. The file's contents (its secret) travel on stdin and
//! stdout, never in a command line.

/// The headset app's package.
pub const PACKAGE: &str = "dev.functor.runner";

/// `adb shell` command: prints the pairing file, or nothing when it does
/// not exist. Fails when `run-as` does (not installed, not debuggable).
pub const READ_PAIRING: &str =
    "run-as dev.functor.runner sh -c 'cat files/device-link.json 2>/dev/null; true'";

/// `adb shell` command: replaces the pairing file with stdin, whole (a new
/// file renamed over it), readable by the app only.
pub const WRITE_PAIRING: &str = "run-as dev.functor.runner sh -c 'umask 077 && mkdir -p files \
&& cat > files/device-link.cable && mv files/device-link.cable files/device-link.json'";

/// One `adb devices [-l]` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub serial: String,
    /// `device` when ready; `unauthorized` until the headset allows this
    /// computer; `offline` and others otherwise.
    pub state: String,
    /// On a USB cable (`-l` only; adb over the network has no `usb:`).
    pub usb: bool,
    /// `model:Quest_3` as `Quest 3` (`-l` only).
    pub model: Option<String>,
    /// New on every reconnect (`-l` only).
    pub transport: Option<String>,
}

/// The entries `adb devices [-l]` lists.
pub fn devices(out: &str) -> Vec<Entry> {
    out.lines()
        .filter(|line| !line.starts_with("List of devices") && !line.starts_with('*'))
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?.to_string();
            let state = fields.next()?.to_string();
            let mut entry = Entry {
                serial,
                state,
                usb: false,
                model: None,
                transport: None,
            };
            for field in fields {
                match field.split_once(':') {
                    Some(("usb", _)) => entry.usb = true,
                    Some(("model", m)) => entry.model = Some(m.replace('_', " ")),
                    Some(("transport_id", t)) => entry.transport = Some(t.to_string()),
                    _ => {}
                }
            }
            Some(entry)
        })
        .collect()
}

/// Whether `pm list packages <PACKAGE>` lists the app (it also lists
/// packages whose names merely contain it).
pub fn installed(pm: &str) -> bool {
    let want = format!("package:{PACKAGE}");
    pm.lines().any(|line| line.trim() == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_with_and_without_details() {
        let out = "* daemon started successfully\nList of devices attached\n\
2G0YC5ZF87037Q         device usb:0-1 product:eureka model:Quest_3 device:eureka transport_id:1\n\
1WMHH000000000\tunauthorized usb:0-2 transport_id:2\n\
192.168.1.9:5555\tdevice product:eureka model:Quest_3 transport_id:3\n\nX\toffline\n";
        let list = devices(out);
        assert_eq!(list.len(), 4);
        assert_eq!(
            list[0],
            Entry {
                serial: "2G0YC5ZF87037Q".into(),
                state: "device".into(),
                usb: true,
                model: Some("Quest 3".into()),
                transport: Some("1".into()),
            }
        );
        assert_eq!(
            (list[1].state.as_str(), list[1].usb),
            ("unauthorized", true)
        );
        assert!(!list[2].usb, "over the network");
        assert_eq!(
            (list[3].serial.as_str(), list[3].state.as_str()),
            ("X", "offline")
        );
    }

    #[test]
    fn the_file_commands_name_the_app() {
        for command in [READ_PAIRING, WRITE_PAIRING] {
            assert!(command.starts_with(&format!("run-as {PACKAGE} ")));
            assert!(command.contains("files/device-link.json"));
        }
    }

    #[test]
    fn installed_only_for_the_exact_package() {
        assert!(installed("package:dev.functor.runner\n"));
        assert!(!installed("package:dev.functor.runner.other\n"));
        assert!(!installed(""));
    }
}
