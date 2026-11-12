//! The Functor desktop app's core: state, the pure `update`, the wire
//! protocol, project discovery and the game session's state machine. No UI,
//! window or server; the shell in `functor-app` runs IO and feeds results in
//! as events.

pub mod print;
pub mod projects;
pub mod protocol;
pub mod session;
pub mod state;

pub use print::print;
pub use projects::discover;
pub use session::{
    listening_url, Display, Polled, Session, SessionEvent, SessionMode, SessionStatus,
};
pub use state::{update, AppState, Command, Event, GitInfo};

#[cfg(all(test, feature = "ts"))]
mod ts_bindings;
