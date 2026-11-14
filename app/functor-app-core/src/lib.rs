//! The Functor desktop app's core: state, the pure `update`, the wire
//! protocol, project discovery, the game session's state machine and agent
//! tasks. No UI, window or server; the shell in `functor-app` runs IO and
//! feeds results in as events.

pub mod activity;
pub mod agent_view;
pub mod changes;
pub mod cli;
pub mod creation;
pub mod decision;
pub mod devices;
pub mod evidence;
pub mod folders;
pub mod layout;
pub mod metrics;
pub mod panel;
pub mod print;
pub mod projects;
pub mod protocol;
pub mod queue;
pub mod route;
pub mod review;
pub mod session;
pub mod state;
pub mod tasks;
pub mod transport;
pub mod voice;

pub use agent_view::AppContext;
pub use changes::Change;
pub use cli::{CliCause, CliProblem};
pub use devices::{Device, DeviceState, Devices, Headset, LinkCommand, LinkEvent};
pub use folders::Recent;
pub use layout::{Layout, LayoutOp, View};
pub use print::{print, print_change, print_item, print_layout};
pub use projects::discover;
pub use review::{ReviewEvent, ReviewSetting, ReviewSpec};
pub use session::{
    listening_url, Display, Polled, PolledClock, Session, SessionEvent, SessionMode, SessionStatus,
};
pub use state::{update, AppState, Command, Event, GitInfo, RunSpec};
pub use tasks::{AgentEvent, Harness, Item, RunEvent, Scope, Task, TaskStatus};
pub use transport::{LogPage, SessionCall, Transport, TransportCall};

#[cfg(all(test, feature = "ts"))]
mod ts_bindings;
