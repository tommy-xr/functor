//! The active project's game session: a `functor run native` child the shell
//! spawns and polls over its debug server.

use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::transport::{LogPage, Timeline};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Session {
    /// Unique per start; events from an older session's child are dropped.
    pub id: u64,
    /// The child's process id once spawned.
    pub pid: Option<u32>,
    pub status: SessionStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionStatus {
    Stopped,
    /// Spawning, or waiting for the debug server's listening line.
    Starting,
    /// `surface` is from the last successful poll.
    Running {
        debug_url: String,
        /// The game's pointer surface (`/state.input.mouse.surface_width`,
        /// `surface_height`): the space `/input` mouse coordinates are in.
        surface: Option<[u32; 2]>,
        /// Its clock and marks: sent as `ServerMsg::Transport` and
        /// `ServerMsg::Marks`, never with the state, as they change every poll.
        #[serde(skip)]
        #[cfg_attr(feature = "ts", ts(skip))]
        timeline: Box<Timeline>,
    },
    /// The child exited on its own after it was running.
    Exited {
        reason: String,
    },
    /// It could not be spawned, or exited before its debug server listened.
    Failed {
        reason: String,
    },
}

/// How the session runs: its project's game placement decides (`Placement::session_mode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// A hidden window whose frames the page streams.
    #[default]
    Hidden,
    /// A normal, visible game window.
    Window,
}

/// How the shell runs the child, from the mode and whether sessions get a GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    /// `--headless`: no window or GL, so no frames to show.
    Headless,
    /// `--hidden`.
    Hidden,
    /// A visible window.
    Window,
}

impl Display {
    pub fn new(mode: SessionMode, no_gpu: bool) -> Self {
        match (mode, no_gpu) {
            (_, true) => Self::Headless,
            (SessionMode::Hidden, false) => Self::Hidden,
            (SessionMode::Window, false) => Self::Window,
        }
    }

    /// The `functor run native` flags. A session the app shows opens paused
    /// on frame 0 (D37); its own window runs at once, as it has no transport bar.
    pub fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Headless => &["--headless", "--paused"],
            Self::Hidden => &["--hidden", "--paused"],
            Self::Window => &[],
        }
    }
}

impl SessionStatus {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Starting | Self::Running { .. })
    }
}

/// What the shell observed about session `id`'s child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Spawned {
        pid: u32,
    },
    /// The `[debug-server] listening on …` line, as [`listening_url`] reads it.
    Listening {
        debug_url: String,
    },
    /// A `GET /state` sent after `after` transport calls (`Command::PollState`),
    /// or why it failed.
    Polled {
        after: u64,
        result: Result<Polled, String>,
    },
    /// A `GET /input-log` read (`Command::ReadInputLog`), or why it failed.
    InputLog(Result<LogPage, String>),
    /// The child exited; `reason` is its status and the error it printed.
    Exited {
        reason: String,
    },
    /// The child could not be spawned.
    StartFailed {
        reason: String,
    },
    /// A project file changed after the child ended.
    SourceChanged,
}

/// What the core keeps from a `GET /state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Polled {
    pub frame: u64,
    /// `None` while the game reports no surface (0×0).
    pub surface: Option<[u32; 2]>,
    /// `/state.clock`; `None` from a runtime too old to report it.
    pub clock: Option<PolledClock>,
}

/// `GET /state`'s `clock` (debug protocol v37).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolledClock {
    pub paused: bool,
    /// `[oldest, newest]`, numbered as `frame`.
    pub recorded: Option<[u64; 2]>,
    pub generation: u64,
    /// Successful hot reloads since the program loaded.
    pub program_revision: u64,
}

/// The debug server's URL from its stderr line, with the port it really
/// bound: `[debug-server] listening on http://127.0.0.1:52011`.
pub fn listening_url(line: &str) -> Option<String> {
    loopback_url(line, "[debug-server]")
}

/// The `http://<ip>:<port>` a `<tag> listening on http://<ip>:<port>` line names.
pub(crate) fn loopback_url(line: &str, tag: &str) -> Option<String> {
    let addr = line
        .strip_prefix(tag)?
        .strip_prefix(" listening on http://")?
        .trim();
    addr.parse::<SocketAddr>().ok()?;
    Some(format!("http://{addr}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_the_app_shows_open_paused() {
        assert_eq!(Display::Hidden.flags(), ["--hidden", "--paused"]);
        assert_eq!(Display::Headless.flags(), ["--headless", "--paused"]);
        assert!(Display::Window.flags().is_empty());
    }

    #[test]
    fn reads_the_bound_address_from_the_listening_line() {
        assert_eq!(
            listening_url("[debug-server] listening on http://127.0.0.1:52011\n").as_deref(),
            Some("http://127.0.0.1:52011")
        );
        for other in [
            "[debug-server] listening on http://127.0.0.1:",
            "[debug-server] listening on http://localhost:8077",
            "[functor] listening on http://127.0.0.1:8077",
            "",
        ] {
            assert_eq!(listening_url(other), None, "{other}");
        }
    }
}
