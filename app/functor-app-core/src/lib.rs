//! The Functor desktop app's core: state, the pure `update`, the wire
//! protocol and project discovery. No UI, window or server; the shell in
//! `functor-app` runs IO and feeds results in as events.

pub mod print;
pub mod projects;
pub mod protocol;
pub mod state;

pub use print::print;
pub use projects::discover;
pub use state::{update, AppState, Command, Event};

#[cfg(all(test, feature = "ts"))]
mod ts_bindings;
