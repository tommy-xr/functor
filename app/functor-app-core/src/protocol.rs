//! Messages between the UI and the core.

use serde::{Deserialize, Serialize};

use crate::AppState;

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
}

/// Core → UI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// The whole state; the UI replaces what it holds.
    State { state: AppState },
}
