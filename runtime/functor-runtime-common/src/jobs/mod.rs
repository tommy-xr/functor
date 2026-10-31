//! Jobs: heavy work off the frame thread. [`Shareable`] is the form a value
//! takes to cross to a worker and back.

mod shareable;

pub use shareable::{HostValue, Key, Shareable, NODE_CAP};
