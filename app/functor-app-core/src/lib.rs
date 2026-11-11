//! The Functor desktop app's core: state, the pure `update`, the wire
//! protocol, project discovery, the game session's state machine and agent
//! tasks. No UI, window or server; the shell in `functor-app` runs IO and
//! feeds results in as events.

pub mod print;
pub mod projects;
pub mod protocol;
pub mod session;
pub mod state;
pub mod tasks;

pub use print::{print, print_item};
pub use projects::discover;
pub use session::{
    listening_url, Display, Polled, Session, SessionEvent, SessionMode, SessionStatus,
};
pub use state::{update, AppContext, AppState, Command, Event, GitInfo, RunSpec};
pub use tasks::{AgentEvent, Harness, Item, RunEvent, Task, TaskStatus};

#[cfg(all(test, feature = "ts"))]
mod ts_bindings;
