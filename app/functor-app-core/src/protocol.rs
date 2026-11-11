//! Messages between the UI and the core.

use serde::{Deserialize, Serialize};

use crate::{AppState, SessionMode};

/// UI → core.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    GetState,
    /// `path` is a project's path relative to the repo root.
    SetActiveProject {
        path: String,
    },
    /// Start a game session for the active project, unless one is live.
    StartSession,
    StopSession,
    /// Where the game shows; restarts a live session in the new mode.
    SetSessionMode {
        mode: SessionMode,
    },
    /// Forwarded to a running session; dropped otherwise. Not answered.
    Input {
        input: GameInput,
    },
}

/// Keyboard and mouse input in the debug server's `POST /input` shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameInput {
    /// `key` is a wire name: `a`–`z`, `0`–`9`, `up`, `space`, `enter`, ….
    Key {
        key: String,
        down: bool,
    },
    /// In the game's pointer surface ([`crate::SessionStatus::Running`]).
    MouseMove {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: MouseButton,
        down: bool,
    },
    /// Positive scrolls up, as GLFW reports it.
    MouseWheel {
        delta: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Core → UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// The whole state; the UI replaces what it holds.
    State { state: AppState },
}
